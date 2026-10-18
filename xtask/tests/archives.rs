use std::{fs, io::Read, path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn installer_archives_extract_both_executable_aliases_for_each_target() -> Result<()> {
	let root = tempdir()?;
	fs::create_dir(root.path().join("dist"))?;
	let mut originals = Vec::new();
	for asset in xtask::checksums::PLATFORM_ASSETS {
		fs::write(root.path().join("dist").join(asset), format!("binary: {asset}"))?;
	}
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.current_dir(root.path())
		.args(["package", "archives", "--assets-dir", "dist"])
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	for asset in xtask::checksums::PLATFORM_ASSETS {
		let path = root
			.path()
			.join("dist")
			.join(format!("{}.zip", asset.trim_end_matches(".exe")));
		let first = fs::read(&path)?;
		let mut archive = ZipArchive::new(fs::File::open(&path)?)?;
		assert_eq!(archive.len(), 2);
		let extension = if Path::new(asset)
			.extension()
			.is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
		{
			".exe"
		} else {
			""
		};
		for alias in ["omp", "harvest"] {
			let mut entry = archive.by_name(&format!("{alias}{extension}"))?;
			assert_eq!(entry.unix_mode().unwrap_or_default() & 0o777, 0o755);
			let mut body = String::new();
			entry.read_to_string(&mut body)?;
			assert_eq!(body, format!("binary: {asset}"));
		}
		originals.push((path, first));
	}
	// Repeated preparation must yield the same downloadable bytes.
	xtask::packaging::archives(&JailedFs::open(root.path())?, Path::new("dist"))?;
	for (path, first) in originals {
		assert_eq!(fs::read(path)?, first);
	}
	Ok(())
}

#[test]
fn failed_archive_producer_preserves_the_previous_payload() -> Result<()> {
	let root = tempdir()?;
	fs::create_dir(root.path().join("dist"))?;
	fs::write(root.path().join("dist/omp-linux-x64"), b"")?;
	fs::write(root.path().join("dist/omp-linux-x64.zip"), b"previous release")?;
	let jailed = JailedFs::open(root.path())?;
	assert!(xtask::packaging::archives(&jailed, Path::new("dist")).is_err());
	assert_eq!(fs::read(root.path().join("dist/omp-linux-x64.zip"))?, b"previous release");
	assert_eq!(fs::read_dir(root.path().join("dist"))?.count(), 2);
	Ok(())
}

#[cfg(unix)]
#[test]
fn escaped_archive_source_cannot_be_packaged() -> Result<()> {
	let root = tempdir()?;
	let outside = tempdir()?;
	fs::create_dir(root.path().join("dist"))?;
	fs::write(outside.path().join("binary"), b"outside")?;
	std::os::unix::fs::symlink(
		outside.path().join("binary"),
		root.path().join("dist/omp-linux-x64"),
	)?;
	let jailed = JailedFs::open(root.path())?;
	assert!(xtask::packaging::archives(&jailed, Path::new("dist")).is_err());
	assert!(!root.path().join("dist/omp-linux-x64.zip").exists());
	Ok(())
}
