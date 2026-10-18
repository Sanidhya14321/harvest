//! End-to-end transport, durable accounting and safe replay contracts.
use std::{
	path::Path,
	process::{Command, Output},
};

use anyhow::{Context, Result};
mod common;
use harvest_omp::session_format::{SessionDocument, SessionObject};
use serde_json::{Value, json};

fn run(
	root: &Path,
	body: String,
	content_type: &'static str,
	request_usage: bool,
) -> Result<(Output, Value)> {
	let server = common::HttpFixture::spawn(200, "OK", content_type, body)?;
	let mut command = Command::new(env!("CARGO_BIN_EXE_harvest"));
	command
		.args(["run", "--workspace"])
		.arg(root)
		.args(["--prompt", "Compute the answer", "--model", "fixture", "--base-url"])
		.arg(server.base_url())
		.args(["--api-key", "", "--max-steps", "1"])
		.env_remove("OMP_MAX_STEPS");
	if request_usage {
		command.arg("--request-usage");
	}
	let output = command.output()?;
	let request = server.finish()?;
	Ok((output, request))
}

fn assistant(root: &Path) -> Result<SessionObject> {
	let bytes = std::fs::read(root.join(".omp/session.jsonl"))?;
	let document = SessionDocument::decode(&bytes)?;
	let raw = document
		.entries()
		.last()
		.context("missing persisted entry")?
		.get("message")
		.context("missing persisted message")?;
	Ok(serde_json::from_str(raw.get())?)
}

fn stream(usage: &str, done: bool) -> String {
	format!(
		"data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"done\"}},\"finish_reason\":\"\
		 stop\"}}]}}\n\ndata: {{\"choices\":[],\"usage\":{usage}}}\n\n{}",
		if done { "data: [DONE]\n\n" } else { "" }
	)
}

#[test]
fn streamed_usage_is_durable_lossless_and_excluded_from_replayed_model_requests() -> Result<()> {
	let root = tempfile::tempdir()?;
	let raw = r#"{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":30,"cache_write_tokens":10},"completion_tokens_details":{"reasoning_tokens":7},"future":900719925474099312345,"unbounded_future_number":1e400}"#;
	let (output, request) =
		run(root.path(), stream(raw, true), "Text/Event-Stream; charset=utf-8", true)?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert_eq!(request["stream_options"], json!({"include_usage":true}));
	let message = assistant(root.path())?;
	assert_eq!(message["providerUsage"].get(), raw);
	assert_eq!(message["usageUnavailable"].get(), "false");
	assert_eq!(message["costUnavailable"].get(), "true");
	let usage: Value = serde_json::from_str(message["usage"].get())?;
	assert_eq!(
		(
			usage["input"].as_u64(),
			usage["output"].as_u64(),
			usage["cacheRead"].as_u64(),
			usage["cacheWrite"].as_u64(),
			usage["totalTokens"].as_u64(),
			usage["reasoningTokens"].as_u64()
		),
		(Some(60), Some(20), Some(30), Some(10), Some(120), Some(7))
	);
	let (output, request) = run(root.path(), r#"{"choices":[{"message":{"role":"assistant","content":"resumed"},"finish_reason":"stop"}]}"#.into(), "application/json", false)?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let replayed = request["messages"]
		.as_array()
		.unwrap()
		.iter()
		.find(|message| message["role"] == "assistant")
		.unwrap();
	assert_eq!(replayed, &json!({"role":"assistant","content":"done"}));
	assert!(request.get("stream_options").is_none());
	let message = assistant(root.path())?;
	assert_eq!(message["usageUnavailable"].get(), "true");
	assert!(!message.contains_key("providerUsage"));
	Ok(())
}

#[test]
fn json_fallback_preserves_hit_miss_accounting_and_reports_token_availability() -> Result<()> {
	let root = tempfile::tempdir()?;
	let body = r#"{"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_cache_hit_tokens":30,"prompt_cache_miss_tokens":70}}"#;
	let (output, request) = run(root.path(), body.into(), "application/json", false)?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert!(request.get("stream_options").is_none());
	let message = assistant(root.path())?;
	let usage: Value = serde_json::from_str(message["usage"].get())?;
	assert_eq!(usage["input"], 70);
	assert_eq!(usage["cacheRead"], 30);
	assert_eq!(usage["cacheWrite"], 0);
	assert_eq!(usage["totalTokens"], 120);
	assert_eq!(message["usageUnavailable"].get(), "false");
	Ok(())
}

#[test]
fn partial_usage_is_retained_without_claiming_complete_token_accounting() -> Result<()> {
	let root = tempfile::tempdir()?;
	let raw = r#"{"total_tokens":7,"future":900719925474099312345}"#;
	let (output, _) = run(root.path(), stream(raw, true), "text/event-stream", false)?;
	assert!(output.status.success());
	let message = assistant(root.path())?;
	assert_eq!(message["providerUsage"].get(), raw);
	assert_eq!(message["usageUnavailable"].get(), "true");
	Ok(())
}

#[test]
fn unsuccessful_json_and_unterminated_sse_never_commit_assistant_usage() -> Result<()> {
	for (body, content_type) in [
		(r#"{"choices":[{"message":{"role":"assistant","content":"partial"},"finish_reason":"length"}],"usage":{"prompt_tokens":10,"completion_tokens":20}}"#.to_owned(), "application/json"),
		(stream(r#"{"prompt_tokens":10,"completion_tokens":20}"#, false), "text/event-stream"),
	] {
		let root = tempfile::tempdir()?;
		let (output, _) = run(root.path(), body, content_type, false)?;
		assert!(!output.status.success());
		let bytes = std::fs::read(root.path().join(".omp/session.jsonl"))?;
		let document = SessionDocument::decode(&bytes)?;
		assert_eq!(document.entries().len(), 1);
		let message = assistant(root.path())?;
		assert_eq!(message["role"].get(), "\"user\"");
		assert!(!message.contains_key("providerUsage"));
	}
	Ok(())
}
