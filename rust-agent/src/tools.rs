use std::{
	io::{self, IsTerminal, Write},
	path::Path,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use serde::Deserialize;

use crate::chat::ToolCall;

const OUTPUT_LIMIT: usize = 20_000;

/// The host owns approval; a model or retrieved skill cannot grant it.
pub trait Approval {
	fn approve(&self, call: &ToolCall) -> Result<bool>;
}

pub struct DenyMutations;
impl Approval for DenyMutations {
	fn approve(&self, _call: &ToolCall) -> Result<bool> {
		Ok(false)
	}
}

pub struct TerminalApproval;
impl Approval for TerminalApproval {
	fn approve(&self, call: &ToolCall) -> Result<bool> {
		if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
			return Ok(false);
		}
		// Approval must show the complete request. Never hide executable suffixes.
		let preview = format!("{:?}", call.function.arguments);
		if preview.len() > 2000 {
			bail!(
				"request is too large for terminal approval; a host approval integration is required"
			);
		}
		write!(io::stderr(), "Approve {} {}? [y/N] ", call.function.name, preview)?;
		io::stderr().flush()?;
		let mut answer = String::new();
		io::stdin().read_line(&mut answer)?;
		Ok(answer.trim().eq_ignore_ascii_case("y"))
	}
}

pub fn invoke(root: &Path, call: &ToolCall) -> Result<String> {
	invoke_with_approval(root, call, &DenyMutations)
}

pub fn invoke_with_approval(root: &Path, call: &ToolCall, policy: &dyn Approval) -> Result<String> {
	invoke_with_cancellation(root, call, policy, &Default::default())
}

pub fn invoke_with_cancellation(
	root: &Path,
	call: &ToolCall,
	policy: &dyn Approval,
	cancellation: &harvest_runtime::cancel::CancelToken,
) -> Result<String> {
	invoke_with_store(root, call, policy, cancellation, &pi_edit::EditStore::new())
}

/// Invoke with a session-scoped edit store: `read` records the snapshots
/// whose `[path#TAG]` headers anchor later `edit` calls.
pub fn invoke_with_store(
	root: &Path,
	call: &ToolCall,
	policy: &dyn Approval,
	cancellation: &harvest_runtime::cancel::CancelToken,
	store: &pi_edit::EditStore,
) -> Result<String> {
	cancellation.heartbeat()?;
	invoke_in_session(root, call, policy, cancellation, store, None, &SessionState::default())
}

/// Session-scoped tool state: conflict ids from `read`/`write` and the todo
/// list.
#[derive(Default)]
pub struct SessionState {
	pub conflicts: crate::conflict::ConflictHistory,
	pub todos:     crate::todo::TodoList,
	/// Staged previews (`ast_edit`), newest last; `xd://resolve` applies the
	/// newest, `xd://reject` discards it.
	pending:       std::sync::Mutex<Vec<crate::ast::EditPlan>>,
	/// Active `checkpoint` (goal) and the report of a `rewind` waiting for the
	/// session loop to branch the context.
	checkpoint:    std::sync::Mutex<CheckpointState>,
}

#[derive(Default)]
struct CheckpointState {
	active:         bool,
	completed:      bool,
	pending_report: Option<String>,
}

impl SessionState {
	fn checkpoint(&self) -> std::sync::MutexGuard<'_, CheckpointState> {
		self
			.checkpoint
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// The report of a successful `rewind`, consumed once by the session loop.
	pub fn take_rewind(&self) -> Option<String> {
		let mut state = self.checkpoint();
		let report = state.pending_report.take()?;
		state.active = false;
		state.completed = true;
		Some(report)
	}
}

#[derive(Deserialize)]
struct CheckpointArgs {
	goal: String,
}

#[derive(Deserialize)]
struct RewindArgs {
	report: String,
}

impl SessionState {
	fn pending(&self) -> std::sync::MutexGuard<'_, Vec<crate::ast::EditPlan>> {
		self
			.pending
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}
}

/// `dispatchResolutionDevice` for `xd://resolve` and `xd://reject`; `None`
/// when `path` is not one of those devices.
fn resolution_device(
	root: &Path,
	path: &str,
	content: &str,
	store: &pi_edit::EditStore,
	state: &SessionState,
) -> Option<Result<String>> {
	let apply = match path.trim() {
		"xd://resolve" => true,
		"xd://reject" => false,
		_ => return None,
	};
	let reason = content.trim();
	let Some(plan) = state.pending().pop() else {
		return Some(if apply {
			Err(anyhow::anyhow!(
				"No pending action to apply — xd://resolve is only valid while a staged preview is \
				 pending."
			))
		} else {
			Ok("Nothing to reject; no pending action remains.".to_owned())
		});
	};
	if !apply {
		return Some(Ok(format!("Discarded: {}. Reason: {reason}", plan.label)));
	}
	Some(match crate::ast::apply_edit(root, &plan, store) {
		Ok(text) => Ok(text),
		Err(error) => {
			// A failed apply keeps the preview pending so it can be rejected or
			// retried.
			let message = error.to_string();
			if message.starts_with("Apply failed") {
				state.pending().push(plan);
			}
			Err(error)
		},
	})
}

/// Invoke inside a persisted session: results over 50 KB spill into
/// `artifacts` (readable back through `read artifact://N`) instead of being
/// capped.
pub fn invoke_in_session(
	root: &Path,
	call: &ToolCall,
	policy: &dyn Approval,
	cancellation: &harvest_runtime::cancel::CancelToken,
	store: &pi_edit::EditStore,
	artifacts: Option<&crate::output::ArtifactStore>,
	state: &SessionState,
) -> Result<String> {
	let name = call.function.name.as_str();
	if call.function.arguments.len() > harvest_runtime::MAX_EVENT_BYTES {
		bail!("tool arguments exceed frame limit");
	}
	if !crate::chat::is_tool(name) {
		bail!("unknown tool {name}");
	}
	if matches!(name, "write" | "edit" | "bash") && !policy.approve(call)? {
		bail!("tool {name} requires human approval");
	}
	cancellation.heartbeat()?;
	invoke_authorized(root, call, cancellation, store, artifacts, state)
}

#[cfg(test)]
fn invoke_approved(root: &Path, call: &ToolCall) -> Result<String> {
	invoke_authorized(
		root,
		call,
		&Default::default(),
		&pi_edit::EditStore::new(),
		None,
		&SessionState::default(),
	)
}

fn invoke_authorized(
	root: &Path,
	call: &ToolCall,
	cancellation: &harvest_runtime::cancel::CancelToken,
	store: &pi_edit::EditStore,
	artifacts: Option<&crate::output::ArtifactStore>,
	state: &SessionState,
) -> Result<String> {
	use crate::output::ToolOutput;
	let conflicts = &state.conflicts;
	let name = call.function.name.as_str();
	let arguments = call.function.arguments.as_str();
	let mut reading_artifact = false;
	let output = match name {
		"read" => {
			let args = parse::<crate::read::Args>(arguments)?;
			reading_artifact = args.is_artifact();
			crate::read::read(root, &args, store, artifacts, conflicts, cancellation)?
		},
		"write" => {
			let args = parse::<WriteArgs>(arguments)?;
			match resolution_device(root, &args.path, &args.content, store, state) {
				Some(result) => ToolOutput::text(result?),
				None => ToolOutput::text(write(root, &args, store, conflicts)?),
			}
		},
		"think" => ToolOutput::text("------"),
		"web_search" => ToolOutput::text(crate::web_search::web_search(&parse::<
			crate::web_search::Args,
		>(arguments)?)),
		"checkpoint" => {
			let args = parse::<CheckpointArgs>(arguments)?;
			let mut checkpoint = state.checkpoint();
			if checkpoint.active {
				bail!("Checkpoint already active.");
			}
			checkpoint.active = true;
			ToolOutput::text(format!(
				"Checkpoint: {}\nFinish exploration and formulate findings.",
				args.goal
			))
		},
		"rewind" => {
			let args = parse::<RewindArgs>(arguments)?;
			let mut checkpoint = state.checkpoint();
			if !checkpoint.active {
				if checkpoint.completed {
					bail!(
						"Checkpoint already completed; continue from the retained rewind report instead \
						 of calling rewind again."
					);
				}
				bail!("No active checkpoint. Create a checkpoint before calling rewind.");
			}
			let report = args.report.trim();
			if report.is_empty() {
				bail!("Report cannot be empty.");
			}
			checkpoint.pending_report = Some(report.to_owned());
			ToolOutput::text("Rewind requested.\nReport captured for context replacement.")
		},
		"ast_edit" => {
			let (text, plan) = crate::ast::ast_edit(
				root,
				&parse::<crate::ast::EditArgs>(arguments)?,
				store,
				cancellation,
			)?;
			if let Some(plan) = plan {
				state.pending().push(plan);
			}
			ToolOutput::text(text)
		},
		"edit" => ToolOutput::text(crate::edit::hashline(root, arguments, store, cancellation)?),
		"grep" => {
			crate::search::grep(root, &parse::<crate::search::Args>(arguments)?, store, cancellation)?
		},
		"glob" => ToolOutput::text(crate::glob::glob(
			root,
			&parse::<crate::glob::Args>(arguments)?,
			cancellation,
		)?),
		"bash" => ToolOutput::text(bash(root, &parse::<BashArgs>(arguments)?, cancellation)?),
		"todo" => ToolOutput::text(crate::todo::todo(&state.todos, arguments)),
		"ast_grep" => ToolOutput::text(crate::ast::ast_grep(
			root,
			&parse::<crate::ast::Args>(arguments)?,
			store,
			cancellation,
		)?),
		_ => bail!("unknown tool {name}"),
	};
	Ok(match artifacts {
		Some(artifacts) => crate::output::spill(output, name, artifacts, reading_artifact).render(),
		// Without a session there is nowhere to spill to: keep the hard cap.
		None => bounded(&output.render()),
	})
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
	path:    String,
	/// `content` in the legacy schema; `contents` is still accepted.
	#[serde(alias = "contents")]
	content: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(not(feature = "full"), allow(dead_code, reason = "used by the full shell"))]
struct BashArgs {
	command: String,
	env:     Option<std::collections::HashMap<String, String>>,
	timeout: Option<f64>,
	cwd:     Option<String>,
}

/// `TOOL_TIMEOUTS.bash`.
#[cfg(feature = "full")]
const BASH_TIMEOUT: (f64, f64, f64) = (300.0, 1.0, 3600.0);

/// `clampTimeout("bash", …)` (no `tools.maxTimeout` ceiling); `None` when the
/// caller passed `0` to disable the deadline. Returns the clamp notice too.
#[cfg(feature = "full")]
fn bash_timeout(requested: Option<f64>) -> (Option<f64>, Option<String>) {
	let (default, min, max) = BASH_TIMEOUT;
	let requested = requested.unwrap_or(default);
	if requested == 0.0 {
		return (None, None);
	}
	let effective = requested.clamp(min, max);
	let notice = (effective != requested).then(|| {
		format!(
			"Timeout clamped to {effective}s (requested {requested}s; allowed range {min}-{max}s)."
		)
	});
	(Some(effective), notice)
}

/// Validate env names and the working directory before running anything.
#[cfg(feature = "full")]
fn bash_cwd(root: &Path, args: &BashArgs) -> Result<std::path::PathBuf> {
	for key in args.env.iter().flat_map(|env| env.keys()) {
		let mut chars = key.chars();
		let valid = chars
			.next()
			.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
			&& chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
		if !valid {
			bail!("Invalid bash env name: {key}");
		}
	}
	let fs = harvest_runtime::JailedFs::open(root)?;
	let relative = fs.relative(Path::new(args.cwd.as_deref().unwrap_or(".")))?;
	let absolute = fs.root().join(&relative);
	match std::fs::metadata(&absolute) {
		Err(_) => bail!("Working directory does not exist: {}", absolute.display()),
		Ok(metadata) if !metadata.is_dir() => {
			bail!("Working directory is not a directory: {}", absolute.display())
		},
		Ok(_) => {},
	}
	let canonical = std::fs::canonicalize(&absolute)?;
	if !canonical.starts_with(fs.root()) {
		bail!("path escapes the authorized root");
	}
	Ok(canonical)
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T> {
	serde_json::from_str(arguments).context("tool arguments are not valid JSON")
}

fn write(
	root: &Path,
	args: &WriteArgs,
	store: &pi_edit::EditStore,
	conflicts: &crate::conflict::ConflictHistory,
) -> Result<String> {
	// A pasted `[path#TAG]` read header names the same target.
	let path = pi_edit::path_policy::unwrap_hashline_header_path(&args.path);
	let Some(uri) = crate::conflict::parse_uri(path)? else {
		return write_file(root, path, &args.content, store);
	};
	if let (Some(id), Some(scope)) = (uri.id, uri.scope) {
		let scope = scope.name();
		bail!(
			"Conflict URI scope '/{scope}' is read-only — read `conflict://{id}/{scope}` to inspect \
			 that side. To write, drop the scope (`conflict://{id}`) and put the chosen content (or \
			 shorthand like `@{scope}`) in `content`."
		);
	}
	let fs = JailedFs::open(root)?;
	let (clean, stripped) = crate::conflict::strip_write_content(&args.content);
	let resolver = crate::conflict::Resolver { fs: &fs, history: conflicts, store };
	let mut result = match uri.id {
		None => resolver.resolve_all(&clean, stripped, &args.content)?,
		Some(id) => resolver.resolve_one(id, &clean, stripped)?,
	};
	if let Some(prefix) = uri.recovered_prefix {
		let id = uri.id.map_or_else(|| "*".to_owned(), |id| id.to_string());
		let _ = std::fmt::Write::write_fmt(
			&mut result,
			format_args!(
				"\nNote: stripped erroneous '{prefix}:' prefix from path; conflict URIs are global \
				 (use `conflict://{id}`, not `<file>:conflict://{id}`)."
			),
		);
	}
	Ok(result)
}

const EXECUTABLE_NOTICE: &str = "[Notice: Made executable via chmod +x]";

/// Plain filesystem `write` (the legacy local-file path): fail closed on
/// mis-dispatched reads, refuse auto-generated files, strip copied hashline
/// prefixes, persist through the workspace jail, mark `#!` scripts
/// executable, and hand back a fresh snapshot header.
fn write_file(
	root: &Path,
	path: &str,
	content: &str,
	store: &pi_edit::EditStore,
) -> Result<String> {
	let trimmed = path.trim();
	if let Some((scheme, _)) = trimmed.split_once("://")
		&& !scheme.is_empty()
		&& scheme
			.chars()
			.next()
			.is_some_and(|ch| ch.is_ascii_alphabetic())
		&& scheme
			.chars()
			.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
	{
		bail!(
			"Unknown URI-like write target '{trimmed}'. This host only supports filesystem paths and \
			 conflict:// targets. Prefix the path with './' to write it as a filesystem path."
		);
	}
	let fs = JailedFs::open(root)?;
	let (clean, stripped) = crate::conflict::strip_write_content(content);
	let note = |mut text: String| {
		if stripped {
			text.push_str(
				"\nNote: auto-stripped hashline display prefixes from content before writing.",
			);
		}
		text
	};
	if let Some(result) = write_archive_member(&fs, path, &clean)? {
		return Ok(note(result));
	}
	if let Some(result) = crate::sqlite::write_sqlite(&fs, path, &clean)? {
		return Ok(note(result));
	}
	let relative = fs.relative(Path::new(path))?;
	let literal_missing = || !fs.exists(&relative).unwrap_or(true);
	// A `;`-joined list of read selectors is never a real write target.
	let segments: Vec<&str> = path.split(';').collect();
	if segments.len() >= 2
		&& segments.iter().all(|segment| {
			let segment = segment.trim();
			!segment.is_empty() && crate::read::split_path(segment).1.is_some()
		}) && literal_missing()
	{
		bail!(
			"write target '{path}' is a semicolon-joined list of {} read-tool selectors, not a \
			 filesystem path — refusing to create it. write creates a single file; issue one read() \
			 per path to read these ranges (e.g. read({{ path: \"<one path>:<range>\" }})).",
			segments.len()
		);
	}
	if content.is_empty()
		&& let (_, Some(selector)) = crate::read::split_path(path)
		&& literal_missing()
	{
		bail!(
			"write target '{path}' ends with a read-tool selector ':{selector}' and no such file \
			 exists — refusing to create a literal file by that name. If you meant to read it, use \
			 read({{ path: \"{path}\" }}). If you truly intend to create this file, pass its \
			 contents in `content` (a non-empty write is never blocked)."
		);
	}
	if let Ok(Some(existing)) = fs.read_optional(&relative, MAX_FILE_BYTES * 4) {
		let policy = crate::edit::policy(fs.root());
		if let Some(message) =
			policy.auto_generated_message(path, &existing[..existing.len().min(1024)])
		{
			bail!("{message}");
		}
	}
	if clean.len() > MAX_FILE_BYTES {
		bail!("write exceeds file limit");
	}
	fs.atomic_write(&relative, clean.as_bytes())?;
	let absolute = fs.root().join(&relative);
	let made_executable = clean.starts_with("#!") && mark_executable(&absolute);
	let display = relative.display().to_string();
	let key = pi_edit::path_policy::canonical_key(&absolute);
	store.invalidate(&key);
	let tag = store.record(&key, &pi_edit::text::normalize_to_lf(&clean), Some(&[]));
	let mut result = format!(
		"{}\nSuccessfully wrote {} bytes to {display}",
		pi_edit::modes::hashline::format::format_hashline_header(&display, &tag),
		clean.encode_utf16().count()
	);
	if stripped {
		result
			.push_str("\nNote: auto-stripped hashline display prefixes from content before writing.");
	}
	if made_executable {
		result.push('\n');
		result.push_str(EXECUTABLE_NOTICE);
	}
	Ok(result)
}

/// `#resolveArchiveWritePath` + `#writeArchiveEntry`: rewrite the whole
/// archive with one member set, through the workspace capability.
fn write_archive_member(fs: &JailedFs, path: &str, content: &str) -> Result<Option<String>> {
	let candidates: Vec<(String, String)> = harvest_archive::path_candidates(path)
		.into_iter()
		.filter(|(archive, _)| archive != path)
		.collect();
	let Some(fallback) = candidates.last().cloned() else {
		return Ok(None);
	};
	let member = |raw: &str| -> Result<String> {
		let unified = raw.replace('\\', "/");
		if unified.is_empty() {
			bail!("Archive write path must target a file inside the archive");
		}
		if unified.ends_with('/') {
			bail!("Archive write path must target a file, not a directory");
		}
		let mut parts = Vec::new();
		for part in unified.split('/') {
			match part {
				"" | "." => {},
				".." => bail!("Archive path cannot contain '..'"),
				part => parts.push(part),
			}
		}
		if parts.is_empty() {
			bail!("Archive write path must target a file inside the archive");
		}
		Ok(parts.join("/"))
	};
	let mut target = (fallback.0.clone(), member(&fallback.1)?, false);
	fs.relative(Path::new(&fallback.0))?;
	for (archive, sub) in &candidates {
		let relative = fs.relative(Path::new(archive))?;
		match fs.file_type(&relative) {
			Ok(kind) if kind.is_dir() => {},
			Ok(_) => {
				target = (archive.clone(), member(sub)?, true);
				break;
			},
			Err(_) => {},
		}
	}
	let (archive_path, member, exists) = target;
	let relative = fs.relative(Path::new(&archive_path))?;
	let format = harvest_archive::format_from_path(&archive_path).unwrap_or("tar");
	if !harvest_archive::is_writable(format) {
		bail!("Writing entries inside {format} archives is not supported (read-only format).");
	}
	let mut members: Vec<(String, Vec<u8>)> = if exists {
		let bytes = fs.read(&relative, 256 * 1024 * 1024)?;
		harvest_archive::Archive::from_bytes(format, bytes, &archive_path, Default::default())
			.and_then(|archive| archive.files())
			.map_err(|error| anyhow::anyhow!("{error}"))?
	} else {
		Vec::new()
	};
	let write_target = format!("{archive_path}:{member}");
	if content.is_empty()
		&& let (_, Some(selector)) = crate::read::split_path(&write_target)
		&& !members.iter().any(|(name, _)| *name == member)
	{
		bail!(
			"write target '{write_target}' ends with a read-tool selector ':{selector}' and no such \
			 file exists — refusing to create a literal file by that name. If you meant to read it, \
			 use read({{ path: \"{write_target}\" }}). If you truly intend to create this file, pass \
			 its contents in `content` (a non-empty write is never blocked)."
		);
	}
	match members.iter_mut().find(|(name, _)| *name == member) {
		Some((_, bytes)) => *bytes = content.as_bytes().to_vec(),
		None => members.push((member.clone(), content.as_bytes().to_vec())),
	}
	let encoded =
		harvest_archive::encode(format, &members).map_err(|error| anyhow::anyhow!("{error}"))?;
	fs.atomic_write(&relative, &encoded)?;
	Ok(Some(format!(
		"Successfully wrote {} bytes to {}:{member}",
		content.encode_utf16().count(),
		relative.display()
	)))
}

/// `chmod a+x`; failures never fail the write. True when the mode changed.
#[cfg(unix)]
fn mark_executable(path: &Path) -> bool {
	use std::os::unix::fs::PermissionsExt;
	let Ok(metadata) = std::fs::metadata(path) else {
		return false;
	};
	let mode = metadata.permissions().mode() & 0o7777;
	let next = mode | 0o111;
	next != mode && std::fs::set_permissions(path, std::fs::Permissions::from_mode(next)).is_ok()
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> bool {
	false
}

#[cfg(not(feature = "full"))]
fn bash(
	_root: &Path,
	args: &BashArgs,
	_cancellation: &harvest_runtime::cancel::CancelToken,
) -> Result<String> {
	if args.command.is_empty() {
		bail!("command is empty");
	}
	bail!("shell execution requires the full Rust profile");
}

#[cfg(feature = "full")]
fn bash(
	root: &Path,
	args: &BashArgs,
	cancellation: &harvest_runtime::cancel::CancelToken,
) -> Result<String> {
	if args.command.is_empty() {
		bail!("command is empty");
	}
	let cwd = bash_cwd(root, args)?;
	let (timeout, clamp_notice) = bash_timeout(args.timeout);
	let timeout_ms = timeout.map(|seconds| (seconds * 1000.0) as u32);
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?;
	let started = std::time::Instant::now();
	runtime.block_on(async {
		let (sender, receiver) = flume::bounded::<String>(16);
		let drain = tokio::spawn(async move {
			let mut captured = String::new();
			while let Ok(chunk) = receiver.recv_async().await {
				let remaining = harvest_runtime::MAX_TOOL_BYTES.saturating_sub(captured.len());
				captured.push_str(harvest_runtime::truncate_bytes(&chunk, remaining));
			}
			captured
		});
		let mut execution_token = harvest_runtime::cancel::CancelToken::new(timeout_ms);
		let abort = execution_token.emplace_abort_token();
		let parent = cancellation.clone();
		let bridge = tokio::spawn(async move {
			abort.abort(parent.wait().await);
		});
		let result = pi_shell::execute_shell(
			pi_shell::ShellExecuteOptions {
				command: args.command.clone(),
				cwd: Some(cwd.to_string_lossy().into_owned()),
				env: args.env.clone(),
				timeout_ms,
				..Default::default()
			},
			Some(sender),
			execution_token,
		)
		.await;
		bridge.abort();
		let output = drain.await?;
		let result = result?;
		cancellation.heartbeat()?;
		if result.timed_out {
			let message = timeout.map_or_else(
				|| "Command timed out".to_owned(),
				|seconds| format!("Command timed out after {seconds} seconds"),
			);
			if output.starts_with(&format!("[{message}]\n")) {
				return Ok(output);
			}
			let body = if output.is_empty() {
				"(no output)"
			} else {
				output.as_str()
			};
			return Ok(format!("{body}\n\n[{message}]"));
		}
		if result.cancelled {
			if output.starts_with("[Command cancelled]") {
				bail!("{output}");
			}
			if output.is_empty() {
				bail!("Command aborted");
			}
			bail!("{output}\n\n[Command aborted]");
		}
		let body = if output.is_empty() {
			"(no output)"
		} else {
			output.as_str()
		};
		let mut lines = vec![body.to_owned(), String::new()];
		lines.push(format!("Wall time: {:.2} seconds", started.elapsed().as_secs_f64()));
		lines.extend(clamp_notice);
		match result.exit_code {
			None => bail!("{}\n\nCommand failed: missing exit status", lines.join("\n")),
			Some(0) => {},
			Some(code) => lines.extend([String::new(), format!("Command exited with code {code}")]),
		}
		Ok(lines.join("\n"))
	})
}

fn bounded(text: &str) -> String {
	let mut out = String::new();
	for (index, character) in text.chars().enumerate() {
		if index >= OUTPUT_LIMIT {
			out.push_str("\ntruncated");
			break;
		}
		out.push(character);
	}
	out
}

#[cfg(test)]
mod tests {
	use super::invoke_approved as invoke;
	use crate::chat::{FunctionCall, ToolCall};

	fn call(name: &str, arguments: &str) -> ToolCall {
		ToolCall {
			id:       "1".to_owned(),
			kind:     "function".to_owned(),
			function: FunctionCall { name: name.to_owned(), arguments: arguments.to_owned() },
		}
	}

	#[cfg(all(unix, feature = "full"))]
	#[test]
	fn parent_cancellation_stops_an_approved_long_running_shell() {
		struct Allow;
		impl super::Approval for Allow {
			fn approve(&self, _: &ToolCall) -> anyhow::Result<bool> {
				Ok(true)
			}
		}
		let root = tempfile::tempdir().unwrap();
		let mut cancellation = harvest_runtime::cancel::CancelToken::default();
		let abort = cancellation.emplace_abort_token();
		let signal = std::thread::spawn(move || {
			std::thread::sleep(std::time::Duration::from_millis(100));
			abort.abort(harvest_runtime::cancel::AbortReason::User);
		});
		let started = std::time::Instant::now();
		let result = super::invoke_with_cancellation(
			root.path(),
			&call("bash", r#"{"command":"sleep 60"}"#),
			&Allow,
			&cancellation,
		);
		signal.join().unwrap();
		assert!(result.is_err());
		assert!(started.elapsed() < std::time::Duration::from_secs(5));
	}

	#[test]
	fn unapproved_file_and_shell_mutations_cannot_create_a_file() {
		let root = tempfile::tempdir().unwrap();
		for request in [
			call("write", r#"{"path":"blocked.txt","contents":"x"}"#),
			call("bash", r#"{"command":"echo blocked > blocked.txt"}"#),
		] {
			let error = super::invoke(root.path(), &request).unwrap_err();
			assert!(error.to_string().contains("requires human approval"));
			assert!(!root.path().join("blocked.txt").exists());
		}
	}

	fn tag_of(read: &str) -> String {
		read
			.lines()
			.next()
			.and_then(|header| header.split('#').nth(1))
			.unwrap()
			.trim_end_matches(']')
			.to_owned()
	}

	#[test]
	fn write_read_hashline_edit_and_grep_round_trip() {
		let root = tempfile::tempdir().unwrap();
		let store = pi_edit::EditStore::new();
		let run = |name: &str, arguments: &str| {
			super::invoke_authorized(
				root.path(),
				&call(name, arguments),
				&Default::default(),
				&store,
				None,
				&super::SessionState::default(),
			)
		};
		assert!(
			run("write", r#"{"path":"src/a.txt","contents":"alpha beta\n"}"#)
				.unwrap()
				.contains("wrote")
		);
		let read = run("read", r#"{"path":"src/a.txt"}"#).unwrap();
		let tag = tag_of(&read);
		assert_eq!(read, format!("[src/a.txt#{tag}]\n1:alpha beta"));
		let input = format!("[src/a.txt#{tag}]\nPUT 1.=1:\n+alpha gamma\n");
		let edited = run("edit", &serde_json::json!({ "input": input }).to_string()).unwrap();
		assert!(edited.starts_with("[src/a.txt#"), "{edited}");
		assert_eq!(std::fs::read_to_string(root.path().join("src/a.txt")).unwrap(), "alpha gamma\n");
		let hits = run("grep", r#"{"pattern":"gamma","path":"src"}"#).unwrap();
		assert!(hits.contains("alpha gamma"));
	}

	#[test]
	fn stale_tags_and_escaping_paths_are_rejected_without_writes() {
		let root = tempfile::tempdir().unwrap();
		let store = pi_edit::EditStore::new();
		std::fs::write(root.path().join("a.txt"), "one\n").unwrap();
		let input = "[a.txt#0000]\nPUT 1.=1:\n+two\n";
		let error = super::invoke_authorized(
			root.path(),
			&call("edit", &serde_json::json!({ "input": input }).to_string()),
			&Default::default(),
			&store,
			None,
			&super::SessionState::default(),
		)
		.unwrap_err();
		assert!(!error.to_string().is_empty());
		assert_eq!(std::fs::read_to_string(root.path().join("a.txt")).unwrap(), "one\n");
		let escaped = invoke(root.path(), &call("read", r#"{"path":"../secret"}"#)).unwrap_err();
		assert!(escaped.to_string().contains("escapes"));
	}

	#[test]
	fn session_results_over_50_kb_spill_to_a_readable_artifact() {
		let root = tempfile::tempdir().unwrap();
		let rows = (1..=3000)
			.map(|n| format!("{n:0>60}\n"))
			.collect::<Vec<_>>()
			.concat();
		std::fs::write(root.path().join("big.txt"), &rows).unwrap();
		let artifacts =
			crate::output::ArtifactStore::for_session(&root.path().join("s.jsonl")).unwrap();
		let store = pi_edit::EditStore::new();
		let run = |path: &str| {
			super::invoke_authorized(
				root.path(),
				&call("read", &serde_json::json!({ "path": path }).to_string()),
				&Default::default(),
				&store,
				Some(&artifacts),
				&super::SessionState::default(),
			)
			.unwrap()
		};
		let spilled = run("big.txt:1-3000");
		assert!(spilled.contains("ln elided…]"), "{}", &spilled[spilled.len() - 300..]);
		assert!(
			spilled.ends_with("Read artifact://0 for full output]"),
			"{}",
			&spilled[spilled.len() - 300..]
		);
		let page = run("artifact://0:3-4");
		// Artifact line 1 is the read's `[big.txt#TAG]` header; `:3-4` shows lines 2-7.
		assert!(page.starts_with("1:0") && page.contains("\n2:0"), "{page}");
		assert!(page.ends_with("Use :8 to continue]"), "{page}");
	}

	#[cfg(unix)]
	#[test]
	fn grep_skips_symlinked_directories() {
		let root = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret.txt"), "outside-secret").unwrap();
		std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
		std::fs::write(root.path().join("local.txt"), "outside-secret").unwrap();
		let hits = invoke(root.path(), &call("grep", r#"{"pattern":"outside-secret"}"#)).unwrap();
		assert!(hits.contains("local.txt"), "{hits}");
		assert!(!hits.contains("secret.txt"), "{hits}");
	}

	#[cfg_attr(miri, ignore)]
	#[test]
	// Keep this active for default builds even if their full feature is lost.
	#[cfg(any(feature = "full", feature = "default"))]
	#[cfg(feature = "full")]
	fn bash_reports_failures_clamps_and_rejects_bad_cwd_or_env() {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("file"), "").unwrap();
		let run = |args: &str| invoke(root.path(), &call("bash", args));
		assert!(
			run(r#"{"command":"true"}"#)
				.unwrap()
				.starts_with("(no output)\n\nWall time: ")
		);
		let failed = run(r#"{"command":"echo out; exit 3"}"#).unwrap();
		assert!(failed.starts_with("out\n\n\nWall time: "), "{failed}");
		assert!(failed.ends_with(" seconds\n\nCommand exited with code 3"), "{failed}");
		let clamped = run(r#"{"command":"echo hi","timeout":99999}"#).unwrap();
		assert!(
			clamped.contains("Timeout clamped to 3600s (requested 99999s; allowed range 1-3600s)."),
			"{clamped}"
		);
		let env = run(r#"{"command":"echo $GREETING","env":{"GREETING":"hey"}}"#).unwrap();
		assert!(env.starts_with("hey\n"), "{env}");
		assert!(
			run(r#"{"command":"x","env":{"1BAD":"v"}}"#)
				.unwrap_err()
				.to_string()
				.contains("Invalid bash env name: 1BAD")
		);
		assert!(
			run(r#"{"command":"x","cwd":"file"}"#)
				.unwrap_err()
				.to_string()
				.contains("is not a directory")
		);
		assert!(run(r#"{"command":"x","cwd":"../.."}"#).is_err());
		let timed = run(r#"{"command":"sleep 5","timeout":1}"#).unwrap();
		assert!(timed.contains("Command timed out after 1 seconds"), "{timed}");
	}

	#[test]
	fn archive_members_are_written_by_rewriting_the_archive() {
		let root = tempfile::tempdir().unwrap();
		let run = |args: &str| invoke(root.path(), &call("write", args));
		assert_eq!(
			run(r#"{"path":"pkg.tar.gz:src/a.txt","content":"one"}"#).unwrap(),
			"Successfully wrote 3 bytes to pkg.tar.gz:src/a.txt"
		);
		run(r#"{"path":"pkg.tar.gz:b.txt","content":"two"}"#).unwrap();
		let read = |path: &str| {
			invoke(root.path(), &call("read", &format!(r#"{{"path":"{path}"}}"#))).unwrap()
		};
		assert_eq!(read("pkg.tar.gz"), "b.txt (3B)\nsrc/");
		assert_eq!(read("pkg.tar.gz:src/a.txt"), "one");
		assert!(
			run(r#"{"path":"x.rar:a","content":"z"}"#)
				.unwrap_err()
				.to_string()
				.contains("read-only format")
		);
		assert!(
			run(r#"{"path":"pkg.zip:../a","content":"z"}"#)
				.unwrap_err()
				.to_string()
				.contains("'..'")
		);
	}

	#[test]
	fn staged_ast_edits_resolve_or_reject_through_xd_devices() {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("a.ts"), "foo(1);\n").unwrap();
		let (store, state) = (pi_edit::EditStore::new(), super::SessionState::default());
		let run = |name: &str, args: &str| {
			super::invoke_authorized(
				root.path(),
				&call(name, args),
				&Default::default(),
				&store,
				None,
				&state,
			)
		};
		let edit = r#"{"ops":[{"pat":"foo($A)","out":"bar($A)"}],"paths":["a.ts"]}"#;
		assert!(
			run("write", r#"{"path":"xd://resolve","content":"go"}"#)
				.unwrap_err()
				.to_string()
				.starts_with("No pending action")
		);
		run("ast_edit", edit).unwrap();
		assert_eq!(
			run("write", r#"{"path":"xd://reject","content":"nope"}"#).unwrap(),
			"Discarded: AST Edit: 1 replacement in 1 file. Reason: nope"
		);
		assert_eq!(std::fs::read_to_string(root.path().join("a.ts")).unwrap(), "foo(1);\n");
		run("ast_edit", edit).unwrap();
		assert!(
			run("write", r#"{"path":"xd://resolve","content":"ok"}"#)
				.unwrap()
				.starts_with("Applied 1 replacement in 1 file.")
		);
		assert_eq!(std::fs::read_to_string(root.path().join("a.ts")).unwrap(), "bar(1);\n");
	}

	#[test]
	fn bash_evaluates_commands_in_default_and_full_builds() {
		let root = tempfile::tempdir().unwrap();
		let output = invoke(root.path(), &call("bash", r#"{"command":"echo $((6 * 7))"}"#)).unwrap();
		assert!(output.lines().any(|line| line.trim() == "42"), "{output}");
	}

	#[cfg(not(feature = "full"))]
	#[test]
	fn lightweight_core_rejects_shell_execution_even_after_host_approval() {
		let root = tempfile::tempdir().unwrap();
		let error = invoke(root.path(), &call("bash", r#"{"command":"echo blocked > blocked.txt"}"#))
			.unwrap_err();
		assert!(error.to_string().contains("requires the full Rust profile"));
		assert!(!root.path().join("blocked.txt").exists());
	}

	#[test]
	fn unknown_tool_and_bad_json_fail() {
		let root = tempfile::tempdir().unwrap();
		let unknown = invoke(root.path(), &call("nope", "{}")).unwrap_err();
		assert!(unknown.to_string().contains("unknown"), "{unknown}");
		let invalid = invoke(root.path(), &call("read", "not-json")).unwrap_err();
		assert!(invalid.to_string().contains("JSON"), "{invalid}");
	}
}
