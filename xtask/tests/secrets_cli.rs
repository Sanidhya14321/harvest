//! The CI-facing command must fail with redacted, parseable locations.
use std::{
	io::Write,
	process::{Command, Stdio},
};

use anyhow::Result;

#[test]
fn ci_receives_failure_without_printing_secret_and_clean_manifest_succeeds() -> Result<()> {
	let root = tempfile::tempdir()?;
	let secret = format!("ghp_{}", "a".repeat(36));
	std::fs::write(root.path().join("config.txt"), format!("header\n{secret}\n"))?;
	let mut process = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["secrets", "--files-stdin"])
		.current_dir(root.path())
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()?;
	process.stdin.take().unwrap().write_all(b"config.txt\0")?;
	let output = process.wait_with_output()?;
	assert!(!output.status.success());
	let stderr = String::from_utf8(output.stderr)?;
	assert!(stderr.contains("config.txt:2: github token"));
	assert!(!stderr.contains(&secret));
	assert!(!String::from_utf8(output.stdout)?.contains(&secret));

	std::fs::write(root.path().join("config.txt"), "harmless settings\n")?;
	let mut process = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["secrets", "--files-stdin"])
		.current_dir(root.path())
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()?;
	process.stdin.take().unwrap().write_all(b"config.txt\0")?;
	let output = process.wait_with_output()?;
	assert!(output.status.success());
	assert_eq!(String::from_utf8(output.stdout)?, "check_secrets: 1 tracked file(s) clean\n");
	Ok(())
}
