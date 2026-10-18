use std::{fs, path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use tempfile::{TempDir, tempdir};

fn workspace() -> Result<TempDir> {
	let root = tempdir()?;
	let files = [
		(
			"Cargo.toml",
			"[workspace]\nmembers = [\"core\", \"app\", \"vendor\"]\nresolver = \
			 \"3\"\n[workspace.package]\nversion = \"0.1.0\"\nedition = \
			 \"2024\"\n[workspace.dependencies]\ncore = { path = \"core\" }\n",
		),
		(
			"core/Cargo.toml",
			"[package]\nname = \"core\"\nversion.workspace = true\nedition.workspace = true\n",
		),
		(
			"app/Cargo.toml",
			"[package]\nname = \"app\"\nversion.workspace = true\nedition.workspace = \
			 true\n[dependencies]\ncore.workspace = true\nforeign = { path = \"../vendor\", version \
			 = \"^0.5.0\" }\n[dev-dependencies]\ncore = { path = \"../core\" \
			 }\n[build-dependencies.core]\npath = \
			 \"../core\"\n[target.'cfg(windows)'.dependencies]\ncore = { path = \"../core\", version \
			 = \"0.1.0\" }\n",
		),
		(
			"vendor/Cargo.toml",
			"[package]\nname = \"foreign\"\nversion = \"0.5.0\"\nedition = \"2024\"\n",
		),
		("core/src/lib.rs", ""),
		("app/src/lib.rs", ""),
		("vendor/src/lib.rs", ""),
	];
	let jailed = JailedFs::open(root.path())?;
	for (path, contents) in files {
		jailed.atomic_write(Path::new(path), contents.as_bytes())?;
	}
	Ok(root)
}

#[test]
fn release_bump_resolves_inherited_and_all_dependency_kinds_for_cargo_consumers() -> Result<()> {
	let root = workspace()?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["version", "--root"])
		.arg(root.path())
		.arg("0.2.0-canary.1")
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let metadata = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.current_dir(root.path())
		.args(["metadata", "--offline", "--no-deps", "--format-version", "1"])
		.output()?;
	assert!(metadata.status.success(), "{}", String::from_utf8_lossy(&metadata.stderr));
	let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
	let packages = metadata["packages"].as_array().unwrap();
	for package in packages {
		assert_eq!(
			package["version"],
			if package["name"] == "foreign" {
				"0.5.0"
			} else {
				"0.2.0-canary.1"
			}
		);
		for dependency in package["dependencies"].as_array().unwrap() {
			assert_eq!(
				dependency["req"],
				if dependency["name"] == "foreign" {
					"^0.5.0"
				} else {
					"=0.2.0-canary.1"
				}
			);
		}
	}
	assert_eq!(xtask::versions::synchronize(&JailedFs::open(root.path())?, None, true)?, 0);
	Ok(())
}

#[test]
fn drift_checks_do_not_write_and_invalid_versions_preserve_all_manifests() -> Result<()> {
	let root = workspace()?;
	let paths = ["Cargo.toml", "app/Cargo.toml", "core/Cargo.toml", "vendor/Cargo.toml"];
	let original: Vec<_> = paths
		.iter()
		.map(|path| fs::read(root.path().join(path)))
		.collect::<std::io::Result<_>>()?;
	let jailed = JailedFs::open(root.path())?;
	assert!(
		xtask::versions::synchronize(&jailed, None, true)
			.unwrap_err()
			.to_string()
			.contains("drift")
	);
	assert!(xtask::versions::synchronize(&jailed, Some("v1.2.3"), false).is_err());
	for (path, original) in paths.iter().zip(original) {
		assert_eq!(fs::read(root.path().join(path))?, original);
	}
	Ok(())
}

#[test]
fn unknown_path_dependencies_fail_before_bumping_the_workspace() -> Result<()> {
	let root = workspace()?;
	let original = fs::read(root.path().join("Cargo.toml"))?;
	let jailed = JailedFs::open(root.path())?;
	jailed.append(
		Path::new("app/Cargo.toml"),
		b"unknown = { path = \"../other\" }\n",
		1024 * 1024,
	)?;
	let error = xtask::versions::synchronize(&jailed, Some("0.3.0"), false).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("not a declared workspace member"),
		"{error}"
	);
	assert_eq!(fs::read(root.path().join("Cargo.toml"))?, original);
	Ok(())
}

#[cfg(unix)]
#[test]
fn escaped_manifest_symlinks_cannot_modify_external_packages() -> Result<()> {
	let root = workspace()?;
	let outside = tempdir()?;
	let protected = b"[package]\nname = \"core\"\nversion.workspace = true\n";
	fs::write(outside.path().join("Cargo.toml"), protected)?;
	fs::remove_file(root.path().join("core/Cargo.toml"))?;
	std::os::unix::fs::symlink(
		outside.path().join("Cargo.toml"),
		root.path().join("core/Cargo.toml"),
	)?;
	let original = fs::read(root.path().join("Cargo.toml"))?;
	assert!(
		xtask::versions::synchronize(&JailedFs::open(root.path())?, Some("0.3.0"), false).is_err()
	);
	assert_eq!(fs::read(outside.path().join("Cargo.toml"))?, protected);
	assert_eq!(fs::read(root.path().join("Cargo.toml"))?, original);
	Ok(())
}
