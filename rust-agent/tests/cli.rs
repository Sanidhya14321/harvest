use std::io::{Read, Write};
use std::net::TcpListener;
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
            let mut buffer = [0_u8; 8192];
            if socket.read(&mut buffer).is_err() {
                return;
            }
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
