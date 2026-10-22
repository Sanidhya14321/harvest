use std::{path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use serde_json::Value;
use tempfile::tempdir;

const PROVIDER: &str = "packages/catalog/src/wire/gemini-headers.ts";

#[test]
fn installed_checker_matches_legacy_drift_update_and_invalid_release_contracts() -> Result<()> {
	let oracle: Value = serde_json::from_str(include_str!("fixtures/spoofed-versions-oracle.json"))?;
	for case in oracle["cases"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|case| case["http_status"] == 200)
	{
		let root = tempdir()?;
		let fs = JailedFs::open(root.path())?;
		fs.atomic_write(Path::new(PROVIDER), case["source"].as_str().unwrap().as_bytes())?;
		fs.atomic_write(Path::new("release.json"), &serde_json::to_vec(&case["release"])?)?;
		let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
		command
			.args(["spoofed-versions", "--root"])
			.arg(root.path())
			.args(["--release-json", "release.json"]);
		if case["update"] == true {
			command.arg("--update");
		}
		// The tested command is the installed native binary itself, not Cargo,
		// Bun, Node or a source-loading wrapper.
		let output = command.env("PATH", root.path()).output()?;
		assert_eq!(
			output.status.code(),
			case["exit_code"]
				.as_i64()
				.map(|code| i32::try_from(code).unwrap()),
			"{}: {}",
			case["name"],
			String::from_utf8_lossy(&output.stderr)
		);
		assert_eq!(
			fs.read(Path::new(PROVIDER), 1024 * 1024)?,
			case["expected_source"].as_str().unwrap().as_bytes(),
			"{}",
			case["name"]
		);
	}
	Ok(())
}

#[test]
fn invalid_or_ambiguous_declarations_do_not_overwrite_source() -> Result<()> {
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	let declaration = "const version = process.env.PI_AI_GEMINI_CLI_VERSION || \"0.46.0\";\n";
	for source in ["no version declaration".to_owned(), declaration.repeat(2)] {
		fs.atomic_write(Path::new(PROVIDER), source.as_bytes())?;
		assert!(xtask::spoofed_versions::check(&fs, br#"{"tag_name":"v0.48.2"}"#, true).is_err());
		assert_eq!(fs.read(Path::new(PROVIDER), 1024 * 1024)?, source.as_bytes());
	}
	Ok(())
}

#[cfg(unix)]
#[test]
fn updating_a_provider_cannot_follow_an_external_symlink() -> Result<()> {
	let root = tempdir()?;
	let external = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	let source = b"const version = process.env.PI_AI_GEMINI_CLI_VERSION || \"0.46.0\";\n";
	let private = external.path().join("private.ts");
	std::fs::write(&private, source)?;
	std::fs::create_dir_all(root.path().join("packages/catalog/src/wire"))?;
	std::os::unix::fs::symlink(&private, root.path().join(PROVIDER))?;
	assert!(xtask::spoofed_versions::check(&fs, br#"{"tag_name":"v0.48.2"}"#, true).is_err());
	assert_eq!(std::fs::read(&private)?, source);
	Ok(())
}
