//! `sessions` tool: list, inspect and rename persisted sessions for the
//! workspace (decoded via `session_format`). Live-session management
//! (create/send/stop) needs a host-provided live registry, which the Rust
//! runtime does not wire yet; those actions report that, as TS headless
//! hosts do.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::session_format::{SESSION_LIMIT, SessionDocument, decoded_string};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionsArgs {
	action:     String,
	session_id: Option<String>,
	title:      Option<String>,
	message:    Option<String>,
}

/// `list`/`inspect` are reads; everything else executes.
pub fn requires_approval(arguments: &str) -> bool {
	serde_json::from_str::<serde_json::Value>(arguments)
		.ok()
		.and_then(|value| {
			value
				.get("action")
				.and_then(|a| a.as_str())
				.map(str::to_owned)
		})
		.is_none_or(|action| action != "list" && action != "inspect")
}

struct Summary {
	path:    PathBuf,
	id:      String,
	title:   Option<String>,
	entries: usize,
}

fn summarize(fs: &harvest_runtime::JailedFs, path: &Path, cwd: &Path) -> Option<Summary> {
	let bytes = fs.read(path, SESSION_LIMIT).ok()?;
	let document = SessionDocument::decode(&bytes).ok()?;
	let header = document.header();
	let recorded = PathBuf::from(decoded_string(header, "cwd").ok()?);
	if recorded != cwd {
		return None;
	}
	Some(Summary {
		path:    path.to_owned(),
		id:      decoded_string(header, "id").unwrap_or_default(),
		title:   decoded_string(header, "title").ok(),
		entries: document.entries().len(),
	})
}

fn find(
	fs: &harvest_runtime::JailedFs,
	dir: &Path,
	cwd: &Path,
	session_id: &str,
) -> Result<Summary> {
	if session_id.trim().is_empty() {
		bail!("sessionId must not be empty");
	}
	let wanted = session_id.to_lowercase();
	let mut prefixes = Vec::new();
	for path in crate::commands::render::list_sessions(dir) {
		let Some(summary) = summarize(fs, &path, cwd) else {
			continue;
		};
		let stem = path
			.file_stem()
			.map(|s| s.to_string_lossy().to_lowercase())
			.unwrap_or_default();
		if summary.id.to_lowercase() == wanted || stem == wanted {
			return Ok(summary);
		}
		if summary.id.to_lowercase().starts_with(&wanted) {
			prefixes.push(summary);
		}
	}
	if prefixes.len() > 1 {
		bail!("Session prefix {session_id} is ambiguous; use the full id.");
	}
	prefixes
		.pop()
		.ok_or_else(|| anyhow::anyhow!("Session {session_id} was not found in this workspace."))
}

pub fn sessions(cwd: &Path, arguments: &str) -> Result<String> {
	sessions_in(cwd, None, arguments)
}

/// The directory is a host grant. Every candidate must also record this
/// workspace.
pub fn sessions_in(cwd: &Path, directory: Option<&Path>, arguments: &str) -> Result<String> {
	let cwd = cwd.canonicalize()?;
	let args: SessionsArgs = serde_json::from_str(arguments)?;
	if !matches!(args.action.as_str(), "list" | "create") && args.session_id.is_none() {
		bail!("sessionId is required except for \"list\" and \"create\"");
	}
	let dir = match directory {
		Some(dir) => dir.to_owned(),
		None => crate::commands::render::default_session_dir(&cwd)?,
	};
	if !dir.exists() && args.action == "list" {
		return Ok("Sessions (0):\n".into());
	}
	let fs = harvest_runtime::JailedFs::open(&dir)?;
	match args.action.as_str() {
		"list" => {
			let summaries: Vec<Summary> = crate::commands::render::list_sessions(&dir)
				.iter()
				.filter_map(|path| summarize(&fs, path, &cwd))
				.collect();
			let lines: Vec<String> = summaries
				.iter()
				.map(|s| {
					format!(
						"- {} (session, persisted, {} entries): {}",
						s.id,
						s.entries,
						s.title.as_deref().unwrap_or("(untitled)")
					)
				})
				.collect();
			Ok(format!("Sessions ({}):\n{}", summaries.len(), lines.join("\n")))
		},
		"inspect" => {
			let s = find(&fs, &dir, &cwd, args.session_id.as_deref().unwrap_or_default())?;
			Ok(format!(
				"Session {}: {} (session, persisted). File: {}. Entries: {}.",
				s.id,
				s.title.as_deref().unwrap_or("(untitled)"),
				s.path.display(),
				s.entries
			))
		},
		"rename" => {
			let title = args.title.as_deref().map(str::trim).unwrap_or_default();
			if title.is_empty() {
				bail!("\"rename\" requires \"title\".");
			}
			let s = find(&fs, &dir, &cwd, args.session_id.as_deref().unwrap_or_default())?;
			let fs = harvest_runtime::JailedFs::open(&dir)?;
			let file = s
				.path
				.file_name()
				.map(PathBuf::from)
				.context("invalid session path")?;
			let _lease = crate::durable_session::lock_session(&fs, &file)?;
			let document = SessionDocument::decode(&fs.read(&file, SESSION_LIMIT)?)?;
			let bytes = document.retitled(title, Some("user"), &chrono::Utc::now().to_rfc3339())?;
			fs.atomic_write(&file, &bytes)?;
			Ok(format!("Renamed session {} to \"{title}\".", s.id))
		},
		"create" => bail!(
			"Live session creation is not wired for this host (interactive TUI registers the \
			 live-session factory; headless hosts report this instead of creating)."
		),
		"send" => {
			if args.message.as_deref().is_none_or(|m| m.trim().is_empty()) {
				bail!("\"send\" requires \"message\".");
			}
			let s = find(&fs, &dir, &cwd, args.session_id.as_deref().unwrap_or_default())?;
			bail!("Session {} has no live runtime (status: persisted).", s.id)
		},
		"stop" => {
			let s = find(&fs, &dir, &cwd, args.session_id.as_deref().unwrap_or_default())?;
			bail!("Session {} has no live runtime (status: persisted).", s.id)
		},
		other => bail!("Unknown sessions action \"{other}\"."),
	}
}
