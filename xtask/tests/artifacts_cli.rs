//! Exercise the CI matrix through real packaging and manifest consumers.
use std::{
	fs,
	io::{Cursor, Read},
	path::Path,
	process::Command,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

#[derive(Deserialize)]
struct BuildTarget {
	target:   String,
	artifact: String,
	binary:   String,
}

fn run(root: &Path, args: &[&str]) -> Result<std::process::Output> {
	Ok(Command::new(env!("CARGO_BIN_EXE_xtask"))
		.current_dir(root)
		.env("PATH", "")
		.args(["package"])
		.args(args)
		.output()?)
}

fn matrix(root: &Path) -> Result<Vec<BuildTarget>> {
	let output = run(root, &["matrix"])?;
	assert!(output.status.success());
	let matrix: Value = serde_json::from_slice(&output.stdout)?;
	Ok(serde_json::from_value(matrix["include"].clone())?)
}

fn fixture(root: &Path) -> Result<Vec<BuildTarget>> {
	fs::write(root.join("LICENSE"), "license bytes\n")?;
	fs::write(root.join("THIRD-PARTY-NOTICES.txt"), "native notices\n")?;
	fs::create_dir(root.join("dist"))?;
	let targets = matrix(root)?;
	for target in &targets {
		// Distinct payloads catch swapped architecture/ABI assets.
		fs::write(root.join("dist").join(&target.artifact), target.target.as_bytes())?;
		let output = run(root, &[
			"archive",
			"--target",
			&target.target,
			"--binary",
			&format!("dist/{}", target.artifact),
			"--output-dir",
			"dist",
		])?;
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	}
	Ok(targets)
}

#[test]
fn ci_matrix_packages_every_promised_abi_and_both_installable_aliases() -> Result<()> {
	let root = tempfile::tempdir()?;
	let targets = fixture(root.path())?;
	let mut observed = std::collections::BTreeSet::new();
	for target in targets {
		let name = target
			.artifact
			.strip_suffix(".exe")
			.unwrap_or(&target.artifact)
			.to_owned()
			+ ".zip";
		let bytes = fs::read(root.path().join("dist").join(&name))?;
		let mut archive = ZipArchive::new(Cursor::new(&bytes))?;
		let alias = if target.binary == "omp.exe" {
			"harvest.exe"
		} else {
			"harvest"
		};
		for name in [target.binary.as_str(), alias] {
			let mut file = archive.by_name(name)?;
			assert_eq!(file.unix_mode().unwrap() & 0o777, 0o755);
			assert_eq!(file.last_modified().context("missing ZIP time")?.year(), 1980);
			let mut payload = String::new();
			file.read_to_string(&mut payload)?;
			assert_eq!(payload, target.target);
		}
		let output = run(root.path(), &[
			"archive",
			"--target",
			&target.target,
			"--binary",
			&format!("dist/{}", target.artifact),
			"--output-dir",
			"dist",
		])?;
		assert!(output.status.success());
		assert_eq!(fs::read(root.path().join("dist").join(&name))?, bytes);
		observed.insert(target.artifact);
	}
	// Existing installer ABI names are the shipping contract consumed by CI.
	assert_eq!(
		observed,
		[
			"omp-linux-x64",
			"omp-linux-arm64",
			"omp-linux-musl-x64",
			"omp-linux-musl-arm64",
			"omp-darwin-x64",
			"omp-darwin-arm64",
			"omp-windows-x64.exe",
			"omp-windows-arm64.exe"
		]
		.into_iter()
		.map(str::to_owned)
		.collect()
	);
	let output = run(root.path(), &["verify-assets", "--directory", "dist"])?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	Ok(())
}

#[test]
fn missing_mismatched_and_corrupt_assets_fail_before_release_validation_succeeds() -> Result<()> {
	let root = tempfile::tempdir()?;
	fixture(root.path())?;
	let path = root.path().join("dist/omp-windows-arm64.exe");
	let original = fs::read(&path)?;
	fs::write(&path, b"wrong architecture binary")?;
	let output = run(root.path(), &["verify-assets", "--directory", "dist"])?;
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("does not match"));
	fs::write(&path, &original)?;
	fs::remove_file(&path)?;
	assert!(
		!run(root.path(), &["verify-assets", "--directory", "dist"])?
			.status
			.success()
	);
	fs::write(&path, &original)?;
	let zip = root.path().join("dist/omp-windows-arm64.zip");
	fs::write(&zip, b"corrupt archive")?;
	assert!(
		!run(root.path(), &["verify-assets", "--directory", "dist"])?
			.status
			.success()
	);
	Ok(())
}

#[test]
fn archive_errors_and_symlink_escapes_cannot_overwrite_previous_packages() -> Result<()> {
	let root = tempfile::tempdir()?;
	fixture(root.path())?;
	let output = root.path().join("dist/omp-linux-x64.zip");
	let original = fs::read(&output)?;
	fs::remove_file(root.path().join("THIRD-PARTY-NOTICES.txt"))?;
	assert!(
		!run(root.path(), &[
			"archive",
			"--target",
			"x86_64-unknown-linux-gnu",
			"--binary",
			"dist/omp-linux-x64",
			"--output-dir",
			"dist"
		])?
		.status
		.success()
	);
	assert_eq!(fs::read(&output)?, original);
	assert!(
		!run(root.path(), &[
			"archive",
			"--target",
			"unknown",
			"--binary",
			"dist/omp-linux-x64",
			"--output-dir",
			"dist"
		])?
		.status
		.success()
	);
	assert_eq!(fs::read(&output)?, original);
	#[cfg(unix)]
	{
		let outside = tempfile::tempdir()?;
		fs::write(root.path().join("THIRD-PARTY-NOTICES.txt"), "notices")?;
		std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
		fs::write(outside.path().join("omp-linux-x64.zip"), "external")?;
		assert!(
			!run(root.path(), &[
				"archive",
				"--target",
				"x86_64-unknown-linux-gnu",
				"--binary",
				"dist/omp-linux-x64",
				"--output-dir",
				"escape"
			])?
			.status
			.success()
		);
		assert_eq!(fs::read(outside.path().join("omp-linux-x64.zip"))?, b"external");
	}
	Ok(())
}

#[test]
fn archive_with_wrong_alias_payload_is_rejected_even_when_raw_binary_is_intact() -> Result<()> {
	use std::io::Write;
	let root = tempfile::tempdir()?;
	fixture(root.path())?;
	let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
	for (name, bytes) in [
		("omp.exe", b"aarch64-pc-windows-msvc".as_slice()),
		("harvest.exe", b"wrong alias"),
		("LICENSE", b"license bytes\n"),
		("THIRD-PARTY-NOTICES.txt", b"native notices\n"),
	] {
		writer.start_file(name, SimpleFileOptions::default().unix_permissions(0o755))?;
		writer.write_all(bytes)?;
	}
	fs::write(root.path().join("dist/omp-windows-arm64.zip"), writer.finish()?.into_inner())?;
	let output = run(root.path(), &["verify-assets", "--directory", "dist"])?;
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("alias does not match"));
	Ok(())
}

#[test]
fn conflicting_local_and_central_zip_names_cannot_pass_release_validation() -> Result<()> {
	let root = tempfile::tempdir()?;
	fixture(root.path())?;
	let path = root.path().join("dist/omp-windows-arm64.zip");
	let mut bytes = fs::read(&path)?;
	// Alter only the first local filename. The central directory still advertises
	// omp.exe, so checking decompressed bytes alone would accept the ambiguity.
	let position = bytes
		.windows(7)
		.position(|name| name == b"omp.exe")
		.context("fixture ZIP filename missing")?;
	bytes[position] = b'b';
	fs::write(&path, &bytes)?;
	let output = run(root.path(), &["verify-assets", "--directory", "dist"])?;
	assert!(!output.status.success());
	assert_eq!(fs::read(&path)?, bytes);
	Ok(())
}

fn checksums(root: &Path, manifest: bool) -> Result<()> {
	let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
	command.current_dir(root).env("PATH", "").args([
		"checksums",
		"SHA256SUMS.txt",
		"dist/omp-windows-x64.zip",
		"dist/omp-windows-arm64.zip",
	]);
	if manifest {
		command.arg("dist/omp-winget-manifests.zip");
	}
	let output = command.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	Ok(())
}

fn winget_fixture(root: &Path) -> Result<()> {
	fixture(root)?;
	checksums(root, false)?;
	let output = run(root, &[
		"winget",
		"--version",
		"1.2.3",
		"--release-date",
		"2026-10-18",
		"--checksums",
		"SHA256SUMS.txt",
		"--archive-output",
		"dist/omp-winget-manifests.zip",
	])?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	checksums(root, true)
}

fn verify_winget(root: &Path) -> Result<std::process::Output> {
	run(root, &[
		"verify-winget",
		"--version",
		"1.2.3",
		"--archive",
		"dist/omp-winget-manifests.zip",
		"--checksums",
		"SHA256SUMS.txt",
		"--assets-dir",
		"dist",
		"--output-dir",
		"verified",
	])
}

#[test]
fn published_winget_data_binds_each_architecture_to_its_downloaded_archive() -> Result<()> {
	let root = tempfile::tempdir()?;
	winget_fixture(root.path())?;
	let output = verify_winget(root.path())?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let installer: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&fs::read(
		root.path().join("verified/Pulkitxm.Omp.installer.yaml"),
	)?)?;
	for entry in installer["Installers"]
		.as_sequence()
		.context("no installers")?
	{
		let arch = entry["Architecture"]
			.as_str()
			.context("no installer architecture")?;
		let filename = format!("omp-windows-{arch}.zip");
		let digest =
			harvest_runtime::content_hash(&fs::read(root.path().join("dist").join(&filename))?);
		assert_eq!(entry["InstallerSha256"].as_str(), Some(digest.to_ascii_uppercase().as_str()));
		assert!(
			entry["InstallerUrl"]
				.as_str()
				.unwrap()
				.ends_with(&format!("/v1.2.3/{filename}"))
		);
		let aliases = entry["NestedInstallerFiles"]
			.as_sequence()
			.context("no binary aliases")?;
		assert_eq!(
			aliases
				.iter()
				.map(|file| file["RelativeFilePath"].as_str().unwrap())
				.collect::<Vec<_>>(),
			vec!["omp.exe", "harvest.exe"]
		);
	}
	let before = fs::read(root.path().join("verified/Pulkitxm.Omp.installer.yaml"))?;
	fs::write(root.path().join("dist/omp-windows-arm64.zip"), "tampered Windows binary archive")?;
	assert!(!verify_winget(root.path())?.status.success());
	assert_eq!(fs::read(root.path().join("verified/Pulkitxm.Omp.installer.yaml"))?, before);
	Ok(())
}

#[test]
fn matching_archive_checksums_cannot_authorize_injected_manifest_fields() -> Result<()> {
	use std::io::Write;
	let root = tempfile::tempdir()?;
	winget_fixture(root.path())?;
	let path = root.path().join("dist/omp-winget-manifests.zip");
	let mut archive = ZipArchive::new(Cursor::new(fs::read(&path)?))?;
	let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
	for index in 0..archive.len() {
		let mut file = archive.by_index(index)?;
		let name = file.name()?.into_owned();
		let mut bytes = Vec::new();
		file.read_to_end(&mut bytes)?;
		if name == "Pulkitxm.Omp.installer.yaml" {
			bytes.extend_from_slice(b"InstallerSwitches:\n  Custom: unexpected-executable-argument\n");
		}
		writer.start_file(name, SimpleFileOptions::default().unix_permissions(0o644))?;
		writer.write_all(&bytes)?;
	}
	fs::write(&path, writer.finish()?.into_inner())?;
	checksums(root.path(), true)?;
	let output = verify_winget(root.path())?;
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("does not match"));
	assert!(!root.path().join("verified").exists());
	Ok(())
}

#[test]
fn special_file_modes_cannot_pass_as_executable_archive_entries() -> Result<()> {
	let root = tempfile::tempdir()?;
	fixture(root.path())?;
	let path = root.path().join("dist/omp-windows-arm64.zip");
	let mut bytes = fs::read(&path)?;
	let start = bytes
		.windows(4)
		.position(|bytes| bytes == b"PK\x01\x02")
		.context("missing central ZIP header")?;
	assert_eq!(&bytes[start + 46..start + 53], b"omp.exe");
	// Preserve permissions and payload, but advertise a FIFO in the Unix type
	// bits. Checking only 0755 or `is_file()` would accept this special mode.
	bytes[start + 38..start + 42].copy_from_slice(&(0o010755_u32 << 16).to_le_bytes());
	fs::write(&path, &bytes)?;
	assert!(
		!run(root.path(), &["verify-assets", "--directory", "dist"])?
			.status
			.success()
	);
	assert_eq!(fs::read(&path)?, bytes);
	Ok(())
}
