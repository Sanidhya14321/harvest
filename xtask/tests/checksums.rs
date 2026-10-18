use std::{fs, path::PathBuf, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use tempfile::tempdir;

#[test]
fn checksum_cli_streams_sorted_download_compatible_digests() -> Result<()> {
	let root = tempdir()?;
	fs::write(root.path().join("omp-a"), b"abc")?;
	fs::write(root.path().join("omp-z"), b"")?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.args(["checksums", "--root"])
		.arg(root.path())
		.args(["SHA256SUMS.txt", "omp-z", "omp-a"])
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert_eq!(
		fs::read_to_string(root.path().join("SHA256SUMS.txt"))?,
		"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  \
		 omp-a\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  omp-z\n"
	);
	Ok(())
}

#[test]
fn invalid_asset_sets_preserve_the_existing_manifest() -> Result<()> {
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(std::path::Path::new("SHA256SUMS.txt"), b"previous release\n")?;
	fs.atomic_write(std::path::Path::new("one/omp-a"), b"a")?;
	fs.atomic_write(std::path::Path::new("two/omp-a"), b"b")?;
	for (assets, require_platforms, reason) in [
		(vec![PathBuf::from("one/omp-a"), PathBuf::from("two/omp-a")], false, "duplicate"),
		(vec![PathBuf::from("one/omp-a")], true, "missing release asset"),
		(vec![PathBuf::from("../outside")], false, "escapes"),
		(vec![PathBuf::from("SHA256SUMS.txt")], false, "output cannot"),
	] {
		let error = xtask::checksums::generate(
			&fs,
			std::path::Path::new("SHA256SUMS.txt"),
			&assets,
			require_platforms,
		)
		.expect_err("invalid manifest input");
		assert!(error.to_string().contains(reason), "{error}");
		assert_eq!(fs.read(std::path::Path::new("SHA256SUMS.txt"), 1024)?, b"previous release\n");
	}
	Ok(())
}

#[cfg(unix)]
#[test]
fn escaped_symlinked_output_cannot_overwrite_an_external_file() -> Result<()> {
	let root = tempdir()?;
	let outside = tempdir()?;
	fs::write(root.path().join("omp-a"), b"a")?;
	fs::write(outside.path().join("SHA256SUMS.txt"), b"protected")?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	let jailed = JailedFs::open(root.path())?;
	assert!(
		xtask::checksums::generate(
			&jailed,
			std::path::Path::new("escape/SHA256SUMS.txt"),
			&[PathBuf::from("omp-a")],
			false
		)
		.is_err()
	);
	assert_eq!(fs::read(outside.path().join("SHA256SUMS.txt"))?, b"protected");
	Ok(())
}
