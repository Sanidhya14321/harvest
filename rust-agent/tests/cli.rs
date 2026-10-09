use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::{fs, thread};

use assert_cmd::Command;
use predicates::str::contains;

fn or_abort<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            let mut sink = std::io::stderr().lock();
            match writeln!(sink, "{error}") {
                Ok(()) => {}
                Err(write_error) => drop(write_error),
            }
            std::process::abort();
        }
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
