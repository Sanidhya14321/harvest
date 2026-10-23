use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail};
use harvest_omp::durable_session::DurableSession;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

async fn frame<R: tokio::io::AsyncBufRead + Unpin>(output: &mut R) -> Result<Value> {
	let mut bytes = Vec::new();
	if tokio::time::timeout(Duration::from_secs(10), output.read_until(b'\n', &mut bytes)).await??
		== 0
	{
		bail!("RPC executable closed stdout early");
	}
	assert!(bytes.len() <= harvest_runtime::MAX_EVENT_BYTES);
	Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
async fn executable_rpc_streams_real_provider_bytes_and_closes_its_session_on_stdin_eof()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let installation = tempfile::tempdir()?;
	let candidate = installation
		.path()
		.join(format!("harvest{}", std::env::consts::EXE_SUFFIX));
	std::fs::copy(env!("CARGO_BIN_EXE_omp"), &candidate)?;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let address = listener.local_addr()?;
	let provider = tokio::spawn(async move {
		let (mut socket, _) = listener.accept().await?;
		let mut request = Vec::new();
		loop {
			let mut chunk = [0u8; 4096];
			let count = socket.read(&mut chunk).await?;
			if count == 0 {
				bail!("provider request closed early");
			}
			request.extend_from_slice(&chunk[..count]);
			if request.len() > 64 * 1024 {
				bail!("fixture request exceeds bound");
			}
			if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
				let header = std::str::from_utf8(&request[..end])?;
				let length = header
					.lines()
					.find_map(|line| {
						line
							.to_ascii_lowercase()
							.strip_prefix("content-length:")
							.map(|value| value.trim().parse::<usize>())
					})
					.transpose()?
					.context("missing provider request length")?;
				if request.len() >= end + 4 + length {
					assert!(header.starts_with("POST /v1/chat/completions "));
					let body: Value = serde_json::from_slice(&request[end + 4..end + 4 + length])?;
					assert_eq!(body["model"], "rpc-contract");
					assert_eq!(body["stream"], true);
					assert_eq!(
						body["messages"].as_array().unwrap().last().unwrap()["content"],
						"wire prompt"
					);
					break;
				}
			}
		}
		let body = concat!(
			"data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"from \
			 \"}}]}\n\n",
			"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Rust\"}}]}\n\n",
			"data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
			"data: [DONE]\n\n",
		);
		socket
			.write_all(
				format!(
					"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: \
					 {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)
				.as_bytes(),
			)
			.await?;
		Ok::<(), anyhow::Error>(())
	});
	let mut child = tokio::process::Command::new(&candidate)
		.current_dir(root.path())
		.env("PATH", installation.path())
		.env("OMP_SKIP_INTEGRATIONS", "1")
		.args([
			"rpc",
			"--session",
			"rpc.jsonl",
			"--model",
			"rpc-contract",
			"--base-url",
			&format!("http://{address}/v1"),
			"--api-key",
			"fixture",
		])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.kill_on_drop(true)
		.spawn()?;
	let mut input = child.stdin.take().context("missing RPC stdin")?;
	let mut output = BufReader::new(child.stdout.take().context("missing RPC stdout")?);
	assert_eq!(frame(&mut output).await?["type"], "ready");
	input
		.write_all(b"{\"id\":\"prompt\",\"type\":\"prompt\",\"message\":\"wire prompt\"}\n")
		.await?;
	let mut deltas = String::new();
	let mut acknowledged = false;
	let assistant = loop {
		let event = frame(&mut output).await?;
		if event["type"] == "response" && event["id"] == "prompt" {
			assert_eq!(event["success"], true, "RPC command response: {event}");
			acknowledged = true;
		}
		if event["assistantMessageEvent"]["type"] == "text_delta" {
			deltas.push_str(event["assistantMessageEvent"]["delta"].as_str().unwrap());
		}
		if event["type"] == "agent_end" {
			assert!(acknowledged, "prompt acknowledgement precedes its lifecycle events");
			assert_eq!(event["isTerminal"], true);
			break event["messages"][1].clone();
		}
	};
	assert_eq!(deltas, "from Rust");
	assert_eq!(assistant["content"][0]["text"], "from Rust");
	assert_eq!(assistant["model"], "rpc-contract");
	input
		.write_all(b"{\"id\":\"history\",\"type\":\"get_messages\"}\n")
		.await?;
	let history = frame(&mut output).await?;
	assert_eq!(history["id"], "history");
	assert_eq!(history["data"]["messages"][1], assistant);
	for (request, expected) in [
		(json!({"id":"copy","type":"get_last_assistant_text"}), json!({"text":"from Rust"})),
		(
			json!({"id":"page","type":"get_messages_page","limit":256}),
			json!({"messages":history["data"]["messages"],"totalMessages":2}),
		),
	] {
		let mut bytes = serde_json::to_vec(&request)?;
		bytes.push(b'\n');
		input.write_all(&bytes).await?;
		let response = frame(&mut output).await?;
		assert_eq!(response["id"], request["id"]);
		assert_eq!(response["success"], true);
		assert_eq!(response["data"], expected);
	}
	input.shutdown().await?;
	drop(input);
	let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await??;
	if !status.success() {
		let mut error = String::new();
		child
			.stderr
			.take()
			.context("missing stderr")?
			.take(64 * 1024)
			.read_to_string(&mut error)
			.await?;
		bail!("RPC executable failed: {error}");
	}
	provider.await??;
	let reopened = DurableSession::open(root.path(), Path::new("rpc.jsonl"), None)?;
	assert_eq!(reopened.messages().len(), 2);
	assert_eq!(reopened.messages()[1].content.as_deref(), Some("from Rust"));
	assert_eq!(json!(reopened.leaf().is_some()), true);
	Ok(())
}
