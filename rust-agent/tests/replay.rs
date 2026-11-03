use std::{fs, process::Command};

use anyhow::Result;
use harvest_terminal::text;

#[test]
fn terminal_replay_preserves_source_and_renders_only_the_selected_safe_branch() -> Result<()> {
	let root = tempfile::tempdir()?;
	let source = include_bytes!("../../crates/harvest-protocol/tests/fixtures/web_replay.jsonl");
	let path = root.path().join("session.jsonl");
	fs::write(&path, source)?;
	let run = |leaf: &str| {
		Command::new(assert_cmd::cargo::cargo_bin!("harvest"))
			.args(["replay", "--workspace"])
			.arg(root.path())
			.args(["--leaf", leaf, "--columns", "32", "session.jsonl"])
			.output()
	};
	let output = run("browser-smoke")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let body = String::from_utf8(output.stdout)?;
	assert!(body.contains("Historical JavaScript"));
	assert!(body.lines().any(|line| line == "42"));
	assert!(!body.contains("provider-secret") && !body.contains("hidden-body-secret"));
	for line in body.lines() {
		assert!(!line.chars().any(char::is_control));
		assert!(text::visible_width(&line.encode_utf16().collect::<Vec<_>>(), 3) <= 32);
	}
	let failure = run("missing-branch")?;
	assert!(!failure.status.success());
	assert!(failure.stdout.is_empty(), "invalid branch cannot emit a partial transcript");
	assert_eq!(fs::read(path)?, source);
	Ok(())
}
