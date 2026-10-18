use std::{path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use semver::Version;
use serde::Deserialize;
use tempfile::tempdir;

#[derive(Deserialize)]
struct Case {
	target:    String,
	floor:     String,
	changelog: String,
	expected:  String,
}

#[test]
fn generated_notes_match_legacy_silent_tag_deduplication_category_order_and_canary_contracts()
-> Result<()> {
	let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/release_notes.json"))?;
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(Path::new("packages/sample/package.json"), br#"{"name":"@harvest/sample"}"#)?;
	for case in cases {
		fs.atomic_write(Path::new("packages/sample/CHANGELOG.md"), case.changelog.as_bytes())?;
		let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
			.args(["release-notes", &case.target, "notes.md", "--floor", &case.floor, "--root"])
			.arg(root.path())
			.output()?;
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		assert_eq!(String::from_utf8(fs.read(Path::new("notes.md"), 64 * 1024)?)?, case.expected);
	}
	Ok(())
}

#[test]
fn release_floor_uses_only_published_stable_versions_strictly_below_the_target() -> Result<()> {
	let data=br#"[{"tagName":"v1.1.0"},{"tagName":"v1.3.0"},{"tagName":"v1.2.8","isDraft":true},{"tagName":"v1.2.7-canary.1"},{"tagName":"v1.2.6","isPrerelease":true},{"tagName":"v1.2.5"},{"tagName":"V1.2.9"}]"#;
	assert_eq!(
		xtask::release_notes::published_floor(data, &Version::new(1, 3, 0))?,
		Some(Version::new(1, 2, 5))
	);
	assert_eq!(xtask::release_notes::published_floor(data, &Version::new(1, 0, 0))?, None);
	assert!(
		xtask::release_notes::published_floor(
			br#"{"error":"authentication failed"}"#,
			&Version::new(1, 3, 0)
		)
		.is_err()
	);
	Ok(())
}

#[cfg(unix)]
#[test]
fn release_query_failure_preserves_previous_notes_instead_of_dropping_silent_tag_changes()
-> Result<()> {
	use std::os::unix::fs::PermissionsExt;
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(Path::new("notes.md"), b"previous notes")?;
	fs.atomic_write(Path::new("bin/gh"), b"#!/bin/sh\nexit 7\n")?;
	std::fs::set_permissions(root.path().join("bin/gh"), std::fs::Permissions::from_mode(0o700))?;
	let paths = std::env::join_paths(
		std::iter::once(root.path().join("bin"))
			.chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
	)?;
	let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["release-notes", "1.3.0", "notes.md", "--root"])
		.arg(root.path())
		.env("PATH", paths)
		.env_remove("OMP_RELEASE_NOTES_FLOOR")
		.output()?;
	assert!(!result.status.success());
	assert!(
		String::from_utf8_lossy(&result.stderr).contains("refusing to omit unpublished tag changes")
	);
	assert_eq!(fs.read(Path::new("notes.md"), 1024)?, b"previous notes");
	Ok(())
}

#[cfg(unix)]
#[test]
fn note_output_through_an_escaping_parent_cannot_replace_external_files() -> Result<()> {
	let root = tempdir()?;
	let outside = tempdir()?;
	std::fs::write(outside.path().join("notes.md"), b"previous release notes")?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["release-notes", "1.2.3-canary.1", "escape/notes.md", "--root"])
		.arg(root.path())
		.output()?;
	assert!(!result.status.success());
	assert_eq!(std::fs::read(outside.path().join("notes.md"))?, b"previous release notes");
	Ok(())
}
