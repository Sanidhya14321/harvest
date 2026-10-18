//! Installed-command contract: portable manifests and fail-closed publication.
use std::{fs, process::Command};

use anyhow::Result;

#[test]
fn release_manifest_works_without_guest_runtimes_and_failures_preserve_it() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::write(root.path().join("omp-z"), b"")?;
	fs::write(root.path().join("omp-a"), b"abc")?;
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.current_dir(root.path())
		.env("PATH", "")
		.args(["checksums", "SHA256SUMS", "omp-z", "omp-a"])
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let manifest = fs::read(root.path().join("SHA256SUMS"))?;
	assert_eq!(manifest, b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  omp-a\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  omp-z\n");
	for args in [vec!["checksums", "SHA256SUMS"], vec!["checksums", "SHA256SUMS", "missing"], vec![
		"checksums",
		"SHA256SUMS",
		"omp-a",
		"omp-a",
	]] {
		let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
			.current_dir(root.path())
			.env("PATH", "")
			.args(args)
			.output()?;
		assert!(!output.status.success());
		assert_eq!(fs::read(root.path().join("SHA256SUMS"))?, manifest);
	}
	Ok(())
}
