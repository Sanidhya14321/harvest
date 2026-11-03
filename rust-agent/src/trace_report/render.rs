use std::io::Write;

use anyhow::{Result, ensure};
use harvest_runtime::BoundedWriter;

use super::{Accounting, Item, MAX_REPORT, Options, Run};

// Markdown remains report data (not terminal control text). Escape transcript
// labels so a tool/model name cannot create report headings or links.
fn inline(text: &str) -> String {
	let mut output = String::new();
	for c in text.chars() {
		if c.is_control() {
			output.push(' ');
		} else {
			if "\\`*_{}[]<>()#!|".contains(c) {
				output.push('\\');
			}
			output.push(c);
		}
	}
	output
}
fn prose(text: &str) -> String {
	text
		.chars()
		.filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
		.collect()
}
pub(super) fn turn_log(items: &[Item], sentences: &[Option<String>]) -> Result<String> {
	let mut output = BoundedWriter::new(MAX_REPORT);
	writeln!(output, "### Turn Log\n")?;
	for (index, item) in items.iter().enumerate() {
		match item {
			Item::Notice { text } => {
				writeln!(output, "{}. **— harness: notice: \"{}\"**", index + 1, inline(text))?
			},
			Item::Turn { model, tools, results, .. } => {
				let named: Vec<_> = super::tool_counts(tools)
					.into_iter()
					.map(|(t, n)| {
						if n > 1 {
							format!("`{}` ×{n}", inline(t))
						} else {
							format!("`{}`", inline(t))
						}
					})
					.collect();
				let tools = if tools.is_empty() {
					"prose only (no tool calls)".into()
				} else {
					format!("tools called: {}", named.join(", "))
				};
				let errors: Vec<_> = results
					.iter()
					.filter(|r| r.is_error)
					.map(|r| format!("`{}`", inline(&r.tool)))
					.collect();
				let note = if errors.is_empty() {
					String::new()
				} else {
					format!(" (errored: {})", errors.join(", "))
				};
				writeln!(output, "{}. **[{}]** {tools}{note}.", index + 1, inline(model))?;
				writeln!(
					output,
					"   - Grounded action: {}",
					prose(
						sentences[index]
							.as_deref()
							.unwrap_or("(summary unavailable)")
					)
				)?;
			},
		}
	}
	Ok(String::from_utf8(output.into_inner())?.trim_end().into())
}
pub(super) fn metadata(
	run: Option<&Run>,
	expected_run: &str,
	trace: &str,
) -> Result<(String, bool)> {
	let Some(run) = run else {
		return Ok((String::new(), false));
	};
	ensure!(run.run.job_name == expected_run, "run metadata identity mismatch");
	ensure!(run.traces.len() <= 10000, "too many run traces");
	for text in [&run.run.benchmark, &run.run.dataset, &run.run.models, &run.run.job_name] {
		super::label(text)?;
	}
	let mut output = format!(
		"Benchmark: {} ({})\nConfigured model: {}",
		inline(&run.run.benchmark),
		inline(&run.run.dataset),
		inline(&run.run.models)
	);
	let mut matching = run.traces.iter().filter(|r| r.name == trace);
	let row = matching.next();
	ensure!(matching.next().is_none(), "duplicate trace metadata");
	let mut failed = false;
	if let Some(row) = row {
		super::label(&row.task)?;
		super::label(&row.status)?;
		ensure!(
			row.reward.is_none_or(f64::is_finite)
				&& row.cost_usd.is_none_or(|n| n.is_finite() && n >= 0.0)
				&& row
					.duration_ms
					.is_none_or(|n| n.is_finite() && n >= 0.0 && n < 1e15),
			"invalid run metrics"
		);
		failed = row.status == "fail";
		let duration = row.duration_ms.map_or_else(
			|| "?".into(),
			|ms| {
				let seconds = (ms / 1000.0).round() as u64;
				format!("{}m{:02}s", seconds / 60, seconds % 60)
			},
		);
		let reward = row.reward.map_or_else(|| "?".into(), |n| n.to_string());
		let cost = row
			.cost_usd
			.map_or_else(|| "?".into(), |n| format!("{n:.2}"));
		output.push_str(&format!(
			"\nTask: {} — status: {} (reward {reward}), cost ${cost}, duration {duration}",
			inline(&row.task),
			inline(&row.status.to_uppercase())
		));
	}
	Ok((output, failed))
}
fn usage(model: &str, u: &Accounting) -> String {
	format!(
		"{}: {} calls, {} input / {} output / {} cache-read / {} cache-write tokens; {} calls with \
		 unavailable usage; cost unavailable",
		inline(model),
		u.calls,
		u.input,
		u.output,
		u.cache_read,
		u.cache_write,
		u.unavailable_calls
	)
}
pub(super) fn report(
	options: &Options,
	metadata: &str,
	turn_log: &str,
	story: &str,
	tiny: &str,
	synth: &str,
	tiny_usage: &Accounting,
	synth_usage: &Accounting,
) -> Result<String> {
	let mut output = BoundedWriter::new(MAX_REPORT);
	writeln!(output, "# Trace report: {} / {}\n", inline(&options.run), inline(&options.trace))?;
	if !metadata.is_empty() {
		writeln!(output, "{metadata}\n")?;
	}
	writeln!(
		output,
		"{turn_log}\n\n{}\n\n---\n_{}; {}_",
		prose(story.trim()),
		usage(tiny, tiny_usage),
		usage(synth, synth_usage)
	)?;
	Ok(String::from_utf8(output.into_inner())?)
}
