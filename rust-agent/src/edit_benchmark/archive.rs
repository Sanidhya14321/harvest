//! Native fixture materialization. Archive contents remain inert input data;
//! never use tar's ambient unpack API or execute archived source files.
use std::{
	collections::{BTreeMap, BTreeSet},
	io::{self, Read},
	path::{Component, Path, PathBuf},
};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken, read_bounded};
use serde::Serialize;

const MAX_COMPRESSED: u64 = 32 * 1024 * 1024;
const MAX_DECOMPRESSED: u64 = 256 * 1024 * 1024;
const MAX_PAYLOAD: u64 = 128 * 1024 * 1024;
struct BoundedReader<'a, R> {
	inner:        R,
	remaining:    u64,
	cancellation: &'a CancelToken,
}
impl<R: Read> Read for BoundedReader<'_, R> {
	fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
		if output.is_empty() {
			return Ok(0);
		}
		self.cancellation.heartbeat().map_err(io::Error::other)?;
		if self.remaining == 0 {
			let mut probe = [0];
			return if self.inner.read(&mut probe)? == 0 {
				Ok(0)
			} else {
				Err(io::Error::other("archive exceeds byte budget"))
			};
		}
		let length = output.len().min(self.remaining as usize);
		let count = self.inner.read(&mut output[..length])?;
		self.remaining -= count as u64;
		Ok(count)
	}
}
#[derive(Debug, Serialize)]
pub struct ArchiveReport {
	pub directory:     PathBuf,
	pub files:         usize,
	pub payload_bytes: u64,
}
/// Keeps automatic temporary materialization alive for all SDK/CLI consumers.
pub struct FixtureSource {
	directory:  PathBuf,
	_temporary: Option<tempfile::TempDir>,
}
impl FixtureSource {
	pub fn open(source: &Path, cancellation: &CancelToken) -> Result<Self> {
		if source.is_dir() {
			return Ok(Self { directory: source.canonicalize()?, _temporary: None });
		}
		let temporary = tempfile::Builder::new()
			.prefix("harvest-benchmark-")
			.tempdir()?;
		let report = unpack_fixtures(source, temporary.path(), cancellation)?;
		Ok(Self { directory: report.directory, _temporary: Some(temporary) })
	}

	pub fn directory(&self) -> &Path {
		&self.directory
	}

	pub fn is_temporary(&self) -> bool {
		self._temporary.is_some()
	}
}
fn normalized(path: &Path) -> Result<PathBuf> {
	ensure!(path.as_os_str().len() <= 4096, "archive path exceeds 4096-byte limit");
	let mut components = Vec::new();
	for component in path.components() {
		match component {
			Component::Normal(name) => {
				let name = name
					.to_str()
					.ok_or_else(|| anyhow::anyhow!("archive paths must be UTF-8"))?;
				ensure!(
					!name.contains(['\\', ':']) && !name.ends_with(['.', ' ']),
					"archive path is not portable"
				);
				ensure!(name.len() <= 255, "archive path component exceeds limit");
				let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
				let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
					|| base
						.strip_prefix("COM")
						.or_else(|| base.strip_prefix("LPT"))
						.is_some_and(|s| s.len() == 1 && s.bytes().all(|c| c.is_ascii_digit()));
				ensure!(!reserved, "archive contains reserved device path");
				components.push(name.to_owned());
			},
			Component::CurDir => {},
			_ => anyhow::bail!("archive path escapes fixture root"),
		}
	}
	if components.first().is_some_and(|s| s == "fixtures") {
		components.remove(0);
	}
	ensure!(components.len() <= 64, "archive path nesting exceeds limit");
	Ok(components.iter().collect())
}
/// Explicit materialization retains a partial destination on failure. Retry
/// into a fresh empty directory. Automatic FixtureSource cleanup is RAII-owned.
pub fn unpack_fixtures(
	source: &Path,
	output: &Path,
	cancellation: &CancelToken,
) -> Result<ArchiveReport> {
	cancellation.heartbeat()?;
	let absolute = source.canonicalize()?;
	let parent = absolute
		.parent()
		.ok_or_else(|| anyhow::anyhow!("archive has no parent"))?;
	let fs = JailedFs::open(parent)?;
	let filename = absolute
		.file_name()
		.ok_or_else(|| anyhow::anyhow!("archive has no filename"))?;
	let input = fs.open_read(Path::new(filename), MAX_COMPRESSED)?;
	let output = JailedFs::open_or_create(output)?;
	ensure!(!absolute.starts_with(output.root()), "archive must be outside extraction output");
	let _owner = output.resource_lock(Path::new(".archive-owner"))?;
	ensure!(
		output
			.entries(Path::new(""), 4)?
			.iter()
			.all(|e| e.name == ".archive-owner.lock"),
		"archive output must be empty"
	);
	let destination = JailedFs::open_or_create(&output.root().join("fixtures"))?;
	let compressed = BoundedReader { inner: input, remaining: MAX_COMPRESSED, cancellation };
	let gzip = flate2::read::MultiGzDecoder::new(compressed);
	let decompressed = BoundedReader { inner: gzip, remaining: MAX_DECOMPRESSED, cancellation };
	let mut archive = tar::Archive::new(decompressed);
	let mut names = BTreeSet::new();
	let mut aliases = BTreeSet::new();
	let mut prefixes = BTreeMap::new();
	let mut files = 0;
	let mut payload_bytes = 0_u64;
	let mut entry_count = 0;
	for entry in archive.entries()? {
		cancellation.heartbeat()?;
		let mut entry = entry?;
		entry_count += 1;
		ensure!(entry_count <= 10_000, "archive entry count exceeds limit");
		let path = normalized(&entry.path()?)?;
		let kind = entry.header().entry_type();
		ensure!(
			kind.is_file() || kind.is_dir(),
			"archive links, sparse files and special entries are forbidden"
		);
		if path.as_os_str().is_empty() {
			ensure!(kind.is_dir(), "archive file has empty path");
			continue;
		}
		let mut prefix = PathBuf::new();
		for component in path.components() {
			prefix.push(component.as_os_str());
			let alias = prefix.to_string_lossy().to_lowercase();
			if let Some(previous) = prefixes.insert(alias, prefix.clone()) {
				ensure!(previous == prefix, "archive contains case-alias directory prefixes");
			}
		}
		let alias = path.to_string_lossy().to_lowercase();
		ensure!(
			names.insert(path.clone()) && aliases.insert(alias),
			"archive contains duplicate or case-alias paths"
		);
		if kind.is_dir() {
			ensure!(entry.size() == 0, "archive directory has payload");
			let _directory = JailedFs::open_or_create(&destination.root().join(path))?;
			continue;
		}
		let size = entry.size();
		ensure!(size <= MAX_FILE_BYTES as u64, "archive file exceeds limit");
		payload_bytes = payload_bytes
			.checked_add(size)
			.ok_or_else(|| anyhow::anyhow!("archive size overflow"))?;
		ensure!(payload_bytes <= MAX_PAYLOAD, "archive payload exceeds limit");
		let bytes = read_bounded(&mut entry, MAX_FILE_BYTES)?;
		ensure!(bytes.len() as u64 == size, "archive entry is truncated");
		destination.atomic_create(&path, &bytes)?;
		files += 1;
	}
	// Force gzip CRC/footer verification and reject concatenated non-padding
	// payload after the tar terminator. Never publish unvalidated trailing data.
	let mut reader = archive.into_inner();
	let mut padding = [0; 8192];
	loop {
		let count = reader.read(&mut padding)?;
		if count == 0 {
			break;
		}
		ensure!(
			padding[..count].iter().all(|&byte| byte == 0),
			"archive contains data after its terminator"
		);
	}
	ensure!(files > 0, "fixture archive is empty");
	let tasks = super::load_tasks(destination.root())?;
	ensure!(!tasks.is_empty(), "archive contains no benchmark tasks");
	Ok(ArchiveReport { directory: destination.root().to_owned(), files, payload_bytes })
}
