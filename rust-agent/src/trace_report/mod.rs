//! Native metaharness trace narrative pipeline. Trace data and generated prose
//! are untrusted report data; no tools, scripts, plugins or transcript code
//! run.
pub(crate) mod cli;
mod render;
use std::{
	collections::BTreeMap,
	sync::{
		Mutex,
		atomic::{AtomicBool, AtomicUsize, Ordering},
	},
	time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::{BoundedWriter, cancel::CancelToken};
use serde::{Deserialize, Serialize};

use crate::{chat, provider::Model};

pub const VERSION: &str = "rust-trace-report-v1";
const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_TEXT: usize = 1024 * 1024;
const MAX_REPORT: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum Entry {
	#[serde(rename = "assistant")]
	Assistant { model: String, text: String, tools: Vec<String> },
	#[serde(rename = "toolResult")]
	ToolResult {
		tool:     String,
		#[serde(rename = "isError")]
		is_error: bool,
		text:     String,
	},
	#[serde(rename = "notice")]
	Notice { text: String },
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
	pub job_name:     String,
	pub trace:        String,
	pub entries:      Vec<Entry>,
	pub total_events: u64,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
	pub run:    RunInfo,
	pub traces: Vec<TraceRow>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
	pub benchmark: String,
	pub dataset:   String,
	pub models:    String,
	pub job_name:  String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceRow {
	pub name:        String,
	pub task:        String,
	pub status:      String,
	pub reward:      Option<f64>,
	pub cost_usd:    Option<f64>,
	pub duration_ms: Option<f64>,
}
#[derive(Debug)]
pub enum Item {
	Turn { model: String, text: String, tools: Vec<String>, results: Vec<ResultEntry> },
	Notice { text: String },
}
#[derive(Debug)]
pub struct ResultEntry {
	pub tool:     String,
	pub is_error: bool,
	pub text:     String,
}
#[derive(Debug, Clone)]
pub struct Options {
	pub run:         String,
	pub trace:       String,
	pub focus:       Option<String>,
	pub concurrency: usize,
}
#[derive(Debug, Default, Serialize)]
pub struct Accounting {
	pub calls:             u64,
	pub input:             u64,
	pub output:            u64,
	pub cache_read:        u64,
	pub cache_write:       u64,
	pub unavailable_calls: u64,
}
impl Accounting {
	fn add(&mut self, usage: Option<crate::usage::UsageReport>) -> Result<()> {
		self.calls = self.calls.checked_add(1).context("usage overflow")?;
		if let Some(tokens) = usage.and_then(|u| u.tokens) {
			for (sum, n) in [
				(&mut self.input, tokens.input),
				(&mut self.output, tokens.output),
				(&mut self.cache_read, tokens.cache_read),
				(&mut self.cache_write, tokens.cache_write),
			] {
				*sum = sum.checked_add(n).context("usage overflow")?;
			}
		} else {
			self.unavailable_calls += 1;
		}
		Ok(())
	}
}
#[derive(Debug, Serialize)]
pub struct Report {
	pub version:     &'static str,
	pub markdown:    String,
	pub items:       usize,
	pub turns:       usize,
	pub tiny_usage:  Accounting,
	pub synth_usage: Accounting,
	pub diagnostics: Vec<String>,
}

pub fn decode(bytes: &[u8]) -> Result<Trace> {
	ensure!(bytes.len() <= MAX_INPUT, "trace exceeds byte limit");
	let trace: Trace = serde_json::from_slice(bytes).context("invalid normalized trace")?;
	validate(&trace)?;
	Ok(trace)
}
fn label(text: &str) -> Result<()> {
	ensure!(
		!text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control),
		"invalid trace label"
	);
	Ok(())
}
fn identity(value: &str) -> Result<()> {
	label(value)?;
	ensure!(!matches!(value, "." | "..") && !value.contains('/'), "invalid run/trace identity");
	Ok(())
}
fn validate(trace: &Trace) -> Result<()> {
	label(&trace.job_name)?;
	label(&trace.trace)?;
	ensure!(trace.entries.len() <= MAX_ITEMS, "too many trace entries");
	let mut size = 0usize;
	for entry in &trace.entries {
		let text = match entry {
			Entry::Assistant { model, text, tools } => {
				label(model)?;
				ensure!(tools.len() <= 1024, "too many tools in turn");
				for tool in tools {
					label(tool)?;
					size += tool.len();
				}
				size += model.len();
				text
			},
			Entry::ToolResult { tool, text, .. } => {
				label(tool)?;
				size += tool.len();
				text
			},
			Entry::Notice { text } => text,
		};
		ensure!(text.len() <= MAX_TEXT, "trace entry exceeds text limit");
		size += text.len();
		ensure!(size <= MAX_INPUT, "trace exceeds aggregate text limit");
	}
	Ok(())
}
/// Notices retain their position but do not break result attachment, matching
/// normalized legacy traces. A leading/orphan result is rejected.
pub fn group(trace: &Trace) -> Result<Vec<Item>> {
	validate(trace)?;
	let mut items = Vec::new();
	let mut current = None;
	for entry in &trace.entries {
		match entry {
			Entry::Assistant { model, text, tools } => {
				current = Some(items.len());
				items.push(Item::Turn {
					model:   model.clone(),
					text:    text.clone(),
					tools:   tools.clone(),
					results: Vec::new(),
				});
			},
			Entry::Notice { text } => items.push(Item::Notice { text: text.clone() }),
			Entry::ToolResult { tool, is_error, text } => {
				let Some(Item::Turn { results, .. }) = current.and_then(|i| items.get_mut(i)) else {
					bail!("trace starts with a toolResult; cannot attach it to a turn");
				};
				results.push(ResultEntry {
					tool:     tool.clone(),
					is_error: *is_error,
					text:     text.clone(),
				});
			},
		}
	}
	Ok(items)
}
fn template(template: &str, value: &serde_json::Value) -> Result<String> {
	let mut renderer = handlebars::Handlebars::new();
	renderer.register_escape_fn(handlebars::no_escape);
	let mut output = BoundedWriter::new(MAX_REPORT);
	renderer.render_template_to_write(template.trim_end(), value, &mut output)?;
	Ok(String::from_utf8(output.into_inner())?)
}
fn utf16_prefix(text: &str, limit: usize) -> String {
	let mut n = 0;
	text
		.chars()
		.take_while(|c| {
			n += c.len_utf16();
			n <= limit
		})
		.collect()
}
fn turn_prompt(item: &Item) -> Result<String> {
	let Item::Turn { model, text, results, .. } = item else {
		bail!("not a turn");
	};
	let results: Vec<_> = results
		.iter()
		.enumerate()
		.map(|(index, r)| {
			let body = utf16_prefix(&r.text, 1400);
			serde_json::json!({"number":index+1,"tool":r.tool,"status":if r.is_error {"ERROR"} else {"ok"},
            "body":if body.len() < r.text.len() {format!("{body}…")} else {body}})
		})
		.collect();
	template(
		include_str!("../prompts/trace-report/map-user.md"),
		&serde_json::json!({"model":model,"prose":if text.trim().is_empty() {"(none)".into()} else {utf16_prefix(text.trim(),2000)},
        "results":results,"todo_only":!results.is_empty() && results.iter().all(|r| r["tool"]=="todo")}),
	)
}
fn ask(
	model: &(dyn Model + Sync),
	system: &str,
	user: String,
	usage: &Mutex<Accounting>,
	cancellation: &CancelToken,
	max_chars: usize,
) -> Result<String> {
	let messages = [chat::system(system.trim_end()), chat::user(user)];
	for attempt in 0..4 {
		cancellation.heartbeat()?;
		let mut delivered = 0usize;
		let response = model.stream_response(&messages, &[], cancellation, &mut |delta| {
			delivered = delivered
				.checked_add(delta.len())
				.context("text size overflow")?;
			ensure!(delivered <= max_chars, "completion exceeds text limit");
			Ok(())
		});
		match response {
			Ok(response) => {
				usage
					.lock()
					.map_err(|_| anyhow::anyhow!("usage lock poisoned"))?
					.add(response.usage)?;
				ensure!(
					response.message.role == "assistant"
						&& response
							.message
							.tool_calls
							.as_ref()
							.is_none_or(Vec::is_empty),
					"report model attempted tool calls"
				);
				let text = response.message.content.unwrap_or_default();
				ensure!(text.len() <= max_chars, "completion exceeds text limit");
				if !text.trim().is_empty() {
					return Ok(text.trim().into());
				}
			},
			Err(_) => {
				cancellation.heartbeat()?;
				// Do not include remote bodies, credentials, or full prompts in errors.
				usage
					.lock()
					.map_err(|_| anyhow::anyhow!("usage lock poisoned"))?
					.add(None)?;
			},
		}
		if attempt < 3 {
			harvest_runtime::cancel::wait(Duration::from_secs(attempt + 1), cancellation)?;
		}
	}
	bail!("trace report completion failed after four attempts")
}

/// Whole map/reduce pipeline. SDK model implementations must cooperate with the
/// shared cancellation token. Work is bounded and results stay in trace order.
pub fn generate(
	trace: &Trace,
	run: Option<&Run>,
	options: &Options,
	tiny: &(dyn Model + Sync),
	synth: &(dyn Model + Sync),
	cancellation: &CancelToken,
) -> Result<Report> {
	label(&options.run)?;
	label(&options.trace)?;
	ensure!(options.focus.as_ref().is_none_or(|s| s.len() <= 16384), "focus notes too large");
	ensure!((1..=32).contains(&options.concurrency), "invalid trace-report concurrency");
	ensure!(
		trace.trace == options.trace && trace.job_name == options.run,
		"trace identity mismatch"
	);
	let items = group(trace)?;
	let (metadata, failed) = render::metadata(run, &options.run, &options.trace)?;
	let turns = items
		.iter()
		.filter(|i| matches!(i, Item::Turn { .. }))
		.count();
	let results: Vec<Mutex<Option<String>>> = (0..items.len()).map(|_| Mutex::new(None)).collect();
	let next = AtomicUsize::new(0);
	let workers_failed = AtomicBool::new(false);
	let tiny_usage = Mutex::new(Accounting::default());
	std::thread::scope(|scope| -> Result<()> {
		let mut workers = Vec::new();
		for _ in 0..options.concurrency.min(items.len()) {
			workers.push(scope.spawn(|| -> Result<()> {
				let outcome = (|| {
					while !workers_failed.load(Ordering::Acquire) {
						cancellation.heartbeat()?;
						let index = next.fetch_add(1, Ordering::Relaxed);
						let Some(item) = items.get(index) else {
							break;
						};
						if matches!(item, Item::Notice { .. }) {
							continue;
						}
						let sentence = ask(
							tiny,
							include_str!("../prompts/trace-report/map-system.md"),
							turn_prompt(item)?,
							&tiny_usage,
							cancellation,
							4096,
						)?;
						let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
						*results[index]
							.lock()
							.map_err(|_| anyhow::anyhow!("result lock poisoned"))? = Some(sentence);
					}
					Ok(())
				})();
				if outcome.is_err() {
					workers_failed.store(true, Ordering::Release);
				}
				outcome
			}));
		}
		let mut first_error = None;
		for worker in workers {
			let result = worker
				.join()
				.unwrap_or_else(|_| Err(anyhow::anyhow!("trace report worker panicked")));
			if let Err(error) = result {
				if first_error.is_none() {
					first_error = Some(error);
				}
			}
		}
		if let Some(error) = first_error {
			return Err(error);
		}
		Ok(())
	})?;
	cancellation.heartbeat()?;
	let sentences: Vec<_> = results
		.into_iter()
		.map(|r| {
			r.into_inner()
				.map_err(|_| anyhow::anyhow!("result lock poisoned"))
		})
		.collect::<Result<_>>()?;
	let turn_log = render::turn_log(&items, &sentences)?;
	let final_prose = items
		.iter()
		.rev()
		.find_map(|i| match i {
			Item::Turn { text, .. } if !text.trim().is_empty() => Some(text.as_str()),
			_ => None,
		})
		.unwrap_or("(none)");
	let prompt = template(
		include_str!("../prompts/trace-report/synth-user.md"),
		&serde_json::json!({"run":options.run,"trace":options.trace,"metadata":metadata,"focus":options.focus,
        "turn_log":turn_log,"final_prose":utf16_prefix(final_prose,4000),"failed":failed}),
	)?;
	// Explicit prompt bound before the model/transport allocates a request.
	ensure!(prompt.len() <= 2 * 1024 * 1024, "synthesis prompt exceeds byte limit");
	let synth_usage = Mutex::new(Accounting::default());
	let story = ask(
		synth,
		include_str!("../prompts/trace-report/synth-system.md"),
		prompt,
		&synth_usage,
		cancellation,
		65536,
	)?;
	let tiny_usage = tiny_usage
		.into_inner()
		.map_err(|_| anyhow::anyhow!("usage lock poisoned"))?;
	let synth_usage = synth_usage
		.into_inner()
		.map_err(|_| anyhow::anyhow!("usage lock poisoned"))?;
	let mut diagnostics = vec![format!(
		"Source contains {} normalized entries from {} recorded events; normalization/server limits \
		 may omit content. Map inputs excerpt prose to 2000 and each result to 1400 UTF-16 units. \
		 Generated narrative is model-written, not verification evidence.",
		trace.entries.len(),
		trace.total_events
	)];
	if run.is_none() {
		diagnostics.push("Run metadata unavailable; report uses trace alone.".into());
	}
	let mut markdown = render::report(
		options,
		&metadata,
		&turn_log,
		&story,
		tiny.model_id(),
		synth.model_id(),
		&tiny_usage,
		&synth_usage,
	)?;
	markdown.push_str("\n");
	for diagnostic in &diagnostics {
		markdown.push_str(&format!("Note: {diagnostic}\n"));
	}
	ensure!(markdown.len() <= MAX_REPORT, "report including diagnostics exceeds byte limit");
	cancellation.heartbeat()?;
	Ok(Report {
		version: VERSION,
		markdown,
		items: items.len(),
		turns,
		tiny_usage,
		synth_usage,
		diagnostics,
	})
}

/// Fetch only normalized data through the existing bounded/cancellable
/// transport. Base URLs must have no credentials, query or fragment. IDs are
/// path segments.
pub fn fetch(
	base: &str,
	run: &str,
	trace: &str,
	cancellation: &CancelToken,
) -> Result<(Trace, Option<Run>)> {
	identity(run)?;
	identity(trace)?;
	let base = reqwest::Url::parse(base).context("invalid metaharness base URL")?;
	ensure!(
		matches!(base.scheme(), "http" | "https")
			&& base.username().is_empty()
			&& base.password().is_none()
			&& base.query().is_none()
			&& base.fragment().is_none(),
		"invalid metaharness base URL"
	);
	let url = |parts: &[&str]| -> Result<String> {
		let mut url = base.clone();
		{
			let mut path = url
				.path_segments_mut()
				.map_err(|_| anyhow::anyhow!("invalid base path"))?;
			path.pop_if_empty();
			for part in parts {
				path.push(part);
			}
		}
		Ok(url.into())
	};
	let get = |url: String| -> Result<Vec<u8>> {
		let response = crate::transport::Response::open_with_method(
			reqwest::Method::GET,
			url,
			Vec::new(),
			Vec::new(),
			false,
			cancellation,
		)?;
		harvest_runtime::read_bounded(response, MAX_INPUT)
	};
	let mut trace_url = reqwest::Url::parse(&url(&["api", "runs", run, "traces", trace])?)?;
	trace_url.query_pairs_mut().append_pair("tail", "2000");
	let data = decode(&get(trace_url.into())?)?;
	ensure!(data.trace == trace && data.job_name == run, "trace identity mismatch");
	let metadata = (|| -> Result<Run> {
		let bytes = get(url(&["api", "runs", run])?)?;
		let run: Run = serde_json::from_slice(&bytes)?;
		render::metadata(Some(&run), &data.job_name, trace)?;
		Ok(run)
	})()
	.ok();
	cancellation.heartbeat()?;
	Ok((data, metadata))
}

// Preserve first occurrence order; a BTreeMap alone would sort tool names.
fn tool_counts(tools: &[String]) -> Vec<(&str, usize)> {
	let mut counts = BTreeMap::new();
	let mut order = Vec::new();
	for tool in tools {
		if !counts.contains_key(tool.as_str()) {
			order.push(tool.as_str());
		}
		*counts.entry(tool.as_str()).or_insert(0) += 1;
	}
	order.into_iter().map(|tool| (tool, counts[tool])).collect()
}

/// Read a selected session branch through the shared allowlisted display
/// projection, without the legacy metaharness server or an additional codec.
/// Private provider/replay fields, signatures and thinking stay out of prompts.
pub fn from_session(
	bytes: &[u8],
	leaf: &str,
	run: &str,
	trace: &str,
	cancellation: &CancelToken,
) -> Result<Trace> {
	use harvest_protocol::{
		display::{DisplayBlock, DisplayContent, project_branch},
		session::SessionDocument,
	};
	identity(run)?;
	identity(trace)?;
	cancellation.heartbeat()?;
	let document = SessionDocument::decode(bytes)?;
	document.validate_tree()?;
	ensure!(
		document.malformed_records() == 0,
		"session contains malformed records; explicit recovery required"
	);
	let branch = project_branch(&document, leaf)?;
	ensure!(branch.entries.len() <= MAX_ITEMS, "too many session branch entries");
	let total_events = branch.entries.len() as u64;
	let mut entries = Vec::new();
	for entry in branch.entries {
		cancellation.heartbeat()?;
		match entry.content {
			DisplayContent::Message { message } => {
				let mut text = Vec::new();
				let mut tools = Vec::new();
				for block in message.blocks {
					match block {
						DisplayBlock::Text { text: body } => text.push(body),
						DisplayBlock::ToolCall { name, .. } => tools.push(name),
						_ => {},
					}
				}
				let text = text.join("\n");
				match message.role.as_str() {
					"assistant" => entries.push(Entry::Assistant {
						model: message
							.model
							.filter(|s| !s.is_empty())
							.unwrap_or_else(|| "unknown".into()),
						text,
						tools,
					}),
					"toolResult" => entries.push(Entry::ToolResult {
						tool: message.tool_name.unwrap_or_else(|| "unknown".into()),
						is_error: message.is_error,
						text,
					}),
					_ => {},
				}
			},
			DisplayContent::Compaction { .. } => entries.push(Entry::Notice {
				text: "Persisted session compaction boundary (summary omitted from this analysis \
				       view)."
					.into(),
			}),
			DisplayContent::BranchSummary { .. } => entries.push(Entry::Notice {
				text: "Persisted branch-summary boundary (summary omitted from this analysis view)."
					.into(),
			}),
			_ => {},
		}
	}
	let trace = Trace { job_name: run.into(), trace: trace.into(), entries, total_events };
	validate(&trace)?;
	Ok(trace)
}
