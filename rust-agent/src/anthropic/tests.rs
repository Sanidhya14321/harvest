use std::{
	io::{BufRead, BufReader, Read, Write},
	net::TcpListener,
	thread,
};

use super::*;
use crate::{catalog::Wire, chat::user};

/// Serve one canned response; return the lowercased request head and JSON body.
fn serve(body: &'static str) -> (String, thread::JoinHandle<(String, Value)>) {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	let handle = thread::spawn(move || {
		let (mut socket, _) = listener.accept().unwrap();
		let mut reader = BufReader::new(socket.try_clone().unwrap());
		let mut head = String::new();
		let mut length = 0;
		loop {
			let mut line = String::new();
			reader.read_line(&mut line).unwrap();
			if line == "\r\n" {
				break;
			}
			let lower = line.to_ascii_lowercase();
			if let Some(value) = lower.strip_prefix("content-length:") {
				length = value.trim().parse().unwrap();
			}
			head.push_str(&lower);
		}
		let mut request = vec![0; length];
		reader.read_exact(&mut request).unwrap();
		write!(
			socket,
			"HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
			body.len()
		)
		.unwrap();
		(head, serde_json::from_slice(&request).unwrap())
	});
	(format!("http://{address}"), handle)
}

fn model(base_url: String) -> AnthropicModel {
	AnthropicModel::new(Settings {
		model: "claude-test".into(),
		base_url,
		api_key: "secret".into(),
		max_steps: 1,
		wire: Wire::Anthropic,
		max_tokens: Some(64_000),
	})
}

fn message(role: &str, content: Option<&str>) -> ChatMessage {
	ChatMessage {
		role:         role.into(),
		content:      content.map(Into::into),
		tool_calls:   None,
		tool_call_id: None,
	}
}

fn call(id: &str, arguments: &str) -> ToolCall {
	ToolCall {
		id:       id.into(),
		kind:     "function".into(),
		function: FunctionCall { name: "read".into(), arguments: arguments.into() },
	}
}

#[cfg_attr(miri, ignore)]
#[test]
fn transcript_translates_and_tool_use_round_trips() {
	let (url, server) = serve(
		r#"{"content":[{"type":"text","text":"reading"},{"type":"tool_use","id":"tu_2","name":"read","input":{"path":"b"}}],"stop_reason":"tool_use","usage":{"input_tokens":5,"output_tokens":4,"cache_read_input_tokens":10,"cache_creation_input_tokens":2}}"#,
	);
	let mut assistant = message("assistant", None);
	assistant.tool_calls = Some(vec![call("tu_1", r#"{"path":"a"}"#), call("tu_x", "not json")]);
	let mut first = message("tool", Some("A"));
	first.tool_call_id = Some("tu_1".into());
	let mut second = message("tool", Some("bad"));
	second.tool_call_id = Some("tu_x".into());
	let transcript =
		[message("system", Some("be brief")), user("hi"), assistant, first, second, user("next")];
	let response = model(url).complete_response(&transcript, &[]).unwrap();
	let (head, body) = server.join().unwrap();
	assert!(head.starts_with("post /v1/messages "), "{head}");
	assert!(head.contains("x-api-key: secret") && head.contains("anthropic-version: 2023-06-01"));
	assert!(!head.contains("authorization"));
	assert_eq!(body["system"], "be brief");
	assert_eq!(body["max_tokens"], MAX_OUTPUT_TOKENS);
	assert_eq!(
		body["messages"],
		json!([
			{"role": "user", "content": [{"type": "text", "text": "hi"}]},
			{"role": "assistant", "content": [
				{"type": "tool_use", "id": "tu_1", "name": "read", "input": {"path": "a"}},
				{"type": "tool_use", "id": "tu_x", "name": "read", "input": {}},
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "tu_1", "content": "A"},
				{"type": "tool_result", "tool_use_id": "tu_x", "content": "bad"},
				{"type": "text", "text": "next"},
			]},
		])
	);
	assert_eq!(response.message.content.as_deref(), Some("reading"));
	let calls = response.message.tool_calls.unwrap();
	assert_eq!(calls[0].id, "tu_2");
	assert_eq!(calls[0].function.arguments, r#"{"path":"b"}"#);
	let usage = response.usage.unwrap().tokens.unwrap();
	assert_eq!(
		(usage.input, usage.cache_read, usage.cache_write, usage.total_tokens),
		(5, 10, 2, 21)
	);
}

#[cfg_attr(miri, ignore)]
#[test]
fn truncated_responses_are_rejected() {
	let (url, server) =
		serve(r#"{"content":[{"type":"text","text":"cut"}],"stop_reason":"max_tokens"}"#);
	let error = model(url)
		.complete_response(&[user("hi")], &[])
		.unwrap_err();
	server.join().unwrap();
	assert_eq!(error.to_string(), "model stopped with an unsuccessful finish reason");
}
