//! Session contracts: unseen rows, resume authority, final approval and shared
//! owners. Written during conversion; execution belongs to consolidated
//! stabilization.
use std::{
	path::Path,
	sync::{Arc, Mutex},
};

use anyhow::{Result, ensure};
use harvest_omp::{
	agent::RunOptions,
	agent_session::AgentSession,
	chat::{ChatMessage, FunctionCall, ToolCall, ToolSpec},
	configuration::{self, LoadOptions},
	edit::Mode,
	provider::Model,
	session_engine::{SessionEngine, SessionEngineOptions},
	tools::{Approval, DenyMutations, ToolRuntime},
};
use serde_json::{Value, json};

struct Allow;
impl Approval for Allow {
	fn approve(&self, call: &ToolCall) -> Result<bool> {
		if call.function.name == "edit" {
			let args: Value = serde_json::from_str(&call.function.arguments)?;
			ensure!(args["mode"] == "hashline", "host must approve selected mode");
			ensure!(args["arguments"]["input"].is_string(), "host must approve final patch");
		}
		Ok(true)
	}
}
fn call(name: &str, args: Value) -> ToolCall {
	ToolCall {
		id:       format!("{name}-fixture"),
		kind:     "function".into(),
		function: FunctionCall { name: name.into(), arguments: args.to_string() },
	}
}
fn invoke(
	runtime: &mut ToolRuntime,
	root: &Path,
	name: &str,
	args: Value,
) -> Result<harvest_omp::tools::ToolOutput> {
	runtime.invoke_detailed(root, &call(name, args), &Allow, &Default::default())
}
fn runtime() -> ToolRuntime {
	let mut runtime = ToolRuntime::default();
	runtime.configure_edit(Mode::Hashline);
	runtime
}
fn edit(
	runtime: &mut ToolRuntime,
	root: &Path,
	tag: &str,
	line: u32,
	text: &str,
) -> Result<harvest_omp::tools::ToolOutput> {
	invoke(
		runtime,
		root,
		"edit",
		json!({"input":format!("[a.txt#{tag}]\nPUT {line}.={line}:\n+{text}")}),
	)
}

#[test]
fn read_windows_grant_only_displayed_lines_and_state_is_per_session() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\ntwo\nthree\n")?;
	let mut owner = runtime();
	let shown =
		invoke(&mut owner, root.path(), "read", json!({"path":"a.txt","offset":2,"limit":1}))?;
	let details = shown.details.unwrap();
	let tag = details["tag"].as_str().unwrap();
	assert_eq!(details["shownLines"], json!([2]));
	assert!(shown.content.contains("2:two"));
	assert!(!shown.content.contains("1:one"));
	assert!(edit(&mut owner, root.path(), tag, 1, "hidden")?.is_error);
	assert!(edit(&mut runtime(), root.path(), tag, 2, "other owner")?.is_error);
	assert!(!edit(&mut owner, root.path(), tag, 2, "changed")?.is_error);
	assert_eq!(std::fs::read_to_string(root.path().join("a.txt"))?, "one\nchanged\nthree\n");
	Ok(())
}

#[test]
fn oversized_rows_and_forged_numbered_content_never_grant_hidden_provenance() -> Result<()> {
	let root = tempfile::tempdir()?;
	let source = format!("2:forged\n{}\nlast\n", "x".repeat(21_000));
	std::fs::write(root.path().join("a.txt"), &source)?;
	let mut owner = runtime();
	let shown = invoke(&mut owner, root.path(), "read", json!({"path":"a.txt"}))?;
	let details = shown.details.unwrap();
	assert_eq!(details["shownLines"], json!([1]));
	assert_eq!(details["nextOffset"], 2);
	let tag = details["tag"].as_str().unwrap();
	assert!(edit(&mut owner, root.path(), tag, 2, "hidden")?.is_error);
	assert_eq!(std::fs::read_to_string(root.path().join("a.txt"))?, source);
	Ok(())
}

#[test]
fn direct_write_and_mode_switch_revoke_old_snapshot_authority() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\n")?;
	let mut owner = runtime();
	let first = invoke(&mut owner, root.path(), "read", json!({"path":"a.txt"}))?;
	let details = first.details.unwrap();
	let tag = details["tag"].as_str().unwrap();
	invoke(&mut owner, root.path(), "write", json!({"path":"a.txt","contents":"one\n"}))?;
	assert!(edit(&mut owner, root.path(), tag, 1, "revoked")?.is_error);
	invoke(&mut owner, root.path(), "read", json!({"path":"a.txt"}))?;
	owner.configure_edit(Mode::Replace);
	owner.configure_edit(Mode::Hashline);
	assert!(edit(&mut owner, root.path(), tag, 1, "revoked again")?.is_error);
	let other = tempfile::tempdir()?;
	assert!(invoke(&mut owner, other.path(), "read", json!({"path":"a.txt"})).is_err());
	Ok(())
}

#[test]
fn host_denial_prevents_hashline_mutation_after_a_real_read() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\n")?;
	let mut owner = runtime();
	let read = invoke(&mut owner, root.path(), "read", json!({"path":"a.txt"}))?;
	let tag = read.details.unwrap()["tag"].as_str().unwrap().to_owned();
	let request = call("edit", json!({"input":format!("[a.txt#{tag}]\nPUT 1.=1:\n+changed")}));
	assert!(
		owner
			.invoke_detailed(root.path(), &request, &DenyMutations, &Default::default())
			.is_err()
	);
	assert_eq!(std::fs::read_to_string(root.path().join("a.txt"))?, "one\n");
	Ok(())
}

// Script derives the edit tag from the actual model-visible read result, then
// checks the next model request receives the completed edit, not host details.
#[derive(Default)]
struct ReadThenEdit {
	phase: Mutex<u8>,
}
impl Model for ReadThenEdit {
	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
		let edit_spec = tools.iter().find(|s| s.name() == "edit").unwrap();
		let spec = serde_json::to_value(edit_spec)?;
		ensure!(spec["function"]["parameters"]["required"] == json!(["input"]), "wrong edit schema");
		let mut phase = self.phase.lock().unwrap();
		let request = match *phase {
			0 => Some(call("read", json!({"path":"a.txt"}))),
			1 => {
				let read = messages.iter().rev().find(|m| m.role == "tool").unwrap();
				let header = read.content.as_deref().unwrap().lines().next().unwrap();
				ensure!(header.starts_with("[a.txt#"), "read must display snapshot header");
				Some(call("edit", json!({"input":format!("{header}\nPUT 1.=1:\n+changed")})))
			},
			_ => {
				ensure!(
					!messages
						.iter()
						.rev()
						.find(|m| m.role == "tool")
						.unwrap()
						.content
						.as_deref()
						.unwrap()
						.contains("error"),
					"edit was rejected"
				);
				None
			},
		};
		*phase += 1;
		Ok(ChatMessage {
			role:         "assistant".into(),
			content:      request.is_none().then(|| "done".into()),
			tool_calls:   request.map(|call| vec![call]),
			tool_call_id: None,
		})
	}
}
fn options() -> RunOptions {
	RunOptions {
		max_steps:      3,
		user_agent_dir: None,
		tags:           Default::default(),
		cancellation:   Default::default(),
	}
}

#[test]
fn synchronous_owner_persists_edit_details_but_resume_needs_a_fresh_read() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\n")?;
	let path = Path::new("session.jsonl");
	let mut session = AgentSession::open(root.path(), path, None)?;
	session.configure_edit(Mode::Hashline)?;
	session.submit("change a.txt", &ReadThenEdit::default(), &Allow, &options(), &mut |_| Ok(()))?;
	assert_eq!(std::fs::read_to_string(root.path().join("a.txt"))?, "changed\n");
	let messages: Vec<Value> = session
		.stored_messages()
		.iter()
		.map(|m| serde_json::from_str(m.get()).unwrap())
		.collect();
	assert!(
		messages
			.iter()
			.any(|m| m["details"]["editMode"] == "hashline" && m["details"]["diff"].is_string())
	);
	drop(session);
	let mut resumed = AgentSession::open(root.path(), path, None)?;
	resumed.configure_edit(Mode::Hashline)?;
	struct KnownTag {
		called: Mutex<bool>,
	}
	impl Model for KnownTag {
		fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ChatMessage> {
			let mut called = self.called.lock().unwrap();
			let request = (!*called).then(|| {
				call(
					"edit",
					json!({"input":format!(
                "[a.txt#{}]\nPUT 1.=1:\n+unauthorized", pi_edit::store::file_hash("changed\n"))}),
				)
			});
			*called = true;
			Ok(ChatMessage {
				role:         "assistant".into(),
				content:      request.is_none().then(|| "done".into()),
				tool_calls:   request.map(|call| vec![call]),
				tool_call_id: None,
			})
		}
	}
	resumed.submit(
		"reuse a known tag",
		&KnownTag { called: Mutex::new(false) },
		&Allow,
		&options(),
		&mut |_| Ok(()),
	)?;
	assert_eq!(std::fs::read_to_string(root.path().join("a.txt"))?, "changed\n");
	let last: Value = serde_json::from_str(
		resumed
			.stored_messages()
			.iter()
			.rev()
			.find(|m| serde_json::from_str::<Value>(m.get()).unwrap()["role"] == "toolResult")
			.unwrap()
			.get(),
	)?;
	assert_eq!(last["isError"], true);
	Ok(())
}

#[tokio::test]
async fn asynchronous_sdk_owner_uses_the_same_read_schema_and_edit_state() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\n")?;
	let owner = SessionEngine::open(
		root.path(),
		Path::new("async.jsonl"),
		None,
		Arc::new(ReadThenEdit::default()),
		Arc::new(Allow),
		SessionEngineOptions {
			edit_mode: Mode::Hashline,
			max_model_turns: Some(3),
			..Default::default()
		},
	)
	.await?;
	owner.submit("change a.txt".into()).await?.wait().await?;
	assert_eq!(std::fs::read_to_string(root.path().join("a.txt"))?, "changed\n");
	owner.shutdown().await?;
	Ok(())
}

#[test]
fn profile_edit_selection_is_overridden_by_host_and_invalid_modes_fail() -> Result<()> {
	assert_eq!(Mode::select(Some(Mode::Replace), Some("invalid"))?, Some(Mode::Replace));
	assert!(Mode::select(None, Some("invalid")).is_err());
	let root = tempfile::tempdir()?;
	std::fs::create_dir(root.path().join(".harvest"))?;
	std::fs::write(
		root.path().join(".harvest/config.toml"),
		"edit_mode = 'patch'\nprofile = 'lines'\n[profiles.lines]\nedit_mode = 'hashline'\n",
	)?;
	let resolved = configuration::load(
		root.path(),
		&Default::default(),
		&Default::default(),
		&LoadOptions::default(),
	)?;
	assert_eq!(resolved.edit_mode, Mode::Hashline);
	let resolved =
		configuration::load(root.path(), &Default::default(), &Default::default(), &LoadOptions {
			edit_mode: Some(Mode::Replace),
			..Default::default()
		})?;
	assert_eq!(resolved.edit_mode, Mode::Replace);
	std::fs::write(root.path().join(".harvest/config.toml"), "edit_mode = 'invented'\n")?;
	assert!(
		configuration::load(
			root.path(),
			&Default::default(),
			&Default::default(),
			&LoadOptions::default()
		)
		.is_err()
	);
	Ok(())
}

#[test]
fn disjoint_tail_and_raw_selectors_preserve_exact_read_authority() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\ntwo\nthree\nfour\nfive\n")?;
	let mut owner = runtime();
	let result = invoke(&mut owner, root.path(), "read", json!({"path":"a.txt:3-3,1+1"}))?;
	let details = result.details.unwrap();
	assert_eq!(details["shownLines"], json!([1, 3]));
	let tag = details["tag"].as_str().unwrap();
	assert!(edit(&mut owner, root.path(), tag, 2, "hidden gap")?.is_error);
	assert!(!edit(&mut owner, root.path(), tag, 3, "changed")?.is_error);
	let tail = invoke(&mut owner, root.path(), "read", json!({"path":"a.txt","selector":"-2"}))?;
	assert_eq!(tail.details.unwrap()["shownLines"], json!([4, 5]));
	assert!(tail.content.contains("4:four"));
	let mut raw_owner = runtime();
	let raw = invoke(&mut raw_owner, root.path(), "read", json!({"path":"a.txt:raw:1-2"}))?;
	assert_eq!(raw.content, "one\ntwo\n\n[Read continues at offset 3 of 5 lines.]");
	let details = raw.details.unwrap();
	assert_eq!(details["shownLines"], json!([]));
	assert!(
		edit(&mut raw_owner, root.path(), details["tag"].as_str().unwrap(), 1, "not seen")?.is_error
	);
	Ok(())
}

#[cfg(unix)]
#[test]
fn literal_colon_names_and_dangling_symlinks_never_fall_back_to_other_files() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "base file\n")?;
	std::fs::write(root.path().join("a.txt:1-1"), "literal file\n")?;
	let mut owner = runtime();
	let literal = invoke(&mut owner, root.path(), "read", json!({"path":"a.txt:1-1"}))?;
	assert!(literal.content.contains("1:literal file"));
	assert!(!literal.content.contains("base file"));
	std::os::unix::fs::symlink("absent-target", root.path().join("a.txt:raw"))?;
	assert!(invoke(&mut owner, root.path(), "read", json!({"path":"a.txt:raw"})).is_err());
	Ok(())
}

#[test]
fn invalid_or_conflicting_selectors_fail_instead_of_widening_the_read() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("a.txt"), "one\ntwo\nthree\n")?;
	let mut owner = runtime();
	for args in [
		json!({"path":"a.txt","selector":"0"}),
		json!({"path":"a.txt","selector":"2+0"}),
		json!({"path":"a.txt","selector":"3-1"}),
		json!({"path":"a.txt","selector":"1-2","offset":1}),
		json!({"path":"a.txt:1-2","selector":"3-3"}),
		json!({"path":"a.txt","selector":"raw:raw"}),
		json!({"path":"a.txt","selector":"conflicts"}),
		json!({"path":"a.txt","selector":"4294967295+2"}),
	] {
		assert!(invoke(&mut owner, root.path(), "read", args).is_err());
	}
	Ok(())
}
