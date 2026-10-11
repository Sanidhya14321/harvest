use std::{
	io::{self, IsTerminal, Write},
	path::Path,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, truncate_bytes};
use regex::Regex;
use serde::Deserialize;

use crate::chat::ToolCall;

const OUTPUT_LIMIT: usize = 20_000;
const MATCH_LIMIT: usize = 50;

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
	cancellation.heartbeat()?;
	let name = call.function.name.as_str();
	if call.function.arguments.len() > harvest_runtime::MAX_EVENT_BYTES {
		bail!("tool arguments exceed frame limit");
	}
	if !matches!(name, "read" | "write" | "edit" | "grep" | "bash") {
		bail!("unknown tool {name}");
	}
	if matches!(name, "write" | "edit" | "bash") && !policy.approve(call)? {
		bail!("tool {name} requires human approval");
	}
	invoke_authorized(root, call, cancellation)
}

#[cfg(test)]
fn invoke_approved(root: &Path, call: &ToolCall) -> Result<String> {
	invoke_authorized(root, call, &Default::default())
}

fn invoke_authorized(
	root: &Path,
	call: &ToolCall,
	cancellation: &harvest_runtime::cancel::CancelToken,
) -> Result<String> {
	let name = call.function.name.as_str();
	let arguments = call.function.arguments.as_str();
	let output = match name {
		"read" => read_file(root, &parse::<ReadArgs>(arguments)?)?,
		"write" => write_file(root, &parse::<WriteArgs>(arguments)?)?,
		"edit" => edit_file(root, &parse::<EditArgs>(arguments)?)?,
		"grep" => grep(root, &parse::<GrepArgs>(arguments)?)?,
		"bash" => bash(root, &parse::<BashArgs>(arguments)?, cancellation)?,
		_ => bail!("unknown tool {name}"),
	};
	Ok(bounded(&output))
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
	path: String,
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
	path:     String,
	contents: String,
}

#[derive(Debug, Deserialize)]
struct EditArgs {
	path: String,
	old:  String,
	new:  String,
}

#[derive(Debug, Deserialize)]
struct GrepArgs {
	pattern: String,
	path:    Option<String>,
}

#[derive(Debug, Deserialize)]
struct BashArgs {
	command: String,
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T> {
	serde_json::from_str(arguments).context("tool arguments are not valid JSON")
}

fn read_file(root: &Path, args: &ReadArgs) -> Result<String> {
	let fs = JailedFs::open(root)?;
	let path = fs.relative(Path::new(&args.path))?;
	let bytes = fs.read(&path, MAX_FILE_BYTES)?;
	if bytes.contains(&0) {
		bail!("{} is not a text file", path.display());
	}
	String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", path.display()))
}

fn write_file(root: &Path, args: &WriteArgs) -> Result<String> {
	if args.contents.len() > MAX_FILE_BYTES {
		bail!("write exceeds file limit");
	}
	JailedFs::open(root)?.atomic_write(Path::new(&args.path), args.contents.as_bytes())?;
	Ok(format!("wrote {}", args.path))
}

fn edit_file(root: &Path, args: &EditArgs) -> Result<String> {
	if args.old.is_empty() {
		bail!("old text is empty");
	}
	let fs = JailedFs::open(root)?;
	let path = fs.relative(Path::new(&args.path))?;
	let text =
		String::from_utf8(fs.read(&path, MAX_FILE_BYTES)?).context("edit target is not UTF-8")?;
	let count = text.matches(&args.old).count();
	if count != 1 {
		bail!("expected one occurrence of the old text, found {count}");
	}
	let updated = text.replacen(&args.old, &args.new, 1);
	if updated.len() > MAX_FILE_BYTES {
		bail!("edit exceeds file limit");
	}
	fs.atomic_write(&path, updated.as_bytes())?;
	Ok(format!("edited {}", args.path))
}

fn grep(root: &Path, args: &GrepArgs) -> Result<String> {
	let pattern = Regex::new(&args.pattern).context("invalid regular expression")?;
	let fs = JailedFs::open(root)?;
	let start = fs.relative(Path::new(args.path.as_deref().unwrap_or(".")))?;
	let mut hits = Vec::new();
	let mut visited = 0;
	collect(&fs, &start, &pattern, &mut hits, &mut visited, 0)?;
	if hits.is_empty() {
		return Ok(String::new());
	}
	Ok(hits.join("\n"))
}

fn collect(
	fs: &JailedFs,
	path: &Path,
	pattern: &Regex,
	hits: &mut Vec<String>,
	visited: &mut usize,
	depth: usize,
) -> Result<()> {
	if hits.len() >= MATCH_LIMIT || *visited >= 10_000 || depth > 32 {
		return Ok(());
	}
	*visited += 1;
	if let Ok(entries) = fs.entries(path, 4096) {
		for entry in entries {
			if entry.is_symlink
				|| matches!(
					entry.name.as_str(),
					".git" | ".omp" | ".harvest" | "node_modules" | "target"
				) {
				continue;
			}
			collect(fs, &path.join(entry.name), pattern, hits, visited, depth + 1)?;
			if hits.len() >= MATCH_LIMIT {
				break;
			}
		}
		return Ok(());
	}
	let Ok(bytes) = fs.read(path, MAX_FILE_BYTES) else {
		return Ok(());
	};
	if bytes.contains(&0) {
		return Ok(());
	}
	let Ok(text) = String::from_utf8(bytes) else {
		return Ok(());
	};
	for (index, line) in text.lines().enumerate() {
		if pattern.is_match(line) {
			hits.push(format!("{}:{}:{}", path.display(), index + 1, truncate_bytes(line, 2000)));
			if hits.len() >= MATCH_LIMIT {
				break;
			}
		}
	}
	Ok(())
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
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?;
	runtime.block_on(async {
		let (sender, receiver) = flume::bounded::<String>(16);
		let drain = tokio::spawn(async move {
			let mut captured = String::new();
			while let Ok(chunk) = receiver.recv_async().await {
				let remaining = harvest_runtime::MAX_TOOL_BYTES.saturating_sub(captured.len());
				captured.push_str(truncate_bytes(&chunk, remaining));
			}
			captured
		});
		let mut execution_token = harvest_runtime::cancel::CancelToken::new(Some(60_000));
		let abort = execution_token.emplace_abort_token();
		let parent = cancellation.clone();
		let bridge = tokio::spawn(async move {
			abort.abort(parent.wait().await);
		});
		let result = pi_shell::execute_shell(
			pi_shell::ShellExecuteOptions {
				command: args.command.clone(),
				cwd: Some(root.to_string_lossy().into_owned()),
				timeout_ms: Some(60_000),
				..Default::default()
			},
			Some(sender),
			execution_token,
		)
		.await;
		bridge.abort();
		let text = drain.await?;
		let result = result?;
		cancellation.heartbeat()?;
		if result.cancelled || result.timed_out {
			bail!("shell execution cancelled or timed out");
		}
		Ok(format!("{text}\nexit {}", result.exit_code.unwrap_or(1)))
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
	fn unapproved_mutation_cannot_create_a_file() {
		let root = tempfile::tempdir().unwrap();
		assert!(
			super::invoke(root.path(), &call("write", r#"{"path":"blocked.txt","contents":"x"}"#))
				.is_err()
		);
		assert!(!root.path().join("blocked.txt").exists());
	}

	#[test]
	fn write_read_edit_and_grep_round_trip() {
		let root = tempfile::tempdir().unwrap();
		let wrote =
			invoke(root.path(), &call("write", r#"{"path":"src/a.txt","contents":"alpha beta"}"#))
				.unwrap();
		assert!(wrote.contains("wrote"));
		let read = invoke(root.path(), &call("read", r#"{"path":"src/a.txt"}"#)).unwrap();
		assert_eq!(read, "alpha beta");
		let edited =
			invoke(root.path(), &call("edit", r#"{"path":"src/a.txt","old":"beta","new":"gamma"}"#))
				.unwrap();
		assert!(edited.contains("edited"));
		let hits = invoke(root.path(), &call("grep", r#"{"pattern":"gamma","path":"src"}"#)).unwrap();
		assert!(hits.contains("alpha gamma"));
	}

	#[test]
	fn edit_requires_one_match_and_paths_cannot_escape() {
		let root = tempfile::tempdir().unwrap();
		let wrote =
			invoke(root.path(), &call("write", r#"{"path":"a.txt","contents":"one one"}"#)).unwrap();
		assert!(wrote.contains("wrote"), "{wrote}");
		let error = invoke(root.path(), &call("edit", r#"{"path":"a.txt","old":"one","new":"two"}"#))
			.unwrap_err();
		assert!(error.to_string().contains("one occurrence"));
		let escaped = invoke(root.path(), &call("read", r#"{"path":"../secret"}"#)).unwrap_err();
		assert!(escaped.to_string().contains("escapes"));
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
	#[cfg(feature = "full")]
	fn bash_runs_in_the_workspace() {
		let root = tempfile::tempdir().unwrap();
		let command = if cfg!(windows) {
			r#"{"command":"echo workspace"}"#
		} else {
			r#"{"command":"printf workspace"}"#
		};
		let output = invoke(root.path(), &call("bash", command)).unwrap();
		assert!(output.contains("workspace"));
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
