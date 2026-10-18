//! Model-context projection must respect durable rewrites without corrupting
//! branch history or producing invalid tool protocols.
use std::{path::Path, process::Command};

use anyhow::{Result, bail};
use harvest_omp::{durable_session::DurableSession, session_format::SessionDocument};
use serde::Deserialize;
use serde_json::{Value, json};

mod common;

#[derive(Deserialize)]
struct ContextCase {
	name:     String,
	leaf:     String,
	entries:  Vec<Value>,
	expected: Vec<Value>,
}

fn write_session(root: &Path, entries: &[Value]) -> Result<Vec<u8>> {
	let mut bytes = serde_json::to_vec(&json!({"type":"session","version":3,"id":"context"}))?;
	bytes.push(b'\n');
	for entry in entries {
		serde_json::to_writer(&mut bytes, entry)?;
		bytes.push(b'\n');
	}
	std::fs::write(root.join("session.jsonl"), &bytes)?;
	Ok(bytes)
}

fn message(id: &str, parent: Option<&str>, role: &str, content: Value) -> Value {
	json!({"type":"message","id":id,"parentId":parent,"message":{"role":role,"content":content}})
}

fn compaction(id: &str, parent: &str, first_kept: &str) -> Value {
	json!({"type":"compaction","id":id,"parentId":parent,"summary":"Preserved work","firstKeptEntryId":first_kept,"tokensBefore":200})
}

fn text(value: &str) -> Value {
	json!([{"type":"text","text":value}])
}

#[test]
fn selected_context_matches_legacy_rewrites_and_retains_the_original_log() -> Result<()> {
	let cases: Vec<ContextCase> =
		serde_json::from_str(include_str!("fixtures/session-context.json"))?;
	for case in cases {
		let root = tempfile::tempdir()?;
		let original = write_session(root.path(), &case.entries)?;
		let session =
			DurableSession::open(root.path(), Path::new("session.jsonl"), Some(&case.leaf))?;
		let actual = session
			.messages()
			.iter()
			.map(|message| json!({"role":message.role,"content":message.content}))
			.collect::<Vec<_>>();
		assert_eq!(actual, case.expected, "{}", case.name);
		assert_eq!(session.leaf(), Some(case.leaf.as_str()), "{}", case.name);
		assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, original);
	}
	Ok(())
}

#[test]
fn compaction_can_elide_opaque_history_but_must_keep_complete_tool_pairs() -> Result<()> {
	let root = tempfile::tempdir()?;
	let opaque =
		message("opaque", None, "assistant", json!([{"type":"thinking","signature":"secret"}]));
	let call = message(
		"call",
		Some("opaque"),
		"assistant",
		json!([{"type":"toolCall","id":"lookup","name":"read","arguments":{"path":"a"}}]),
	);
	let mut result = message("result", Some("call"), "toolResult", text("read result"));
	result["message"]["toolCallId"] = json!("lookup");
	let entries = [opaque, call, result, compaction("c", "result", "call")];
	write_session(root.path(), &entries)?;
	let session = DurableSession::open(root.path(), Path::new("session.jsonl"), None)?;
	assert_eq!(session.messages()[1].tool_calls.as_ref().unwrap()[0].id, "lookup");
	assert_eq!(session.messages()[2].tool_call_id.as_deref(), Some("lookup"));
	assert_eq!(session.messages()[2].content.as_deref(), Some("read result"));
	drop(session);
	// Cutting between the call and result must fail instead of stripping blocks.
	let mut invalid = entries.to_vec();
	invalid[3]["firstKeptEntryId"] = json!("result");
	let original = write_session(root.path(), &invalid)?;
	assert!(DurableSession::open(root.path(), Path::new("session.jsonl"), None).is_err());
	assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, original);
	// Keeping an unresolved call is also invalid, even when a summary follows.
	let original = write_session(root.path(), &[
		entries[0].clone(),
		entries[1].clone(),
		compaction("c", "call", "call"),
	])?;
	assert!(DurableSession::open(root.path(), Path::new("session.jsonl"), None).is_err());
	assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, original);
	Ok(())
}

#[test]
fn compaction_rejects_missing_forward_and_sibling_kept_references_without_writing() -> Result<()> {
	for kept in ["missing", "forward", "sibling"] {
		let root = tempfile::tempdir()?;
		let entries = [
			message("base", None, "user", text("base")),
			message("sibling", Some("base"), "user", text("sibling")),
			compaction("c", "base", kept),
			message("forward", Some("c"), "user", text("future")),
		];
		let original = write_session(root.path(), &entries)?;
		let Err(error) =
			DurableSession::open(root.path(), Path::new("session.jsonl"), Some("forward"))
		else {
			bail!("invalid kept reference {kept} was accepted");
		};
		assert!(error.to_string().contains("kept entry"), "{error:#}");
		assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, original);
	}
	Ok(())
}

#[test]
fn opaque_or_malformed_compaction_payloads_never_silently_become_text_summaries() -> Result<()> {
	for patch in [
		json!({"method":"remote"}),
		json!({"method":"snapcompact"}),
		json!({"method":"future"}),
		json!({"method":null}),
		json!({"preserveData":{"openaiRemoteCompaction":{"replacementHistory":[]}}}),
		json!({"preserveData":{"snapcompact":{"frames":[]}}}),
		json!({"preserveData":{"unknownExtension":{"memory":"opaque"}}}),
		json!({"preserveData":null}),
		json!({"providerReplayThroughEntryId":"base"}),
		json!({"providerPayload":{}}),
		json!({"summary":null}),
	] {
		let root = tempfile::tempdir()?;
		let mut rewrite = compaction("c", "base", "base");
		for (key, value) in patch.as_object().unwrap() {
			rewrite[key] = value.clone();
		}
		let original =
			write_session(root.path(), &[message("base", None, "user", text("base")), rewrite])?;
		assert!(
			DurableSession::open(root.path(), Path::new("session.jsonl"), None).is_err(),
			"accepted {patch}"
		);
		assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, original);
	}
	Ok(())
}

#[test]
fn summary_insertion_cannot_interrupt_a_pending_tool_protocol() -> Result<()> {
	let root = tempfile::tempdir()?;
	let call = message(
		"call",
		None,
		"assistant",
		json!([{"type":"toolCall","id":"lookup","name":"read","arguments":{"path":"a"}}]),
	);
	let summary = json!({"type":"branch_summary","id":"branch","parentId":"call","summary":"return from branch","fromId":"other"});
	let mut result = message("result", Some("branch"), "toolResult", text("read result"));
	result["message"]["toolCallId"] = json!("lookup");
	let original = write_session(root.path(), &[call, summary, result])?;
	assert!(DurableSession::open(root.path(), Path::new("session.jsonl"), None).is_err());
	assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, original);
	Ok(())
}

#[test]
fn cli_resumes_rewritten_context_and_appends_to_the_real_branch_leaf() -> Result<()> {
	let root = tempfile::tempdir()?;
	let entries = [
		message("old", None, "user", text("superseded secret")),
		message("kept", Some("old"), "user", text("recent request")),
		compaction("c", "kept", "kept"),
		message("leaf", Some("c"), "assistant", text("recent answer")),
		message("sibling", Some("old"), "user", text("another branch secret")),
	];
	let original = write_session(root.path(), &entries)?;
	let server = common::HttpFixture::spawn(200, "OK", "application/json", r#"{"choices":[{"message":{"role":"assistant","content":"continued"},"finish_reason":"stop"}]}"#.into())?;
	let output = Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["run", "--workspace"])
		.arg(root.path())
		.args([
			"--session",
			"session.jsonl",
			"--leaf",
			"leaf",
			"--prompt",
			"continue",
			"--model",
			"fixture",
			"--api-key",
			"",
			"--base-url",
		])
		.arg(server.base_url())
		.args(["--max-steps", "1"])
		.env_remove("OMP_MAX_STEPS")
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let request = server.finish()?;
	let sent = request["messages"].as_array().unwrap();
	assert_eq!(sent[2], json!({"role":"user","content":"recent request"}));
	assert_eq!(sent[3], json!({"role":"assistant","content":"recent answer"}));
	assert_eq!(sent[4], json!({"role":"user","content":"continue"}));
	assert!(!request.to_string().contains("superseded secret"));
	assert!(!request.to_string().contains("another branch secret"));
	let bytes = std::fs::read(root.path().join("session.jsonl"))?;
	assert!(bytes.starts_with(&original));
	let document = SessionDocument::decode(&bytes)?;
	let appended = &document.entries()[entries.len()..];
	assert_eq!(appended[0]["parentId"].get(), "\"leaf\"");
	let mut session = DurableSession::open(root.path(), Path::new("session.jsonl"), None)?;
	assert_eq!(session.messages().last().unwrap().content.as_deref(), Some("continued"));
	// A subsequent reopen must not resurrect the discarded prefix either.
	let next = harvest_omp::chat::user("next");
	session.commit(&next, "fixture")?;
	assert!(
		!session
			.messages()
			.iter()
			.any(|message| message.content.as_deref() == Some("superseded secret"))
	);
	Ok(())
}
