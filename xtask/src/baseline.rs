//! Preserve a source-bound rollback bundle without modifying working sources.
use std::{io::Write, path::Path};

use anyhow::{Result, bail};
use harvest_runtime::{JailedFs, content_hash};
use serde::Serialize;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
	checksums::PLATFORM_ASSETS,
	migration::{Inventory, current_inventory},
};

#[derive(Serialize)]
struct Receipt<'a> {
	schema_version:        u32,
	source_fingerprint:    &'a str,
	source_archive:        &'a str,
	source_archive_sha256: String,
	asset:                 &'a str,
	asset_sha256:          String,
	verification:          &'a str,
}

/// Preserve exact source bytes and an explicitly supplied installable artifact.
/// Integrity is established here; functional/platform validation is separate.
pub fn capture(
	fs: &JailedFs,
	inventory: &Inventory,
	artifact: &Path,
	asset: &str,
	output: &Path,
) -> Result<()> {
	if !PLATFORM_ASSETS.contains(&asset) {
		bail!("unknown rollback artifact platform");
	}
	let artifact = fs.relative(artifact)?;
	let output = fs.relative(output)?;
	if !output.starts_with("target/migration") || artifact.starts_with(&output) {
		bail!("baseline output must be isolated under target/migration and cannot contain its input");
	}
	let source = output.join("source.zip");
	let binary = output.join(asset);
	let lock = fs.lock(&output.join("capture.lock"))?;
	if fs.exists(&output.join("baseline.json"))? {
		bail!("rollback baseline already sealed; choose a new output directory");
	}
	fs.atomic_write_with(&source, |file| {
		let mut zip = ZipWriter::new(file);
		let options = SimpleFileOptions::default()
			.compression_method(CompressionMethod::Deflated)
			.unix_permissions(0o600);
		zip.start_file("inventory.json", options)?;
		zip.write_all(&serde_json::to_vec(inventory)?)?;
		for file in &inventory.files {
			let bytes = fs.read(Path::new(&file.path), 64 * 1024 * 1024)?;
			if content_hash(&bytes) != file.sha256 {
				bail!("source changed during baseline capture: {}", file.path);
			}
			zip.start_file(format!("source/{}", file.path), options)?;
			zip.write_all(&bytes)?;
		}
		zip.finish()?;
		Ok(())
	})?;
	let digest = fs.hash_file(&artifact)?;
	fs.atomic_write_with(&binary, |file| {
		if fs.copy_file_to(&artifact, file, 512 * 1024 * 1024)? == 0 {
			bail!("empty rollback artifact");
		}
		Ok(())
	})?;
	if fs.hash_file(&binary)? != digest
		|| current_inventory(fs)?.source_fingerprint != inventory.source_fingerprint
	{
		bail!("rollback inputs changed before sealing");
	}
	let receipt = Receipt {
		schema_version: 1,
		source_fingerprint: &inventory.source_fingerprint,
		source_archive: "source.zip",
		source_archive_sha256: fs.hash_file(&source)?,
		asset,
		asset_sha256: digest,
		verification: "integrity-only; installed functional, permission and platform verification \
		               still required",
	};
	fs.atomic_write(&output.join("baseline.json"), &serde_json::to_vec_pretty(&receipt)?)?;
	drop(lock);
	Ok(())
}
