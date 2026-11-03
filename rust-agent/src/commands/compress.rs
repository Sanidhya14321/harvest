//! `harvest compress` — rewrite text files into the dense prompt register.
//!
//! Ported from `packages/coding-agent/src/compress/`. One isolated model
//! conversation per file with exactly two tools: `rewrite` submits a draft and
//! its declared losses, `approve` accepts the newest reviewed draft. Only an
//! approved draft is ever written, and every write goes through `JailedFs`.
use std::{
	io::Write,
	path::{Path, PathBuf},
	sync::{
		Mutex,
		atomic::{AtomicUsize, Ordering},
	},
};

use anyhow::{Context, Result, bail};
use handlebars::{Handlebars, no_escape};
use harvest_runtime::JailedFs;
use serde_json::{Value, json};

use crate::{
	chat::{self, ChatMessage, ToolSpec},
	config::{self, Overrides},
	provider::{HttpModel, Model},
};

const SYSTEM_PROMPT: &str = include_str!("../prompts/compress/system.md");
const REQUEST_PROMPT: &str = include_str!("../prompts/compress/request.md");
const REVIEW_PROMPT: &str = include_str!("../prompts/compress/review.md");
const REWRITE_DESCRIPTION: &str = include_str!("../prompts/compress/rewrite.md");
const APPROVE_DESCRIPTION: &str = include_str!("../prompts/compress/approve.md");
const NO_LOSSES: &str = include_str!("../prompts/compress/no-losses.md");
const CLOSING_FINAL: &str = include_str!("../prompts/compress/closing-final.md");
const CLOSING_REVIEW: &str = include_str!("../prompts/compress/closing-review.md");
const LOSS_PREVIEW: usize = 200;
/// Model steps allowed inside one turn before it is treated as settled.
const TURN_STEP_LIMIT: usize = 16;
const SOURCE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, clap::Args)]
pub struct CompressArgs {
	/// Files or glob patterns to compress
	#[arg(required = true)]
	files:    Vec<String>,
	/// Write the approved text here instead of stdout (single file)
	#[arg(short, long)]
	out:      Option<PathBuf>,
	/// Overwrite each source file with its approved text
	#[arg(short = 'i', long = "in-place")]
	in_place: bool,
	/// Maximum drafts per file before giving up
	#[arg(short, long, default_value_t = 3, allow_negative_numbers = true)]
	rounds:   i64,
	/// Files compressed concurrently
	#[arg(short = 'n', long, default_value_t = 4, allow_negative_numbers = true)]
	agents:   i64,
	/// Model selector
	#[arg(short, long)]
	model:    Option<String>,
}

#[derive(Clone)]
struct Loss {
	content: String,
	reason:  String,
}

#[derive(Clone)]
struct Draft {
	round:  usize,
	text:   String,
	losses: Vec<Loss>,
}

struct Metrics {
	source_words:  usize,
	draft_words:   usize,
	source_tokens: u32,
	draft_tokens:  u32,
	ratio:         f64,
}

fn words(text: &str) -> usize {
	text.split_whitespace().count()
}

fn tokens(text: &str) -> u32 {
	harvest_tokenizer::Encoding::O200kBase.count(text)
}

/// Draft ledger shared by the protocol tools and the command loop.
struct Protocol {
	source_words:  usize,
	source_tokens: u32,
	drafts:        Vec<Draft>,
	reviewed:      usize,
	approved:      bool,
	verdict:       Option<String>,
}

impl Protocol {
	fn new(source: &str) -> Self {
		Self {
			source_words:  words(source),
			source_tokens: tokens(source),
			drafts:        Vec::new(),
			reviewed:      0,
			approved:      false,
			verdict:       None,
		}
	}

	fn latest(&self) -> Option<&Draft> {
		self.drafts.last()
	}

	fn metrics(&self, draft: &Draft) -> Metrics {
		let draft_tokens = tokens(&draft.text);
		Metrics {
			source_words: self.source_words,
			draft_words: words(&draft.text),
			source_tokens: self.source_tokens,
			draft_tokens,
			ratio: if self.source_tokens == 0 {
				0.0
			} else {
				(f64::from(self.source_tokens) - f64::from(draft_tokens)) / f64::from(self.source_tokens)
			},
		}
	}

	fn submit(&mut self, text: String, losses: Vec<Loss>) -> &Draft {
		self.drafts.push(Draft { round: self.drafts.len() + 1, text, losses });
		self.approved = false;
		self.verdict = None;
		self.drafts.last().expect("just pushed")
	}

	fn accept(&mut self, verdict: String) -> Result<usize> {
		let Some(draft) = self.latest() else {
			bail!("Call rewrite before approve: there is no draft to accept");
		};
		if draft.round > self.reviewed {
			bail!(
				"Draft {} has not been reviewed yet. End this turn; the review turn arrives next, and \
				 you approve there.",
				draft.round
			);
		}
		let round = draft.round;
		self.approved = true;
		self.verdict = Some(verdict);
		Ok(round)
	}

	fn rewrite(&mut self, arguments: &Value) -> Result<String> {
		let object = arguments.as_object().context("rewrite received invalid arguments")?;
		if object.keys().any(|k| k != "text" && k != "losses") {
			bail!("rewrite received invalid arguments: unexpected key");
		}
		let text = object
			.get("text")
			.and_then(Value::as_str)
			.filter(|t| !t.is_empty())
			.context("rewrite received invalid arguments: text must be a non-empty string")?;
		let mut losses = Vec::new();
		for loss in object
			.get("losses")
			.and_then(Value::as_array)
			.context("rewrite received invalid arguments: losses must be an array")?
		{
			let field = |key: &str| {
				loss.get(key)
					.and_then(Value::as_str)
					.filter(|v| !v.is_empty())
					.map(str::to_owned)
					.with_context(|| format!("rewrite received invalid arguments: losses.{key} must be a non-empty string"))
			};
			losses.push(Loss { content: field("content")?, reason: field("reason")? });
		}
		let draft = self.submit(text.to_owned(), losses).clone();
		let metrics = self.metrics(&draft);
		Ok(format!(
			"Draft {} recorded: {} → {} tokens ({:.1}% smaller), {} declared loss(es). A review turn \
			 follows.",
			draft.round,
			metrics.source_tokens,
			metrics.draft_tokens,
			metrics.ratio * 100.0,
			draft.losses.len()
		))
	}

	fn approve(&mut self, arguments: &Value) -> Result<String> {
		let object = arguments.as_object().context("approve received invalid arguments")?;
		if object.keys().any(|k| k != "verdict") {
			bail!("approve received invalid arguments: unexpected key");
		}
		let verdict = object
			.get("verdict")
			.and_then(Value::as_str)
			.filter(|v| !v.is_empty())
			.context("approve received invalid arguments: verdict must be a non-empty string")?;
		let round = self.accept(verdict.to_owned())?;
		Ok(format!("Draft {round} approved. The run ends here."))
	}
}

fn tool_specs() -> Result<Vec<ToolSpec>> {
	let loss = json!({
		"type": "object",
		"properties": {
			"content": {"type": "string", "minLength": 1, "description": "the dropped source content, quoted or described precisely"},
			"reason": {"type": "string", "minLength": 1, "description": "why the compressed text is still correct without it"},
		},
		"required": ["content", "reason"],
		"additionalProperties": false,
	});
	Ok(vec![
		ToolSpec::from_schema(
			"rewrite".into(),
			REWRITE_DESCRIPTION.trim().into(),
			json!({
				"type": "object",
				"description": "submit a compressed draft together with everything it drops",
				"properties": {
					"text": {"type": "string", "minLength": 1, "description": "the complete compressed text, ready to ship verbatim"},
					"losses": {"type": "array", "items": loss, "description": "every claim, qualifier, example, default, or exact string deliberately dropped; empty array only when the draft loses nothing"},
				},
				"required": ["text", "losses"],
				"additionalProperties": false,
			}),
		)?,
		ToolSpec::from_schema(
			"approve".into(),
			APPROVE_DESCRIPTION.trim().into(),
			json!({
				"type": "object",
				"description": "accept the newest draft as the final output",
				"properties": {
					"verdict": {"type": "string", "minLength": 1, "description": "why the newest draft is acceptable as the final output"},
				},
				"required": ["verdict"],
				"additionalProperties": false,
			}),
		)?,
	])
}

fn render(template: &str, data: &Value) -> Result<String> {
	let mut engine = Handlebars::new();
	engine.register_escape_fn(no_escape);
	Ok(engine.render_template(template, data)?)
}

/// Send one prompt and drive tool calls until the model settles.
fn turn(
	model: &dyn Model,
	messages: &mut Vec<ChatMessage>,
	tools: &[ToolSpec],
	protocol: &mut Protocol,
	text: String,
) -> Result<()> {
	messages.push(chat::user(text));
	for _ in 0..TURN_STEP_LIMIT {
		let reply = model.complete(messages, tools)?;
		let calls = reply.tool_calls.clone().unwrap_or_default();
		messages.push(reply);
		if calls.is_empty() {
			return Ok(());
		}
		for call in calls {
			let arguments: Value = serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);
			let outcome = match call.function.name.as_str() {
				"rewrite" => protocol.rewrite(&arguments),
				"approve" => protocol.approve(&arguments),
				other => Err(anyhow::anyhow!("Unknown tool: {other}")),
			};
			let content = outcome.unwrap_or_else(|error| format!("Error: {error:#}"));
			messages.push(chat::tool_result(&call.id, content));
		}
	}
	Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
	Approved,
	Unapproved,
	Stalled,
	Cancelled,
}

impl Status {
	const fn name(self) -> &'static str {
		match self {
			Self::Approved => "approved",
			Self::Unapproved => "unapproved",
			Self::Stalled => "stalled",
			Self::Cancelled => "cancelled",
		}
	}
}

struct FileResult {
	path:        PathBuf,
	status:      Status,
	draft:       Option<Draft>,
	metrics:     Option<Metrics>,
	rounds:      usize,
	output_path: Option<PathBuf>,
	error:       Option<String>,
}

struct Job<'a> {
	target:          &'a Path,
	invocation_dir:  &'a Path,
	args:            &'a CompressArgs,
	max_rounds:      usize,
	emit_to_stdout:  bool,
}

fn build_model(cwd: &Path, selector: Option<&str>) -> Result<HttpModel> {
	let environment = config::environment_from(|key| std::env::var(key).ok())?;
	let resolved = crate::configuration::load(
		cwd,
		&Overrides { model: selector.map(str::to_owned), ..Overrides::default() },
		&environment,
		&crate::configuration::LoadOptions {
			user_directory: config::agent_dir_from(None, |key| std::env::var_os(key)),
			environment_profile: std::env::var("OMP_PROFILE").ok(),
			..Default::default()
		},
	)?;
	HttpModel::from_configuration(cwd, &resolved, false, false)
}

fn jailed(path: &Path) -> Result<(JailedFs, PathBuf)> {
	let parent = path.parent().context("path has no parent directory")?;
	let name = path.file_name().context("path has no file name")?;
	Ok((JailedFs::open(parent)?, PathBuf::from(name)))
}

fn compress_file(job: &Job<'_>, cwd: &Path, nonce: &str) -> Result<FileResult> {
	let (fs, name) = jailed(job.target)?;
	let source = String::from_utf8(fs.read(&name, SOURCE_LIMIT)?)
		.with_context(|| format!("{} is not UTF-8 text", job.target.display()))?;
	if source.trim().is_empty() {
		return Ok(FileResult {
			path:        job.target.to_path_buf(),
			status:      Status::Stalled,
			draft:       None,
			metrics:     None,
			rounds:      0,
			output_path: None,
			error:       Some("no text to compress".into()),
		});
	}
	let mut protocol = Protocol::new(&source);
	let model = build_model(cwd, job.args.model.as_deref())?;
	let tools = tool_specs()?;
	let mut messages = vec![chat::system(SYSTEM_PROMPT.trim())];
	let request = render(REQUEST_PROMPT, &json!({
			"path": shorten_path(job.target),
			"source_size": format!("Source: {} words, {} tokens.", protocol.source_words, protocol.source_tokens),
			"source": source,
			"nonce": nonce,
		}))?;
	turn(&model, &mut messages, &tools, &mut protocol, request)?;
	let mut reviewed = 0;
	while !protocol.approved {
		let Some(draft) = protocol.latest().cloned() else { break };
		if draft.round == reviewed || draft.round > job.max_rounds {
			break;
		}
		reviewed = draft.round;
		protocol.reviewed = protocol.reviewed.max(draft.round);
		let review = render_review(&protocol, &draft, nonce, job.max_rounds)?;
		turn(&model, &mut messages, &tools, &mut protocol, review)?;
	}
	let draft = protocol.latest().cloned();
	let status = if protocol.approved {
		Status::Approved
	} else if draft.is_some() {
		Status::Unapproved
	} else {
		Status::Stalled
	};
	let mut output_path = None;
	if status == Status::Approved
		&& let Some(draft) = &draft
		&& !job.emit_to_stdout
	{
		let destination = if job.args.in_place {
			job.target.to_path_buf()
		} else {
			std::path::absolute(job.invocation_dir.join(job.args.out.as_deref().unwrap_or(Path::new(""))))?
		};
		let mut text = draft.text.clone();
		if !text.ends_with('\n') {
			text.push('\n');
		}
		let (out_fs, out_name) = jailed(&destination)?;
		out_fs.atomic_write(&out_name, text.as_bytes())?;
		output_path = Some(destination);
	}
	let _ = &protocol.verdict;
	Ok(FileResult {
		path: job.target.to_path_buf(),
		status,
		metrics: draft.as_ref().map(|d| protocol.metrics(d)),
		draft,
		rounds: protocol.drafts.len(),
		output_path,
		error: None,
	})
}

fn render_review(protocol: &Protocol, draft: &Draft, nonce: &str, max_rounds: usize) -> Result<String> {
	let metrics = protocol.metrics(draft);
	let losses = if draft.losses.is_empty() {
		NO_LOSSES.trim().to_owned()
	} else {
		draft
			.losses
			.iter()
			.map(|loss| format!("- {}\n  Accepted because: {}", loss.content, loss.reason))
			.collect::<Vec<_>>()
			.join("\n")
	};
	let closing = if draft.round >= max_rounds {
		render(CLOSING_FINAL.trim(), &json!({"max_rounds": max_rounds}))?
	} else {
		CLOSING_REVIEW.trim().to_owned()
	};
	render(REVIEW_PROMPT, &json!({
		"round": draft.round.to_string(),
		"metrics": format!(
			"{} → {} words, {} → {} tokens ({:.1}% smaller).",
			metrics.source_words, metrics.draft_words, metrics.source_tokens, metrics.draft_tokens, metrics.ratio * 100.0
		),
		"losses": losses,
		"draft": draft.text,
		"nonce": nonce,
		"closing": closing,
	}))
}

fn shorten_path(path: &Path) -> String {
	let text = path.display().to_string();
	if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned())
		&& !home.is_empty()
		&& let Some(rest) = text.strip_prefix(&home)
		&& (rest.is_empty() || rest.starts_with('/'))
	{
		return format!("~{rest}");
	}
	text
}

/// Strip control characters so model text cannot drive the terminal.
fn sanitize(text: &str) -> String {
	text.chars()
		.map(|c| if c == '\t' { ' ' } else { c })
		.filter(|c| !c.is_control() || *c == '\n')
		.collect()
}

fn glob_regex(pattern: &str) -> Result<regex::Regex> {
	let mut out = String::from("^");
	let chars: Vec<char> = pattern.chars().collect();
	let mut i = 0;
	let mut in_brace = false;
	while i < chars.len() {
		let c = chars[i];
		match c {
			'*' if chars.get(i + 1) == Some(&'*') => {
				i += 1;
				if chars.get(i + 1) == Some(&'/') {
					i += 1;
					out.push_str("(?:.*/)?");
				} else {
					out.push_str(".*");
				}
			},
			'*' => out.push_str("[^/]*"),
			'?' => out.push_str("[^/]"),
			'[' => {
				let end = chars[i..].iter().position(|ch| *ch == ']').map(|p| p + i);
				if let Some(end) = end {
					let body: String = chars[i + 1..end].iter().collect();
					let body = body.strip_prefix('!').map_or(body.clone(), |b| format!("^{b}"));
					out.push('[');
					out.push_str(&body.replace('\\', "\\\\"));
					out.push(']');
					i = end;
				} else {
					out.push_str("\\[");
				}
			},
			'{' => {
				in_brace = true;
				out.push_str("(?:");
			},
			'}' if in_brace => {
				in_brace = false;
				out.push(')');
			},
			',' if in_brace => out.push('|'),
			_ => out.push_str(&regex::escape(&c.to_string())),
		}
		i += 1;
	}
	out.push('$');
	Ok(regex::Regex::new(&out)?)
}

fn walk(root: &Path, dir: &Path, regex: &regex::Regex, found: &mut Vec<PathBuf>) -> Result<()> {
	let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
	for entry in entries.flatten() {
		let path = entry.path();
		let kind = entry.file_type()?;
		if kind.is_dir() {
			walk(root, &path, regex, found)?;
		} else if kind.is_file()
			&& let Ok(relative) = path.strip_prefix(root)
			&& regex.is_match(&relative.to_string_lossy().replace('\\', "/"))
		{
			found.push(path);
		}
	}
	Ok(())
}

/// Expand patterns into a deduplicated, sorted list of absolute file paths.
fn resolve_targets(patterns: &[String], cwd: &Path) -> Result<Vec<PathBuf>> {
	let mut found = std::collections::BTreeSet::new();
	for pattern in patterns {
		if pattern.contains(['*', '?', '[', ']', '{', '}']) {
			let (base, rest) = if Path::new(pattern).is_absolute() {
				(PathBuf::from("/"), pattern.trim_start_matches('/').to_owned())
			} else {
				(cwd.to_path_buf(), pattern.clone())
			};
			let regex = glob_regex(&rest)?;
			let mut matches = Vec::new();
			walk(&base, &base, &regex, &mut matches)?;
			if matches.is_empty() {
				bail!("No files matched \"{pattern}\"");
			}
			found.extend(matches);
			continue;
		}
		let resolved = std::path::absolute(cwd.join(pattern))?;
		if !resolved.is_file() {
			bail!("Not a file: {}", shorten_path(&resolved));
		}
		found.insert(resolved);
	}
	Ok(found.into_iter().collect())
}

fn report_file(file: &FileResult, emit_to_stdout: bool) {
	let label = shorten_path(&file.path);
	// Standalone command output: progress goes to stderr, approved text to stdout.
	let mut err = std::io::stderr().lock();
	if let Some(error) = &file.error {
		let _ = writeln!(err, "  {label}: {} — {}", file.status.name(), sanitize(error));
		return;
	}
	let size = file.metrics.as_ref().map_or_else(
		|| "no draft".to_owned(),
		|m| format!("{} → {} tok ({:.1}%)", m.source_tokens, m.draft_tokens, m.ratio * 100.0),
	);
	let losses = file.draft.as_ref().map_or(&[][..], |d| &d.losses[..]);
	let _ = writeln!(
		err,
		"  {label}: {}, {size}, {} draft(s), {} loss(es)",
		file.status.name(),
		file.rounds,
		losses.len()
	);
	for loss in losses {
		let content = if loss.content.chars().count() > LOSS_PREVIEW {
			format!("{}…", loss.content.chars().take(LOSS_PREVIEW).collect::<String>())
		} else {
			loss.content.clone()
		};
		let _ = writeln!(err, "    - {}", sanitize(&content));
		let _ = writeln!(err, "      {}", sanitize(&loss.reason));
	}
	if file.status != Status::Approved {
		let _ = writeln!(err, "      nothing written");
		return;
	}
	if let Some(path) = &file.output_path {
		let _ = writeln!(err, "      wrote {}", shorten_path(path));
	}
	if emit_to_stdout && let Some(draft) = &file.draft {
		let _ = writeln!(std::io::stdout().lock(), "{}", draft.text);
	}
}

fn nonce() -> String {
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |d| d.as_nanos());
	let mixed = (seed ^ (u128::from(std::process::id()) << 64)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
	format!("{:08x}", (mixed >> 32) as u32)
}

pub fn execute(args: CompressArgs) -> Result<u8> {
	if args.files.is_empty() {
		bail!("compress requires at least one file or glob pattern");
	}
	if args.rounds <= 0 {
		bail!("--rounds must be a positive integer");
	}
	if args.agents <= 0 {
		bail!("--agents must be a positive integer");
	}
	if args.in_place && args.out.is_some() {
		bail!("--in-place and --out are mutually exclusive");
	}
	let invocation_dir = std::env::current_dir()?;
	let cwd = invocation_dir.clone();
	let targets = resolve_targets(&args.files, &invocation_dir)?;
	if targets.is_empty() {
		bail!("No files to compress");
	}
	if targets.len() > 1 && !args.in_place {
		bail!(
			"{} files matched; pass --in-place to rewrite them (--out takes a single file)",
			targets.len()
		);
	}
	let emit_to_stdout = targets.len() == 1 && !args.in_place && args.out.is_none();
	let max_rounds = usize::try_from(args.rounds)?;
	let concurrency = usize::try_from(args.agents)?.min(targets.len());
	eprintln!(
		"Compressing {} file(s){}",
		targets.len(),
		args.model.as_deref().map(|m| format!(" with {m}")).unwrap_or_default()
	);
	let run_nonce = nonce();
	let next = AtomicUsize::new(0);
	let results: Mutex<Vec<Option<FileResult>>> =
		Mutex::new((0..targets.len()).map(|_| None).collect());
	std::thread::scope(|scope| {
		for _ in 0..concurrency {
			scope.spawn(|| {
				loop {
					let index = next.fetch_add(1, Ordering::SeqCst);
					let Some(target) = targets.get(index) else { break };
					let job = Job {
						target,
						invocation_dir: &invocation_dir,
						args: &args,
						max_rounds,
						emit_to_stdout,
					};
					let result = compress_file(&job, &cwd, &run_nonce).unwrap_or_else(|error| FileResult {
						path:        target.clone(),
						status:      Status::Cancelled,
						draft:       None,
						metrics:     None,
						rounds:      0,
						output_path: None,
						error:       Some(format!("{error:#}")),
					});
					report_file(&result, emit_to_stdout);
					results.lock().expect("results lock")[index] = Some(result);
				}
			});
		}
	});
	let files: Vec<FileResult> = results.into_inner().expect("results lock").into_iter().flatten().collect();
	let (mut source_tokens, mut draft_tokens, mut approved) = (0u64, 0u64, 0usize);
	for file in &files {
		if file.status == Status::Approved {
			approved += 1;
			if let Some(m) = &file.metrics {
				source_tokens += u64::from(m.source_tokens);
				draft_tokens += u64::from(m.draft_tokens);
			}
		}
	}
	if files.len() > 1 {
		let percent = if source_tokens == 0 {
			"0.0".to_owned()
		} else {
			format!("{:.1}", (source_tokens as f64 - draft_tokens as f64) / source_tokens as f64 * 100.0)
		};
		eprintln!("Approved {approved}/{}: {source_tokens} → {draft_tokens} tokens ({percent}% smaller)", files.len());
	}
	if !emit_to_stdout && approved == 0 {
		eprintln!("Nothing written");
	}
	Ok(if approved == files.len() { 0 } else { 1 })
}
