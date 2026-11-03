use std::{fs, io::Read, path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use tempfile::tempdir;

#[test]
fn rollback_archive_keeps_dirty_sources_and_sealed_artifacts_cannot_be_replaced() -> Result<()> {
	let root = tempdir()?;
	assert!(
		Command::new("git")
			.args(["init", "--quiet"])
			.arg(root.path())
			.status()?
			.success()
	);
	let jailed = JailedFs::open(root.path())?;
	jailed.atomic_write(Path::new(".gitignore"), b"/target/\n")?;
	jailed.atomic_write(Path::new("src/uncommitted.ts"), b"original working application\n")?;
	jailed.atomic_write(Path::new("target/original/omp"), b"supplied binary bytes")?;
	let inventory = xtask::source_audit::inspect_files(
		&jailed,
		b".gitignore\0src/uncommitted.ts\0",
		Path::new("source-audit.json"),
	)?;
	let output = Path::new("target/migration/baseline");
	xtask::baseline::capture(
		&jailed,
		&inventory,
		Path::new("target/original/omp"),
		"omp-linux-x64",
		output,
	)?;
	let original = jailed.read(&output.join("baseline.json"), 8192)?;
	let receipt: serde_json::Value = serde_json::from_slice(&original)?;
	assert_eq!(
		receipt["asset_sha256"],
		jailed.hash_file(&output.join("omp-linux-x64"), 128 * 1024 * 1024)?
	);
	assert_eq!(
		receipt["source_archive_sha256"],
		jailed.hash_file(&output.join("source.zip"), 512 * 1024 * 1024)?
	);
	let mut archive =
		zip::ZipArchive::new(fs::File::open(root.path().join(output).join("source.zip"))?)?;
	let mut bytes = Vec::new();
	archive
		.by_name("source/src/uncommitted.ts")?
		.read_to_end(&mut bytes)?;
	assert_eq!(bytes, b"original working application\n");
	jailed.atomic_write(Path::new("target/original/omp"), b"different release")?;
	assert!(
		xtask::baseline::capture(
			&jailed,
			&inventory,
			Path::new("target/original/omp"),
			"omp-linux-x64",
			output
		)
		.is_err()
	);
	assert_eq!(jailed.read(&output.join("baseline.json"), 8192)?, original);
	assert_eq!(jailed.read(&output.join("omp-linux-x64"), 100)?, b"supplied binary bytes");
	Ok(())
}
