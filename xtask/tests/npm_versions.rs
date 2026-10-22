use std::{fs, path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

fn fixture(private_version: &str) -> Result<TempDir> {
	let root = tempdir()?;
	let jailed = JailedFs::open(root.path())?;
	for (path, value) in [
		(
			"package.json",
			json!({"private":true,"workspaces":{"catalog":{"@harvest/core":"0.1.0","external":"^9"}},"note":"@harvest/core: 0.1.0"}),
		),
		(
			"packages/core/package.json",
			json!({"name":"@harvest/core","version":"0.1.0","exports":{".":"./index.js"}}),
		),
		(
			"packages/app/package.json",
			json!({"name":"@harvest/app","version":"0.1.0","dependencies":{"@harvest/core":"^0.0.1","external":"^9"},"devDependencies":{"@harvest/core":"0.0.1"}}),
		),
		(
			"packages/viewer/package.json",
			json!({"name":"@harvest/viewer","private":true,"version":private_version,"dependencies":{"@harvest/core":"catalog:","@harvest/app":"workspace:*"}}),
		),
	] {
		jailed.atomic_write(Path::new(path), &serde_json::to_vec(&value)?)?;
	}
	Ok(root)
}

fn read(root: &TempDir, path: &str) -> Result<Value> {
	Ok(serde_json::from_slice(&fs::read(root.path().join(path))?)?)
}

#[test]
fn strict_sync_matches_legacy_internal_dependency_rewrites_and_is_idempotent() -> Result<()> {
	let root = fixture("0.1.0")?;
	let jailed = JailedFs::open(root.path())?;
	assert_eq!(xtask::npm_versions::synchronize(&jailed, None, false)?, 2);
	let captured: Value = serde_json::from_str(include_str!("fixtures/npm_versions.json"))?;
	for (path, expected) in captured["outputs"].as_object().unwrap() {
		assert_eq!(read(&root, path)?, *expected, "legacy transformation differs for {path}");
	}
	assert_eq!(xtask::npm_versions::synchronize(&jailed, None, true)?, 0);
	Ok(())
}

#[test]
fn release_cli_updates_catalog_and_explicit_links_without_bumping_private_packages() -> Result<()> {
	let root = fixture("16.3.6")?;
	let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["npm-versions", "--release", "0.2.0-canary.1", "--root"])
		.arg(root.path())
		.env("PATH", root.path())
		.output()?;
	assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
	let app = read(&root, "packages/app/package.json")?;
	assert_eq!(app["version"], "0.2.0-canary.1");
	fs::write(
		root.path().join("Cargo.toml"),
		"[workspace.package]\nversion = \"0.2.0-canary.1\"\n",
	)?;
	let checked = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["npm-versions", "--check", "--workspace", "--root"])
		.arg(root.path())
		.env("PATH", root.path())
		.output()?;
	assert!(checked.status.success(), "{}", String::from_utf8_lossy(&checked.stderr));
	assert_eq!(app["dependencies"]["@harvest/core"], "^0.2.0-canary.1");
	assert_eq!(app["devDependencies"]["@harvest/core"], "^0.2.0-canary.1");
	assert_eq!(app["dependencies"]["external"], "^9");
	let viewer = read(&root, "packages/viewer/package.json")?;
	assert_eq!(viewer["version"], "16.3.6");
	assert_eq!(
		viewer["dependencies"],
		json!({"@harvest/core":"catalog:","@harvest/app":"workspace:*"})
	);
	let catalog = read(&root, "package.json")?;
	assert_eq!(
		catalog["workspaces"]["catalog"],
		json!({"@harvest/core":"0.2.0-canary.1","external":"^9"})
	);
	assert_eq!(catalog["note"], "@harvest/core: 0.1.0");
	assert_eq!(
		xtask::npm_versions::synchronize(
			&JailedFs::open(root.path())?,
			Some("0.2.0-canary.1"),
			true
		)?,
		0
	);
	Ok(())
}

#[test]
fn drift_and_invalid_late_manifest_fail_without_partial_updates() -> Result<()> {
	let root = fixture("16.3.6")?;
	let paths = [
		"package.json",
		"packages/app/package.json",
		"packages/core/package.json",
		"packages/viewer/package.json",
	];
	let originals: Vec<_> = paths
		.iter()
		.map(|path| fs::read(root.path().join(path)))
		.collect::<std::io::Result<_>>()?;
	let jailed = JailedFs::open(root.path())?;
	assert!(xtask::npm_versions::synchronize(&jailed, None, false).is_err());
	assert!(xtask::npm_versions::synchronize(&jailed, Some("v1.2.3"), false).is_err());
	assert!(xtask::npm_versions::synchronize(&jailed, Some("0.2.0"), true).is_err());
	jailed.atomic_write(
		Path::new("packages/zzz/package.json"),
		br#"{"name":"invalid","version":"oops"}"#,
	)?;
	assert!(xtask::npm_versions::synchronize(&jailed, Some("0.2.0"), false).is_err());
	for (path, original) in paths.iter().zip(originals) {
		assert_eq!(fs::read(root.path().join(path))?, original);
	}
	Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_manifests_cannot_escape_or_silently_omit_a_release_package() -> Result<()> {
	let root = fixture("0.1.0")?;
	let outside = tempdir()?;
	let protected = br#"{"name":"@harvest/escaped","version":"0.1.0"}"#;
	fs::write(outside.path().join("package.json"), protected)?;
	let path = root.path().join("packages/viewer/package.json");
	fs::remove_file(&path)?;
	std::os::unix::fs::symlink(outside.path().join("package.json"), path)?;
	let original = fs::read(root.path().join("packages/core/package.json"))?;
	assert!(
		xtask::npm_versions::synchronize(&JailedFs::open(root.path())?, Some("0.2.0"), false)
			.is_err()
	);
	assert_eq!(fs::read(outside.path().join("package.json"))?, protected);
	assert_eq!(fs::read(root.path().join("packages/core/package.json"))?, original);
	Ok(())
}
