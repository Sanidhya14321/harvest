//! Curated notes work without JS runtimes and never replace output on failure.
use std::{
	fs,
	path::Path,
	process::{Command, Output},
};

use anyhow::Result;

fn run(root: &Path, args: &[&str]) -> Result<Output> {
	Ok(Command::new(env!("CARGO_BIN_EXE_xtask"))
		.current_dir(root)
		.env("PATH", "")
		.env_remove("OMP_RELEASE_NOTES_FLOOR")
		.env_remove("HARVEST_RELEASE_NOTES_FLOOR")
		.args(["release-notes"])
		.args(args)
		.output()?)
}

fn workspace(root: &Path) -> Result<()> {
	fs::create_dir_all(root.join("packages/a"))?;
	fs::write(root.join("packages/a/package.json"), r#"{"name":"@harvest/a"}"#)?;
	fs::write(
		root.join("packages/a/CHANGELOG.md"),
		"## [Unreleased]\n### Added\n- Future work\n## [1.4.0]\n### Fixed\n- Current fix\n## \
		 [1.3.0]\n### Added\n- Silent addition\n## [1.2.0]\n### Removed\n- Prior release\n",
	)?;
	fs::create_dir_all(root.join("rust-agent"))?;
	fs::write(root.join("rust-agent/Cargo.toml"), "[package]\nname = 'harvest-omp'\n")?;
	fs::write(root.join("rust-agent/CHANGELOG.md"), "## [1.4.0]\n### Added\n- Native feature\n")?;
	fs::create_dir_all(root.join("crates/runtime"))?;
	fs::write(root.join("crates/runtime/Cargo.toml"), "[package]\nname = 'harvest-runtime'\n")?;
	fs::write(root.join("crates/runtime/CHANGELOG.md"), "## [1.3.0]\n### Fixed\n- Runtime fix\n")?;
	fs::write(
		root.join("published.json"),
		r#"[{"tagName":"v1.2.0"},{"tagName":"v1.3.0","isDraft":true}]"#,
	)?;
	Ok(())
}

#[test]
fn native_and_legacy_package_notes_include_silent_tags_without_runtimes() -> Result<()> {
	let root = tempfile::tempdir()?;
	workspace(root.path())?;
	let output =
		run(root.path(), &["v1.4.0", "notes.md", "--published-releases", "published.json"])?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert_eq!(
		fs::read_to_string(root.path().join("notes.md"))?,
		"## harvest-runtime\n\n### Fixed\n\n- Runtime fix\n\n## @harvest/a\n\n### Added\n\n- Silent \
		 addition\n\n### Fixed\n\n- Current fix\n\n## harvest-omp\n\n### Added\n\n- Native feature\n"
	);
	let output = run(root.path(), &["1.4.0", "notes.md", "--single-version"])?;
	assert!(output.status.success());
	let notes = fs::read_to_string(root.path().join("notes.md"))?;
	assert!(!notes.contains("Silent addition") && !notes.contains("Runtime fix"));
	assert!(notes.contains("Current fix") && notes.contains("Native feature"));
	Ok(())
}

#[test]
fn canary_and_empty_notes_do_not_query_github() -> Result<()> {
	let root = tempfile::tempdir()?;
	workspace(root.path())?;
	assert!(
		run(root.path(), &["1.5.0-canary.4", "notes.md"])?
			.status
			.success()
	);
	assert_eq!(
		fs::read_to_string(root.path().join("notes.md"))?,
		"This is a canary prerelease of 1.5.0. Install it with `omp update --canary`.\n"
	);
	assert!(
		run(root.path(), &["1.5.0", "notes.md", "--single-version"])?
			.status
			.success()
	);
	assert_eq!(fs::read(root.path().join("notes.md"))?, b"");
	Ok(())
}

#[test]
fn bad_floor_lookup_and_inputs_preserve_previous_notes_and_source_files() -> Result<()> {
	let root = tempfile::tempdir()?;
	workspace(root.path())?;
	fs::write(root.path().join("notes.md"), "previous release")?;
	fs::write(root.path().join("invalid.json"), "{}")?;
	for args in [
		vec!["1.4.0", "notes.md"],
		vec!["1.4.0", "notes.md", "--floor", "not-a-version"],
		vec!["1.4.0", "notes.md", "--published-releases", "invalid.json"],
		vec!["1.4.0", "notes.md", "--published-releases", "missing.json"],
		vec!["1.4.0", "notes.md", "--floor", "1.2.0", "--single-version"],
		vec!["1.4.0-rc.1", "notes.md", "--single-version"],
		vec!["1.4.0+build", "notes.md", "--single-version"],
	] {
		assert!(!run(root.path(), &args)?.status.success(), "accepted {args:?}");
		assert_eq!(fs::read_to_string(root.path().join("notes.md"))?, "previous release");
	}
	let changelog = fs::read(root.path().join("packages/a/CHANGELOG.md"))?;
	assert!(
		!run(root.path(), &["1.4.0", "packages/a/CHANGELOG.md", "--single-version"])?
			.status
			.success()
	);
	assert_eq!(fs::read(root.path().join("packages/a/CHANGELOG.md"))?, changelog);
	let snapshot = fs::read(root.path().join("published.json"))?;
	assert!(
		!run(root.path(), &["1.4.0", "published.json", "--published-releases", "published.json"])?
			.status
			.success()
	);
	assert_eq!(fs::read(root.path().join("published.json"))?, snapshot);
	Ok(())
}

#[cfg(unix)]
#[test]
fn escaping_changelogs_and_output_paths_cannot_disclose_or_replace_outside_files() -> Result<()> {
	let root = tempfile::tempdir()?;
	let outside = tempfile::tempdir()?;
	workspace(root.path())?;
	fs::write(outside.path().join("private"), "private data")?;
	fs::write(root.path().join("notes.md"), "previous release")?;
	fs::remove_file(root.path().join("packages/a/CHANGELOG.md"))?;
	std::os::unix::fs::symlink(
		outside.path().join("private"),
		root.path().join("packages/a/CHANGELOG.md"),
	)?;
	assert!(
		!run(root.path(), &["1.4.0", "notes.md", "--single-version"])?
			.status
			.success()
	);
	assert_eq!(fs::read_to_string(root.path().join("notes.md"))?, "previous release");
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	assert!(
		!run(root.path(), &["1.4.0-canary.1", "escape/private"])?
			.status
			.success()
	);
	assert_eq!(fs::read_to_string(outside.path().join("private"))?, "private data");
	Ok(())
}

#[cfg(unix)]
#[test]
fn github_lookup_uses_the_selected_repo_and_cannot_silently_fall_back_on_failure() -> Result<()> {
	use std::os::unix::fs::PermissionsExt;
	let root = tempfile::tempdir()?;
	workspace(root.path())?;
	fs::create_dir(root.path().join("bin"))?;
	let gh = root.path().join("bin/gh");
	fs::write(
		&gh,
		"#!/bin/sh\n[ \"$*\" = 'release list --repo fixture/repo --limit 200 --exclude-drafts \
		 --exclude-pre-releases --json tagName,isDraft,isPrerelease' ] || exit 7\nprintf '%s' \
		 '[{\"tagName\":\"v1.2.0\"}]'\n",
	)?;
	fs::set_permissions(&gh, fs::Permissions::from_mode(0o755))?;
	let invoke = || {
		Command::new(env!("CARGO_BIN_EXE_xtask"))
			.current_dir(root.path())
			.env("PATH", root.path().join("bin"))
			.env_remove("OMP_RELEASE_NOTES_FLOOR")
			.env_remove("HARVEST_RELEASE_NOTES_FLOOR")
			.args(["release-notes", "1.4.0", "notes.md", "--repo", "fixture/repo"])
			.output()
	};
	let output = invoke()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let notes = fs::read(root.path().join("notes.md"))?;
	assert!(std::str::from_utf8(&notes)?.contains("Silent addition"));
	fs::write(&gh, "#!/bin/sh\nprintf '%s' 'credential-bearing server diagnostic' >&2\nexit 7\n")?;
	let output = invoke()?;
	assert!(!output.status.success());
	assert!(!String::from_utf8_lossy(&output.stderr).contains("credential-bearing"));
	assert_eq!(fs::read(root.path().join("notes.md"))?, notes);
	Ok(())
}

#[test]
fn environment_version_and_legacy_floor_override_work_without_github() -> Result<()> {
	let root = tempfile::tempdir()?;
	workspace(root.path())?;
	for (floor, expected) in [("v1.2.0", true), ("", false)] {
		let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
			.current_dir(root.path())
			.env("PATH", "")
			.env("GITHUB_REF_NAME", "v1.4.0")
			.env("OMP_RELEASE_NOTES_FLOOR", floor)
			.env_remove("HARVEST_RELEASE_NOTES_FLOOR")
			.arg("release-notes")
			.output()?;
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		assert_eq!(
			fs::read_to_string(root.path().join("release-notes.md"))?.contains("Silent addition"),
			expected
		);
	}
	Ok(())
}
