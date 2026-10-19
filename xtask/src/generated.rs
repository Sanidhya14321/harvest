//! Bounded, read-only drift checks and atomic publication for generated files.
use std::path::Path;

use anyhow::{Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};

pub(crate) fn publish(
	fs: &JailedFs,
	path: &Path,
	bytes: &[u8],
	check: bool,
	command: &str,
) -> Result<bool> {
	if bytes.len() > MAX_FILE_BYTES {
		bail!("generated file exceeds its byte limit");
	}
	if fs.read_optional(path, MAX_FILE_BYTES)?.as_deref() == Some(bytes) {
		return Ok(false);
	}
	if check {
		bail!("{} is stale; run {command} and commit the result", path.display());
	}
	fs.atomic_write(path, bytes)?;
	Ok(true)
}
