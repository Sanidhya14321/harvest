//! Installed CLI and ordinary SDK exercise the same Messages wire boundary.
use std::{fs, process::Command};

use anyhow::Result;
use harvest_omp::{
	chat,
	config::Settings,
	provider::{HttpModel, Model},
	session_format::SessionDocument,
};
use serde_json::{Value, json};
mod common;

fn response() -> String {
	r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":3,"cache_creation_input_tokens":2,"future":1e400}}"#.into()
}
#[test]
fn sdk_sends_messages_endpoint_version_and_api_key_without_bearer_auth() -> Result<()> {
	let server = common::HttpFixture::spawn_inspecting(
		200,
		"OK",
		"application/json",
		response(),
		|headers, body| {
			let headers = headers.to_ascii_lowercase();
			assert!(headers.starts_with("post /v1/messages http/1.1"));
			assert!(headers.contains("x-api-key: fixture-secret\r\n"));
			assert!(headers.contains("anthropic-version: 2023-06-01\r\n"));
			assert!(!headers.contains("authorization:"));
			assert_eq!(body["stream"], false);
			Ok(())
		},
	)?;
	let model = HttpModel::new(Settings {
		model:     "fixture".into(),
		base_url:  server.base_url(),
		api_key:   "fixture-secret".into(),
		max_steps: 1,
	})
	.anthropic_messages(123)?;
	let response = model.complete_response(&[chat::user("question")], &chat::specs())?;
	assert_eq!(response.message.content.as_deref(), Some("native answer"));
	assert_eq!(response.usage.unwrap().tokens.unwrap().total_tokens, 20);
	assert_eq!(server.finish()?["max_tokens"], 123);
	assert!(!format!("{model:?}").contains("fixture-secret"));
	Ok(())
}
#[test]
fn installed_binary_supports_sync_async_and_durable_replay_without_legacy_runtime() -> Result<()> {
	let install = tempfile::tempdir()?;
	let executable = install
		.path()
		.join(format!("harvest{}", std::env::consts::EXE_SUFFIX));
	fs::copy(env!("CARGO_BIN_EXE_harvest"), &executable)?;
	let root = tempfile::tempdir()?;
	for async_session in [false, true] {
		let (content_type, body) = if async_session {
			("text/event-stream", include_str!("fixtures/anthropic/text-usage.sse").into())
		} else {
			("application/json", response())
		};
		let server = common::HttpFixture::spawn(200, "OK", content_type, body)?;
		let mut command = Command::new(&executable);
		command
			.args(["run", "--api", "anthropic-messages", "--model", "fixture", "--base-url"])
			.arg(server.base_url())
			.args([
				"--api-key",
				"fixture-secret",
				"--max-output-tokens",
				"77",
				"--prompt",
				"question",
				"--workspace",
			])
			.arg(root.path())
			.env("PATH", install.path())
			.env_remove("OMP_MAX_STEPS");
		if async_session {
			command.arg("--async-session");
		}
		let output = command.output()?;
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		assert!(String::from_utf8_lossy(&output.stdout).contains("native answer"));
		let request = server.finish()?;
		assert_eq!(request["max_tokens"], 77);
		assert_eq!(request["stream"], true);
		if async_session {
			assert!(
				request["messages"]
					.as_array()
					.unwrap()
					.iter()
					.any(|message| message["role"] == "assistant"
						&& message["content"][0]["text"] == "native answer")
			);
		}
	}
	let document = SessionDocument::decode(&fs::read(root.path().join(".omp/session.jsonl"))?)?;
	let entry = document.entries().last().unwrap();
	let raw: harvest_omp::session_format::SessionObject =
		serde_json::from_str(entry["message"].get())?;
	let usage: Value = serde_json::from_str(raw["usage"].get())?;
	assert_eq!(usage["totalTokens"], 20);
	assert_eq!(raw["costUnavailable"].get(), "true");
	assert!(raw["providerUsage"].get().contains("1e400"));
	Ok(())
}
#[test]
fn failed_provider_does_not_leak_response_secrets() -> Result<()> {
	let server = common::HttpFixture::spawn(
		401,
		"Unauthorized",
		"application/json",
		json!({"error":{"message":"server-secret"}}).to_string(),
	)?;
	let model = HttpModel::new(Settings {
		model:     "fixture".into(),
		base_url:  server.base_url(),
		api_key:   "key-secret".into(),
		max_steps: 1,
	})
	.anthropic_messages(8)?;
	let error = model.complete(&[chat::user("question")], &[]).unwrap_err();
	let diagnostic = format!("{error:#} {error:?}");
	assert!(diagnostic.contains("401"));
	assert!(!diagnostic.contains("server-secret") && !diagnostic.contains("key-secret"));
	server.finish()?;
	Ok(())
}

#[test]
fn real_session_executes_multiple_named_reads_and_replays_their_results() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::write(root.path().join("a"), "first file")?;
	fs::write(root.path().join("b"), "second file")?;
	let calls = json!({"type":"message","role":"assistant","content":[
		{"type":"tool_use","id":"one","name":"read","input":{"path":"a"}},
		{"type":"tool_use","id":"two","name":"read","input":{"path":"b"}}
	],"stop_reason":"tool_use"})
	.to_string();
	let server = common::HttpFixture::spawn_sequence(
		200,
		"OK",
		"application/json",
		vec![calls, response()],
		|_, _| Ok(()),
	)?;
	let output = Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["run", "--api", "anthropic-messages", "--model", "fixture", "--base-url"])
		.arg(server.base_url())
		.args(["--api-key", "", "--prompt", "read the files", "--workspace"])
		.arg(root.path())
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let requests = server.finish_sequence()?;
	assert_eq!(requests.len(), 2);
	let messages = requests[1]["messages"].as_array().unwrap();
	let assistant = messages.iter().find(|m| m["role"] == "assistant").unwrap();
	assert_eq!(assistant["content"][0]["id"], "one");
	assert_eq!(assistant["content"][1]["id"], "two");
	let results = messages.last().unwrap()["content"].as_array().unwrap();
	assert_eq!(results.len(), 2);
	assert_eq!(results[0]["tool_use_id"], "one");
	assert_eq!(results[1]["tool_use_id"], "two");
	assert!(
		results[0]["content"]
			.as_str()
			.unwrap()
			.contains("first file")
	);
	assert!(
		results[1]["content"]
			.as_str()
			.unwrap()
			.contains("second file")
	);
	Ok(())
}

#[test]
fn messages_api_requires_explicit_model_and_host_until_catalog_is_resolved() -> Result<()> {
	let output = Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["run", "--api", "anthropic-messages", "--prompt", "question"])
		.output()?;
	assert!(!output.status.success());
	let error = String::from_utf8_lossy(&output.stderr);
	assert!(error.contains("--base-url") && error.contains("--model"));
	Ok(())
}

#[test]
fn api_keys_are_not_forwarded_to_a_redirected_origin() -> Result<()> {
	use std::{
		io::{BufRead, BufReader, Read, Write},
		net::TcpListener,
		thread,
		time::{Duration, Instant},
	};
	let destination = TcpListener::bind("127.0.0.1:0")?;
	destination.set_nonblocking(true)?;
	let location = format!("http://{}/stolen", destination.local_addr()?);
	let source = TcpListener::bind("127.0.0.1:0")?;
	source.set_nonblocking(true)?;
	let url = format!("http://{}/v1", source.local_addr()?);
	let server = thread::spawn(move || -> Result<()> {
		let deadline = Instant::now() + Duration::from_secs(5);
		let mut socket = loop {
			match source.accept() {
				Ok((socket, _)) => break socket,
				Err(error)
					if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
				{
					thread::sleep(Duration::from_millis(5));
				},
				Err(error) => return Err(error.into()),
			}
		};
		socket.set_nonblocking(false)?;
		socket.set_read_timeout(Some(Duration::from_secs(5)))?;
		let mut reader = BufReader::new(&mut socket);
		let mut length = 0;
		let mut consumed = 0;
		loop {
			let mut line = String::new();
			let count = (&mut reader).take(16 * 1024 + 1).read_line(&mut line)?;
			consumed += count;
			anyhow::ensure!(count > 0 && consumed < 16 * 1024, "invalid fixture request");
			if line == "\r\n" {
				break;
			}
			if let Some((name, value)) = line.split_once(':')
				&& name.eq_ignore_ascii_case("content-length")
			{
				length = value.trim().parse::<u64>()?;
			}
		}
		std::io::copy(&mut (&mut reader).take(length), &mut std::io::sink())?;
		drop(reader);
		write!(
			socket,
			"HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: \
			 0\r\nConnection: close\r\n\r\n"
		)?;
		Ok(())
	});
	let model = HttpModel::new(Settings {
		model:     "fixture".into(),
		base_url:  url,
		api_key:   "redirect-secret".into(),
		max_steps: 1,
	})
	.anthropic_messages(8)?;
	let error = model.complete(&[chat::user("question")], &[]).unwrap_err();
	assert!(error.to_string().contains("307"));
	server.join().unwrap()?;
	assert_eq!(destination.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
	Ok(())
}
