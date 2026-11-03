//! `harvest __complete <kind> [-- <prefix>]` — dynamic shell-completion candidates
//! (port of `commands/complete.ts`). Output: one `value\tdescription` line each.
use std::{collections::BTreeSet, io::Write};

use anyhow::Result;

#[derive(clap::Args, Debug)]
pub struct CompleteArgs {
	#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
	pub argv: Vec<String>,
}

/// Strip control chars that would corrupt the tab-separated line protocol.
fn clean(text: &str) -> String {
	text.split(['\t', '\r', '\n'])
		.filter(|s| !s.is_empty())
		.collect::<Vec<_>>()
		.join(" ")
		.trim()
		.to_owned()
}

pub fn execute(args: CompleteArgs) -> Result<u8> {
	let argv: Vec<&str> = args.argv.iter().map(String::as_str).filter(|t| *t != "--").collect();
	let Some(kind) = argv.first().copied() else { return Ok(0) };
	let prefix = if argv.len() > 1 { argv[argv.len() - 1] } else { "" };
	let lines = match kind {
		"models" => complete_models(prefix),
		"sessions" => complete_sessions(prefix)?,
		_ => Vec::new(),
	};
	if !lines.is_empty() {
		let mut out = std::io::stdout().lock();
		writeln!(out, "{}", lines.join("\n"))?;
	}
	Ok(0)
}

fn complete_models(prefix: &str) -> Vec<String> {
	let needle = prefix.to_lowercase();
	let mut seen = BTreeSet::new();
	let mut lines = Vec::new();
	for (provider, id, _) in crate::catalog::models() {
		for candidate in [format!("{provider}/{id}"), id.to_owned()] {
			if !seen.insert(candidate.clone()) {
				continue;
			}
			if !needle.is_empty() && !candidate.to_lowercase().contains(&needle) {
				continue;
			}
			lines.push(format!("{candidate}\t{provider}"));
		}
	}
	lines.sort();
	lines
}

/// Header `title` (or first user text) from a bounded head of a session file.
fn session_label(path: &std::path::Path) -> String {
	use std::io::{BufRead, BufReader, Read};
	let Ok(file) = std::fs::File::open(path) else { return String::new() };
	let reader = BufReader::new(file.take(256 * 1024));
	for line in reader.lines().take(32).map_while(Result::ok) {
		let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
		if let Some(title) = value.get("title").and_then(|t| t.as_str()) {
			return title.to_owned();
		}
		let message = value.get("message");
		if message.and_then(|m| m.get("role")).and_then(|r| r.as_str()) == Some("user") {
			let content = message.and_then(|m| m.get("content"));
			if let Some(text) = content.and_then(|c| c.as_str()) {
				return text.to_owned();
			}
			if let Some(text) = content
				.and_then(|c| c.as_array())
				.and_then(|a| a.iter().find_map(|b| b.get("text").and_then(|t| t.as_str())))
			{
				return text.to_owned();
			}
		}
	}
	String::new()
}

fn complete_sessions(prefix: &str) -> Result<Vec<String>> {
	let cwd = std::env::current_dir()?;
	let dir = super::render::default_session_dir(&cwd)?;
	let mut lines = Vec::new();
	for path in super::render::list_sessions(&dir) {
		let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
		let id = stem.rsplit_once('_').map_or(stem.as_str(), |(_, id)| id).to_owned();
		if !prefix.is_empty() && !id.starts_with(prefix) {
			continue;
		}
		let label: String = clean(&session_label(&path)).chars().take(72).collect();
		lines.push(format!("{id}\t{label}"));
	}
	Ok(lines)
}
