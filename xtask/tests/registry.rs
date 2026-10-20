use std::{
	fs,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::Result;
use harvest_runtime::JailedFs;
use tempfile::{TempDir, tempdir};

fn project() -> Result<TempDir> {
	let root = tempdir()?;
	let jailed = JailedFs::open(root.path())?;
	for (path, contents) in [
		("Cargo.toml", "[workspace]\nmembers = [\"core\", \"host\"]\nresolver = \"3\"\n"),
		(
			"core/Cargo.toml",
			"[package]\nname = \"harvest-registry-fixture-core\"\nversion = \"0.1.0\"\nedition = \
			 \"2024\"\n[lib]\nname = \"brush_core\"\n",
		),
		("core/src/lib.rs", "pub fn answer() -> u8 { 41 }\n"),
		(
			"host/Cargo.toml",
			"[package]\nname = \"harvest-registry-fixture-host\"\nversion = \"0.1.0\"\nedition = \
			 \"2024\"\n[dependencies]\nbrush-core = { package = \"harvest-registry-fixture-core\", \
			 path = \"../core\", version = \"=0.1.0\" }\n",
		),
		("host/src/lib.rs", "pub fn answer() -> u8 { brush_core::answer() + 1 }\n"),
		(
			"consumer/Cargo.toml",
			"[package]\nname = \"detached-fixture\"\nversion = \"0.0.0\"\nedition = \
			 \"2024\"\n[dependencies]\nsubject = { package = \"harvest-registry-fixture-host\", \
			 version = \"=0.1.0\" }\n",
		),
		(
			"consumer/src/main.rs",
			"fn main() { println!(\"registry answer: {}\", subject::answer()); }\n",
		),
	] {
		jailed.atomic_write(Path::new(path), contents.as_bytes())?;
	}
	let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.current_dir(root.path())
		.args(["generate-lockfile", "--offline"])
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	Ok(root)
}

#[test]
fn actual_archives_resolve_renamed_dependencies_and_execute_without_workspace_paths() -> Result<()>
{
	let root = project()?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["registry", "verify", "--root"])
		.arg(root.path())
		.args([
			"-p",
			"harvest-registry-fixture-host",
			"--offline",
			"--consumer",
			"consumer",
			"--output",
			"stage",
		])
		.output()?;
	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	let receipt: serde_json::Value =
		serde_json::from_slice(&fs::read(root.path().join("stage/receipt.json"))?)?;
	assert_eq!(receipt["consumer_stdout"], "registry answer: 42\n");
	let archives = receipt["archives"].as_array().unwrap();
	assert_eq!(
		archives
			.iter()
			.map(|archive| archive["name"].as_str().unwrap())
			.collect::<Vec<_>>(),
		["harvest-registry-fixture-core", "harvest-registry-fixture-host"]
	);
	for archive in archives {
		let bytes = fs::read(
			root
				.path()
				.join("stage")
				.join(archive["path"].as_str().unwrap()),
		)?;
		assert_eq!(harvest_runtime::content_hash(&bytes), archive["sha256"]);
	}
	Ok(())
}

#[test]
fn registry_consumer_path_override_is_rejected_without_publishing_success_evidence() -> Result<()> {
	let root = project()?;
	let path = root.path().join("consumer/Cargo.toml");
	fs::write(
		&path,
		"[package]\nname = \"invalid-consumer\"\nversion = \"0.0.0\"\nedition = \
		 \"2024\"\n[dependencies]\nsubject = { package = \"harvest-registry-fixture-host\", path = \
		 \"../host\", version = \"=0.1.0\" }\n",
	)?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["registry", "verify", "--root"])
		.arg(root.path())
		.args([
			"-p",
			"harvest-registry-fixture-host",
			"--offline",
			"--consumer",
			"consumer",
			"--output",
			"stage",
		])
		.output()?;
	assert!(!output.status.success());
	assert!(
		String::from_utf8_lossy(&output.stderr)
			.contains("cannot contain workspace, patch or path overrides")
	);
	assert!(!root.path().join("stage/receipt.json").exists());
	Ok(())
}

#[test]
fn unrelated_consumer_cannot_certify_a_staged_package_it_never_resolved() -> Result<()> {
	let root = project()?;
	fs::write(
		root.path().join("consumer/Cargo.toml"),
		"[package]\nname = \"unrelated-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
	)?;
	fs::write(root.path().join("consumer/src/main.rs"), "fn main() {}\n")?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["registry", "verify", "--root"])
		.arg(root.path())
		.args([
			"-p",
			"harvest-registry-fixture-host",
			"--offline",
			"--consumer",
			"consumer",
			"--output",
			"stage",
		])
		.output()?;
	assert!(!output.status.success());
	assert!(
		String::from_utf8_lossy(&output.stderr).contains("consumer did not resolve staged package")
	);
	assert!(!root.path().join("stage/receipt.json").exists());
	Ok(())
}

#[cfg(unix)]
#[test]
fn escaping_stage_output_cannot_write_outside_the_authorized_workspace() -> Result<()> {
	let root = project()?;
	let outside = tempdir()?;
	fs::write(outside.path().join("protected"), b"previous")?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["registry", "verify", "--root"])
		.arg(root.path())
		.args(["-p", "harvest-registry-fixture-host", "--offline", "--output", "escape/stage"])
		.output()?;
	assert!(!output.status.success());
	assert_eq!(fs::read(outside.path().join("protected"))?, b"previous");
	assert!(!outside.path().join("stage").exists());
	Ok(())
}

#[test]
fn cached_upstream_can_verify_offline_but_corrupted_archives_still_fail_checksums() -> Result<()> {
	let root = project()?;
	let first = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["registry", "verify", "--root"])
		.arg(root.path())
		.args([
			"-p",
			"harvest-registry-fixture-host",
			"--offline",
			"--consumer",
			"consumer",
			"--output",
			"stage",
		])
		.output()?;
	assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
	let jailed = JailedFs::open(root.path())?;
	for (path, contents) in [
		("probe/Cargo.toml", "[workspace]\nmembers = [\"package\"]\nresolver = \"3\"\n"),
		(
			"probe/package/Cargo.toml",
			"[package]\nname = \"harvest-registry-cache-probe\"\nversion = \"0.0.0\"\nedition = \
			 \"2024\"\n[dependencies]\nsubject = { package = \"harvest-registry-fixture-host\", \
			 version = \"=0.1.0\" }\n",
		),
		("probe/package/src/lib.rs", "pub fn answer() -> u8 { subject::answer() + 1 }\n"),
	] {
		jailed.atomic_write(Path::new(path), contents.as_bytes())?;
	}
	let lockfile = fs::read_to_string(root.path().join("stage/consumer.Cargo.lock"))?
		.replace("name = \"detached-fixture\"", "name = \"harvest-registry-cache-probe\"");
	jailed.atomic_write(Path::new("probe/Cargo.lock"), lockfile.as_bytes())?;
	for name in ["harvest-registry-fixture-core", "harvest-registry-fixture-host"] {
		for relative in [format!("index/ha/rv/{name}"), format!("archives/{name}-0.1.0.crate")] {
			jailed.atomic_write(
				&PathBuf::from("probe/seed").join(&relative),
				&jailed.read(&PathBuf::from("stage").join(relative), 1024 * 1024)?,
			)?;
		}
	}
	let verify = |output: &str| {
		Command::new(env!("CARGO_BIN_EXE_xtask"))
			.args(["registry", "verify", "--root"])
			.arg(root.path().join("probe"))
			.args([
				"-p",
				"harvest-registry-cache-probe",
				"--offline",
				"--upstream-cache",
				"seed",
				"--output",
				output,
			])
			.output()
	};
	let cached = verify("cached")?;
	assert!(cached.status.success(), "{}", String::from_utf8_lossy(&cached.stderr));
	let receipt: serde_json::Value =
		serde_json::from_slice(&fs::read(root.path().join("probe/cached/receipt.json"))?)?;
	assert_eq!(receipt["upstream_allowed"], false);
	assert_eq!(receipt["archives"][0]["name"], "harvest-registry-cache-probe");
	jailed.atomic_write(
		Path::new("probe/seed/archives/harvest-registry-fixture-host-0.1.0.crate"),
		b"corrupted",
	)?;
	let corrupted = verify("rejected")?;
	assert!(!corrupted.status.success());
	assert!(
		String::from_utf8_lossy(&corrupted.stderr)
			.to_lowercase()
			.contains("checksum"),
		"{}",
		String::from_utf8_lossy(&corrupted.stderr)
	);
	assert!(!root.path().join("probe/rejected/receipt.json").exists());
	Ok(())
}
