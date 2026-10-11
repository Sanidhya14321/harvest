use std::{
	fs,
	io::{Read, Write},
	net::{TcpListener, TcpStream},
	thread,
};

use assert_cmd::Command;
use predicates::str::contains;

fn or_abort<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
	match result {
		Ok(value) => value,
		Err(error) => {
			let mut sink = std::io::stderr().lock();
			match writeln!(sink, "{error}") {
				Ok(()) => {},
				Err(write_error) => drop(write_error),
			}
			std::process::abort();
		},
	}
}

fn omp() -> Command {
	let mut command = or_abort(Command::cargo_bin("omp"));
	let _configured: &mut Command = command.env("OMP_SKIP_INTEGRATIONS", "1");
	command
}

fn keep(assertion: assert_cmd::assert::Assert) {
	drop(assertion);
}

fn drain_request(socket: &mut TcpStream) {
	let mut received = Vec::new();
	let mut buffer = [0_u8; 1024];
	loop {
		let count = match socket.read(&mut buffer) {
			Ok(0) | Err(_) => return,
			Ok(count) => count,
		};
		let Some(chunk) = buffer.get(..count) else {
			return;
		};
		received.extend_from_slice(chunk);
		if request_complete(&received) {
			return;
		}
	}
}

fn request_complete(received: &[u8]) -> bool {
	let Some(header_end) = received.windows(4).position(|mark| mark == b"\r\n\r\n") else {
		return false;
	};
	let Some(header_bytes) = received.get(..header_end) else {
		return false;
	};
	let header = String::from_utf8_lossy(header_bytes).to_ascii_lowercase();
	let mut length = 0_usize;
	for line in header.lines() {
		let Some(value) = line.strip_prefix("content-length:") else {
			continue;
		};
		length = value.trim().parse().unwrap_or(0);
	}
	received.len() >= header_end.saturating_add(4).saturating_add(length)
}

#[test]
fn version_help_and_tools_work() {
	keep(
		omp()
			.arg("--version")
			.assert()
			.success()
			.stdout(contains(env!("CARGO_PKG_VERSION"))),
	);
	keep(
		omp()
			.arg("--help")
			.assert()
			.success()
			.stdout(contains("run")),
	);
	keep(
		omp()
			.arg("tools")
			.assert()
			.success()
			.stdout(contains("grep")),
	);
	keep(omp().arg("man").assert().success().stdout(contains("omp")));
	keep(
		omp()
			.args(["completions", "bash"])
			.assert()
			.success()
			.stdout(contains("omp")),
	);
}

#[test]
fn run_uses_a_local_model_end_to_end() {
	let root = or_abort(tempfile::tempdir());
	or_abort(fs::write(root.path().join("note.txt"), "hello from disk"));
	let listener = or_abort(TcpListener::bind("127.0.0.1:0"));
	let address = or_abort(listener.local_addr());
	let server = thread::spawn(move || {
		for body in [
			r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read","arguments":"{\"path\":\"note.txt\"}"}}]}}]}"#,
			r#"{"choices":[{"message":{"role":"assistant","content":"the note says hello"}}]}"#,
		] {
			let Ok((mut socket, _)) = listener.accept() else {
				return;
			};
			drain_request(&mut socket);
			let response = format!(
				"HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
				body.len()
			);
			if socket.write_all(response.as_bytes()).is_err() {
				return;
			}
		}
	});
	keep(
		omp()
			.args([
				"run",
				"--prompt",
				"read the note",
				"--workspace",
				root.path().to_str().unwrap_or("."),
				"--base-url",
				&format!("http://{address}/v1"),
				"--model",
				"test",
				"--max-steps",
				"4",
			])
			.assert()
			.success()
			.stdout(contains("the note says hello")),
	);
	let session = or_abort(fs::read_to_string(root.path().join(".omp/session.jsonl")));
	assert!(session.contains("hello from disk"), "{session}");
	or_abort(server.join().map_err(|_| "server thread panicked"));
}

#[test]
fn help_and_version_have_no_installation_side_effects() {
	let root = tempfile::tempdir().unwrap();
	for argument in ["--help", "--version"] {
		omp()
			.env_remove("OMP_SKIP_INTEGRATIONS")
			.env("HOME", root.path())
			.env("USERPROFILE", root.path())
			.env("XDG_DATA_HOME", root.path().join("data"))
			.env("XDG_STATE_HOME", root.path().join("state"))
			.arg(argument)
			.assert()
			.success();
	}
	assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn explicit_learning_is_retrievable_and_organizer_alias_is_reversible() {
	let root = tempfile::tempdir().unwrap();
	std::fs::write(
		root.path().join("lesson.md"),
		"Verify the observable outcome before reporting completion.",
	)
	.unwrap();
	omp()
		.current_dir(root.path())
		.args([
			"brain",
			"--scope",
			"project",
			"learn",
			"--title",
			"Verification",
			"--file",
			"lesson.md",
		])
		.assert()
		.success()
		.stdout(contains("supported admission"));
	omp()
		.current_dir(root.path())
		.args(["brain", "--scope", "project", "retrieve", "observable", "--json"])
		.assert()
		.success()
		.stdout(contains("Verify the observable outcome"));
	omp()
		.current_dir(root.path())
		.args(["brain", "--scope", "project", "orgnize-skills", "--dry-run", "--json"])
		.assert()
		.success()
		.stdout(contains("retained"));
}

#[test]
fn user_retrieval_honors_the_existing_application_agent_directory_override() {
	let root = tempfile::tempdir().unwrap();
	let agent = root.path().join("configured-agent");
	fs::create_dir_all(agent.join("brain")).unwrap();
	fs::write(
		agent.join("brain/preference.md"),
		"## Communication\nExplain verification outcomes concisely.",
	)
	.unwrap();
	omp()
		.current_dir(root.path())
		.env("HARVEST_CODING_AGENT_DIR", &agent)
		.env("HARVEST_AGENT_DIR", root.path().join("obsolete-location"))
		.args(["brain", "--scope", "user", "retrieve", "verification", "--json"])
		.assert()
		.success()
		.stdout(contains("Explain verification outcomes concisely"));
}

#[test]
fn executable_streams_events_and_resumes_its_v3_session() {
	let root = or_abort(tempfile::tempdir());
	let listener = or_abort(TcpListener::bind("127.0.0.1:0"));
	let address = or_abort(listener.local_addr());
	let server = thread::spawn(move || {
		for expected in ["first question", "second question"] {
			let (mut socket, _) = or_abort(listener.accept());
			or_abort(socket.set_read_timeout(Some(std::time::Duration::from_secs(10))));
			let mut received = Vec::new();
			let mut buffer = [0_u8; 1024];
			while !request_complete(&received) {
				let count = or_abort(socket.read(&mut buffer));
				assert!(count > 0);
				received.extend_from_slice(&buffer[..count]);
				assert!(received.len() < 32 * 1024);
			}
			let offset = received
				.windows(4)
				.position(|mark| mark == b"\r\n\r\n")
				.unwrap()
				+ 4;
			let request: serde_json::Value = or_abort(serde_json::from_slice(&received[offset..]));
			assert_eq!(request["stream"], true);
			assert!(
				request["messages"]
					.as_array()
					.unwrap()
					.iter()
					.any(|message| message["content"] == expected)
			);
			if expected == "second question" {
				assert!(
					request["messages"]
						.as_array()
						.unwrap()
						.iter()
						.any(|message| message["content"] == "remembered")
				);
			}
			let chunk = serde_json::json!({"choices":[{"index":0,"delta":{"content":"remembered"},"finish_reason":"stop"}]});
			let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
			let header = format!(
				"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: \
				 {}\r\nConnection: close\r\n\r\n",
				body.len()
			);
			or_abort(socket.write_all(header.as_bytes()));
			// Arbitrary network fragmentation must not alter SSE semantics.
			for fragment in body.as_bytes().chunks(7) {
				or_abort(socket.write_all(fragment));
			}
		}
	});
	for prompt in ["first question", "second question"] {
		let result = omp()
			.args([
				"run",
				"--workspace",
				root.path().to_str().unwrap(),
				"--session",
				"current.jsonl",
				"--prompt",
				prompt,
				"--base-url",
				&format!("http://{address}/v1"),
				"--events",
			])
			.assert()
			.success();
		let events: Vec<serde_json::Value> = result
			.get_output()
			.stdout
			.split(|byte| *byte == b'\n')
			.filter(|line| !line.is_empty())
			.map(|line| or_abort(serde_json::from_slice(line)))
			.collect();
		assert!(
			events
				.iter()
				.any(|event| event["event"]["type"] == "text_delta"
					&& event["event"]["text"] == "remembered")
		);
		assert_eq!(events.last().unwrap()["event"]["type"], "completed");
	}
	or_abort(server.join().map_err(|_| "server thread panicked"));
	let bytes = or_abort(fs::read(root.path().join("current.jsonl")));
	let document = or_abort(harvest_omp::session_format::SessionDocument::decode(&bytes));
	assert_eq!(document.entries().len(), 4);
}
