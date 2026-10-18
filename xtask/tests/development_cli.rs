//! Developer commands must run real Cargo gates and preserve fail-fast
//! ordering.
use std::{
	fs,
	path::Path,
	process::{Command, Output},
};

use anyhow::Result;

fn fixture(root: &Path, source: &str) -> Result<()> {
	fs::create_dir_all(root.join("src"))?;
	fs::create_dir_all(root.join("brush-core/src"))?;
	fs::write(
		root.join("Cargo.toml"),
		r#"[package]
name="dev-gate-fixture"
version="0.1.0"
edition="2024"
[workspace]
members=["brush-core"]
"#,
	)?;
	fs::write(
		root.join("brush-core/Cargo.toml"),
		"[package]\nname=\"brush-core\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
	)?;
	fs::write(
		root.join("brush-core/src/lib.rs"),
		"compile_error!(\"vendored fork must be excluded\");\n",
	)?;
	fs::write(root.join("src/lib.rs"), source)?;
	Ok(())
}

fn run(root: &Path, task: &str) -> Result<Output> {
	Ok(Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["dev", task, "--always", "--workspace"])
		.arg(root)
		.env("CARGO_TERM_COLOR", "never")
		.output()?)
}

#[test]
fn formatting_gate_fails_then_fix_repairs_code_and_lint_accepts_it() -> Result<()> {
	let root = tempfile::tempdir()?;
	fixture(root.path(), "pub fn double(value:u32)->u32{value*2}\n")?;
	assert!(!run(root.path(), "check")?.status.success());
	let output = run(root.path(), "fix")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	for task in ["check", "lint", "fmt"] {
		let output = run(root.path(), task)?;
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	}
	// Without Git or rustup on PATH, use Cargo's explicit executable and run
	// conservatively instead of skipping an uninspectable workspace.
	let cargo = Path::new(env!("CARGO"));
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["dev", "lint", "--workspace"])
		.arg(root.path())
		.env("CI", "false")
		.env("CARGO", cargo)
		.env("PATH", cargo.parent().unwrap())
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert!(!String::from_utf8(output.stdout)?.contains("Skipping"));

	Ok(())
}

#[test]
fn doctest_failure_is_reported_after_nextest_and_nextest_failure_stops_before_docs() -> Result<()> {
	let root = tempfile::tempdir()?;
	fixture(
		root.path(),
		"/// ```\n/// compile_error!(\"DOCTEST_FAILURE\");\n/// ```\npub fn double(value: u32) -> \
		 u32 { value * 2 }\n#[test]\nfn computes_result() { assert_eq!(double(21), 42); }\n",
	)?;
	let output = run(root.path(), "test")?;
	assert!(!output.status.success());
	let combined = format!(
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(combined.contains("DOCTEST_FAILURE"), "{combined}");
	fs::write(
		root.path().join("src/lib.rs"),
		"/// ```\n/// compile_error!(\"DOCS_MUST_NOT_RUN\");\n/// ```\npub fn example() \
		 {}\n#[test]\nfn fails() { panic!(\"NEXTEST_FAILURE\"); }\n",
	)?;
	let output = run(root.path(), "test")?;
	assert!(!output.status.success());
	let combined = format!(
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(combined.contains("NEXTEST_FAILURE"), "{combined}");
	assert!(!combined.contains("DOCS_MUST_NOT_RUN"), "{combined}");
	fs::write(
		root.path().join("src/lib.rs"),
		"/// ```\n/// assert_eq!(dev_gate_fixture::double(21), 42);\n/// ```\npub fn double(value: \
		 u32) -> u32 { value * 2 }\n#[test]\nfn computes_result() { assert_eq!(double(21), 42); }\n",
	)?;
	let output = run(root.path(), "test")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	Ok(())
}

#[test]
fn docs_only_git_tree_skips_local_gates_and_ci_disables_that_skip() -> Result<()> {
	let root = tempfile::tempdir()?;
	assert!(
		Command::new("git")
			.args(["init", "--quiet"])
			.arg(root.path())
			.status()?
			.success()
	);
	fs::write(root.path().join("guide.md"), "documentation only")?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["dev", "lint", "--workspace"])
		.arg(root.path())
		.env("CI", "false")
		.output()?;
	assert!(output.status.success());
	assert!(String::from_utf8(output.stdout)?.contains("Skipping"));
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["dev", "lint", "--workspace"])
		.arg(root.path())
		.env("CI", "true")
		.output()?;
	assert!(!output.status.success());
	assert!(!String::from_utf8(output.stdout)?.contains("Skipping"));
	Ok(())
}

#[test]
fn developer_help_succeeds_and_unknown_tasks_fail_before_mutating_the_workspace() -> Result<()> {
	let root = tempfile::tempdir()?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["dev", "--help"])
		.current_dir(root.path())
		.output()?;
	assert!(output.status.success());
	assert!(String::from_utf8(output.stdout)?.contains("--always"));
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["dev", "unknown"])
		.current_dir(root.path())
		.output()?;
	assert!(!output.status.success());
	assert_eq!(fs::read_dir(root.path())?.count(), 0);
	Ok(())
}
