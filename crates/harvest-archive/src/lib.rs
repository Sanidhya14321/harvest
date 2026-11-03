//! Read-only archive browsing shared by the Rust agent: format recognition and
//! `archive.ext:inner/path` splitting for every legacy extension, an indexed
//! reader with synthesized parent directories and case-insensitive listings,
//! and readers for ZIP, tar, gzip/zstd tars and single-stream gzip/zstd.
//! Other recognized formats fail with an explicit unsupported error.
use std::{
	collections::BTreeMap,
	fmt,
	io::{Cursor, Read},
	path::Path,
};

/// Archive failure with a caller-facing message (`ArchiveError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveError(pub String);

impl fmt::Display for ArchiveError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl std::error::Error for ArchiveError {}

pub type Result<T> = std::result::Result<T, ArchiveError>;

fn error(message: impl Into<String>) -> ArchiveError {
	ArchiveError(message.into())
}

/// `DEFAULT_ARCHIVE_LIMITS`.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
	pub max_entries:        usize,
	pub max_in_memory_size: u64,
	pub max_member_size:    u64,
	pub max_path_bytes:     usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			max_entries:        1_000_000,
			max_in_memory_size: 256 * 1024 * 1024,
			max_member_size:    64 * 1024 * 1024,
			max_path_bytes:     4096,
		}
	}
}

/// `FORMAT_EXTENSIONS`, flattened and sorted longest first.
const EXTENSIONS: &[(&str, &str)] = &[
	("tar.bz2", "tar.bz2"),
	("tar.zst", "tar.zst"),
	("tar.gz", "tar.gz"),
	("tar.xz", "tar.xz"),
	("nupkg", "zip"),
	("tar.z", "tar.Z"),
	("asar", "asar"),
	("cpio", "cpio"),
	("lzma", "lzma"),
	("tbz2", "tar.bz2"),
	("tzst", "tar.zst"),
	("vsix", "zip"),
	("apk", "zip"),
	("arj", "arj"),
	("bz2", "bz2"),
	("cab", "cab"),
	("cbr", "rar"),
	("cbz", "zip"),
	("deb", "deb"),
	("ear", "zip"),
	("ipa", "zip"),
	("iso", "iso"),
	("jar", "zip"),
	("lha", "lzh"),
	("lib", "ar"),
	("lzh", "lzh"),
	("rar", "rar"),
	("rpm", "rpm"),
	("tar", "tar"),
	("tbz", "tar.bz2"),
	("tgz", "tar.gz"),
	("txz", "tar.xz"),
	("war", "zip"),
	("whl", "zip"),
	("xpi", "zip"),
	("zip", "zip"),
	("zst", "zst"),
	("7z", "7z"),
	("ar", "ar"),
	("gz", "gz"),
	("xz", "xz"),
	("a", "ar"),
	("z", "Z"),
];

/// `archiveFormatFromPath`.
pub fn format_from_path(path: &str) -> Option<&'static str> {
	let lower = path.to_lowercase();
	EXTENSIONS
		.iter()
		.find(|(ext, _)| lower.ends_with(&format!(".{ext}")))
		.map(|(_, format)| *format)
}

/// `parseArchivePathCandidates`: every `archive.ext` prefix followed by `:` or
/// the end, longest archive path first.
pub fn path_candidates(path: &str) -> Vec<(String, String)> {
	let normalized = path.replace('\\', "/");
	let lower = normalized.to_lowercase();
	let mut seen = Vec::new();
	let mut out: Vec<(String, String)> = Vec::new();
	let mut search = 0;
	while let Some(dot) = lower[search..].find('.').map(|i| search + i) {
		search = dot + 1;
		if let Some((ext, _)) = EXTENSIONS.iter().find(|(ext, _)| {
			let end = dot + 1 + ext.len();
			lower.get(dot + 1..end) == Some(*ext)
				&& (end == lower.len() || lower.as_bytes()[end] == b':')
		}) {
			let end = dot + 1 + ext.len();
			let archive = path[..end].to_owned();
			let sub = normalized[end..].trim_start_matches(':').to_owned();
			let key = (archive.clone(), sub.clone());
			if !seen.contains(&key) {
				seen.push(key.clone());
				out.push(key);
			}
		}
	}
	out.sort_by_key(|candidate| std::cmp::Reverse(candidate.0.len()));
	out
}

/// `normalizeArchiveEntryPath` / `normalizeArchiveLookupPath`.
fn normalize(raw: &str) -> Option<String> {
	let unified = raw.replace('\\', "/");
	let mut parts = Vec::new();
	for part in unified.split('/') {
		match part {
			"" | "." => {},
			".." => return None,
			part => parts.push(part),
		}
	}
	Some(parts.join("/"))
}

#[derive(Debug, Clone)]
struct Entry {
	is_dir: bool,
	size:   u64,
	data:   Option<Storage>,
}

#[derive(Debug, Clone)]
enum Storage {
	Bytes(std::sync::Arc<[u8]>),
	ZipIndex(usize),
}

/// One listed child (`ArchiveDirectoryEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
	pub name:   String,
	pub is_dir: bool,
	pub size:   u64,
}

/// An indexed, read-only archive (`ArchiveReader`).
pub struct Archive {
	entries: BTreeMap<String, Entry>,
	zip:     Option<Vec<u8>>,
	limits:  Limits,
}

impl Archive {
	/// `openArchive` for a file path; the format comes from the extension.
	pub fn open(path: &Path, bytes: Vec<u8>) -> Result<Self> {
		let name = path.to_string_lossy();
		let format = format_from_path(&name).ok_or_else(|| error("Unsupported archive format"))?;
		Self::from_bytes(format, bytes, &name, Limits::default())
	}

	pub fn from_bytes(format: &str, bytes: Vec<u8>, name: &str, limits: Limits) -> Result<Self> {
		if bytes.len() as u64 > limits.max_in_memory_size {
			return Err(error("Archive is too large to read in memory"));
		}
		let mut archive = Self { entries: BTreeMap::new(), zip: None, limits };
		match format {
			"zip" => archive.index_zip(bytes)?,
			"tar" => archive.index_tar(&bytes)?,
			"tar.gz" => archive.index_tar(&decompress_gzip(&bytes, limits)?)?,
			"tar.zst" => archive.index_tar(&decompress_zstd(&bytes, limits)?)?,
			"gz" | "zst" => {
				let data = if format == "gz" {
					decompress_gzip(&bytes, limits)?
				} else {
					decompress_zstd(&bytes, limits)?
				};
				let base = Path::new(name)
					.file_name()
					.map_or_else(|| "data".to_owned(), |n| n.to_string_lossy().into_owned());
				let stem = base
					.rsplit_once('.')
					.map_or("data", |(stem, _)| stem)
					.to_owned();
				archive.insert(stem, false, data.len() as u64, Some(Storage::Bytes(data.into())))?;
			},
			other => {
				return Err(error(format!(
					"Archive format '{other}' is not supported by this host yet"
				)));
			},
		}
		archive.synthesize_parents()?;
		Ok(archive)
	}

	/// `upsertArchiveEntry`: files replace directories; later duplicates win.
	fn insert(&mut self, raw: String, is_dir: bool, size: u64, data: Option<Storage>) -> Result<()> {
		if raw.len() > self.limits.max_path_bytes {
			return Err(error(format!("Archive path exceeds {} bytes", self.limits.max_path_bytes)));
		}
		let Some(path) = normalize(&raw).filter(|path| !path.is_empty()) else {
			return Ok(());
		};
		match self.entries.get(&path) {
			Some(existing) if !existing.is_dir && is_dir => return Ok(()),
			_ => {},
		}
		self.entries.insert(path, Entry { is_dir, size, data });
		if self.entries.len() > self.limits.max_entries {
			return Err(error("Archive has too many entries"));
		}
		Ok(())
	}

	fn synthesize_parents(&mut self) -> Result<()> {
		let paths: Vec<String> = self.entries.keys().cloned().collect();
		for path in paths {
			let mut end = 0;
			while let Some(slash) = path[end..].find('/').map(|i| end + i) {
				let dir = &path[..slash];
				if !self.entries.contains_key(dir) {
					self
						.entries
						.insert(dir.to_owned(), Entry { is_dir: true, size: 0, data: None });
				}
				end = slash + 1;
			}
		}
		if self.entries.len() > self.limits.max_entries {
			return Err(error("Archive has too many entries"));
		}
		Ok(())
	}

	fn index_zip(&mut self, bytes: Vec<u8>) -> Result<()> {
		let mut zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice()))
			.map_err(|e| error(format!("Invalid ZIP archive: {e}")))?;
		for index in 0..zip.len() {
			let file = zip
				.by_index_raw(index)
				.map_err(|e| error(format!("Invalid ZIP archive: {e}")))?;
			let name = file
				.name()
				.map_err(|e| error(format!("Invalid ZIP archive: {e}")))?
				.into_owned();
			let is_dir = name.ends_with('/') || name.ends_with('\\');
			let size = file.size();
			drop(file);
			self.insert(name, is_dir, size, (!is_dir).then_some(Storage::ZipIndex(index)))?;
		}
		self.zip = Some(bytes);
		Ok(())
	}

	/// ustar/GNU/pax tar indexing (member payloads stay in the buffer).
	fn index_tar(&mut self, bytes: &[u8]) -> Result<()> {
		let mut offset = 0;
		let mut long_name: Option<String> = None;
		let mut pax_path: Option<String> = None;
		while offset + 512 <= bytes.len() {
			let header = &bytes[offset..offset + 512];
			if header.iter().all(|&b| b == 0) {
				break;
			}
			let field = |start: usize, len: usize| -> &[u8] {
				let raw = &header[start..start + len];
				let end = raw.iter().position(|&b| b == 0).unwrap_or(len);
				&raw[..end]
			};
			let size = parse_octal(&header[124..136])?;
			let kind = header[156];
			let data_start = offset + 512;
			let data_end = data_start
				.checked_add(size as usize)
				.filter(|end| *end <= bytes.len())
				.ok_or_else(|| error("Truncated tar archive"))?;
			let data = &bytes[data_start..data_end];
			offset = data_start + (size as usize).div_ceil(512) * 512;
			match kind {
				b'L' => {
					long_name = Some(
						String::from_utf8_lossy(data)
							.trim_end_matches('\0')
							.to_owned(),
					);
					continue;
				},
				b'x' => {
					pax_path = parse_pax_path(data);
					continue;
				},
				b'g' => continue,
				_ => {},
			}
			let name = long_name
				.take()
				.or_else(|| pax_path.take())
				.unwrap_or_else(|| {
					let base = String::from_utf8_lossy(field(0, 100)).into_owned();
					let prefix = if &header[257..262] == b"ustar" {
						String::from_utf8_lossy(field(345, 155)).into_owned()
					} else {
						String::new()
					};
					if prefix.is_empty() {
						base
					} else {
						format!("{prefix}/{base}")
					}
				});
			match kind {
				b'5' => self.insert(name, true, 0, None)?,
				b'0' | 0 | b'7' => {
					if size > self.limits.max_member_size {
						return Err(error("Archive member is too large"));
					}
					let is_dir = name.ends_with('/');
					let storage = (!is_dir).then(|| Storage::Bytes(data.into()));
					self.insert(name, is_dir, size, storage)?;
				},
				_ => {},
			}
		}
		Ok(())
	}

	fn lookup(&self, sub: &str) -> Result<Option<(String, Option<&Entry>)>> {
		let path = normalize(sub).ok_or_else(|| error("Archive path cannot contain '..'"))?;
		if path.is_empty() {
			return Ok(Some((path, None)));
		}
		Ok(self.entries.get(&path).map(|entry| (path, Some(entry))))
	}

	/// `getNode`: `Some(is_dir)` when the path exists.
	pub fn node(&self, sub: &str) -> Option<bool> {
		match self.lookup(sub) {
			Ok(Some((_, None))) => Some(true),
			Ok(Some((_, Some(entry)))) => Some(entry.is_dir),
			_ => None,
		}
	}

	/// `listDirectory`, sorted case-insensitively by name.
	pub fn list(&self, sub: &str) -> Result<Vec<DirEntry>> {
		let Some((path, entry)) = self.lookup(sub)? else {
			return Err(error(format!("Archive path '{sub}' not found")));
		};
		if entry.is_some_and(|entry| !entry.is_dir) {
			return Err(error(format!("Archive path '{path}' is not a directory")));
		}
		let prefix = if path.is_empty() {
			String::new()
		} else {
			format!("{path}/")
		};
		let mut children: BTreeMap<String, DirEntry> = BTreeMap::new();
		for (entry_path, entry) in &self.entries {
			let Some(relative) = entry_path.strip_prefix(&prefix) else {
				continue;
			};
			if relative.is_empty() {
				continue;
			}
			let (name, nested) = relative
				.split_once('/')
				.map_or((relative, false), |(name, _)| (name, true));
			children.entry(name.to_owned()).or_insert_with(|| {
				let child = self.entries.get(&format!("{prefix}{name}"));
				let is_dir = nested || child.is_some_and(|c| c.is_dir);
				DirEntry {
					name: name.to_owned(),
					is_dir,
					size: if is_dir {
						0
					} else {
						child.map_or(entry.size, |c| c.size)
					},
				}
			});
		}
		let mut list: Vec<DirEntry> = children.into_values().collect();
		list.sort_by(|a, b| {
			let (a, b) = (a.name.to_lowercase(), b.name.to_lowercase());
			a.cmp(&b)
		});
		Ok(list)
	}

	/// `readFile`: one member's bytes.
	pub fn read(&self, sub: &str) -> Result<Vec<u8>> {
		let Some((path, entry)) = self.lookup(sub)? else {
			return Err(error(format!("Archive file '{sub}' not found")));
		};
		let Some(entry) = entry.filter(|entry| !entry.is_dir) else {
			return Err(error(format!("Archive path '{path}' is a directory")));
		};
		if entry.size > self.limits.max_member_size {
			return Err(error(format!("Archive member '{path}' is too large")));
		}
		match &entry.data {
			Some(Storage::Bytes(bytes)) => Ok(bytes.to_vec()),
			Some(Storage::ZipIndex(index)) => {
				let bytes = self
					.zip
					.as_deref()
					.ok_or_else(|| error("ZIP data unavailable"))?;
				let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
					.map_err(|e| error(format!("Invalid ZIP archive: {e}")))?;
				let file = zip
					.by_index(*index)
					.map_err(|e| error(format!("Cannot read archive member '{path}': {e}")))?;
				let mut out = Vec::with_capacity(entry.size as usize);
				file
					.take(self.limits.max_member_size + 1)
					.read_to_end(&mut out)
					.map_err(|e| error(format!("Cannot read archive member '{path}': {e}")))?;
				if out.len() as u64 != entry.size {
					return Err(error(format!("Archive member '{path}' size mismatch")));
				}
				Ok(out)
			},
			None => Err(error(format!("Archive file '{path}' has no readable storage"))),
		}
	}
}

fn parse_octal(raw: &[u8]) -> Result<u64> {
	if raw.first().is_some_and(|b| b & 0x80 != 0) {
		// GNU base-256.
		return Ok(raw[1..]
			.iter()
			.fold(0u64, |acc, &b| acc.saturating_mul(256).saturating_add(u64::from(b))));
	}
	let text: String = raw
		.iter()
		.take_while(|&&b| b != 0)
		.map(|&b| b as char)
		.collect();
	let text = text.trim();
	if text.is_empty() {
		return Ok(0);
	}
	u64::from_str_radix(text, 8).map_err(|_| error("Invalid tar header"))
}

fn parse_pax_path(data: &[u8]) -> Option<String> {
	let text = String::from_utf8_lossy(data);
	text.lines().find_map(|line| {
		let (_, record) = line.split_once(' ')?;
		record.strip_prefix("path=").map(str::to_owned)
	})
}

fn bounded_read(mut reader: impl Read, limits: Limits) -> Result<Vec<u8>> {
	let mut out = Vec::new();
	(&mut reader)
		.take(limits.max_in_memory_size + 1)
		.read_to_end(&mut out)
		.map_err(|e| error(format!("Invalid compressed archive: {e}")))?;
	if out.len() as u64 > limits.max_in_memory_size {
		return Err(error("Archive is too large to read in memory"));
	}
	Ok(out)
}

fn decompress_gzip(bytes: &[u8], limits: Limits) -> Result<Vec<u8>> {
	bounded_read(flate2::read::MultiGzDecoder::new(bytes), limits)
}

fn decompress_zstd(bytes: &[u8], limits: Limits) -> Result<Vec<u8>> {
	let decoder = zstd::stream::read::Decoder::new(bytes)
		.map_err(|e| error(format!("Invalid compressed archive: {e}")))?;
	bounded_read(decoder, limits)
}

/// `formatArchiveEntryLines` (sizes through the caller's byte formatter).
pub fn format_entry_lines(
	entries: &[DirEntry],
	format_bytes: impl Fn(u64) -> String,
) -> Vec<String> {
	entries
		.iter()
		.map(|entry| {
			if entry.is_dir {
				format!("{}/", entry.name)
			} else if entry.size > 0 {
				format!("{} ({})", entry.name, format_bytes(entry.size))
			} else {
				entry.name.clone()
			}
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use super::*;

	fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
		let mut out = Cursor::new(Vec::new());
		let mut writer = zip::ZipWriter::new(&mut out);
		for (name, body) in files {
			writer
				.start_file(*name, zip::write::SimpleFileOptions::default())
				.unwrap();
			writer.write_all(body.as_bytes()).unwrap();
		}
		writer.finish().unwrap();
		out.into_inner()
	}

	fn tar_bytes(files: &[(&str, &str)]) -> Vec<u8> {
		let mut out = Vec::new();
		for (name, body) in files {
			let mut header = [0u8; 512];
			header[..name.len()].copy_from_slice(name.as_bytes());
			header[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
			header[156] = b'0';
			header[257..262].copy_from_slice(b"ustar");
			out.extend_from_slice(&header);
			out.extend_from_slice(body.as_bytes());
			out.resize(out.len().div_ceil(512) * 512, 0);
		}
		out.resize(out.len() + 1024, 0);
		out
	}

	#[test]
	fn candidates_split_on_archive_extensions_longest_first() {
		assert_eq!(
			path_candidates("a/b.tar.gz:src/x.rs"),
			[("a/b.tar.gz".to_owned(), "src/x.rs".to_owned()), ("a/b.tar".to_owned(), String::new()),]
				.into_iter()
				.filter(|(a, _)| a.ends_with(".tar.gz"))
				.collect::<Vec<_>>()
		);
		assert!(path_candidates("src/main.rs").is_empty());
		assert_eq!(format_from_path("pkg.WHL"), Some("zip"));
	}

	#[test]
	fn zip_and_tar_list_with_synthesized_dirs_and_read_members() -> Result<()> {
		for (format, bytes) in [
			("zip", zip_bytes(&[("src/b.rs", "fn b() {}"), ("A.md", "hi")])),
			("tar", tar_bytes(&[("src/b.rs", "fn b() {}"), ("A.md", "hi")])),
		] {
			let archive = Archive::from_bytes(format, bytes, "x", Limits::default())?;
			let names: Vec<String> = archive.list("")?.into_iter().map(|e| e.name).collect();
			assert_eq!(names, ["A.md", "src"], "{format}");
			assert_eq!(archive.node("src"), Some(true));
			assert_eq!(archive.read("src/b.rs")?, b"fn b() {}");
			assert!(archive.read("../x").unwrap_err().0.contains("'..'"));
			assert!(
				archive
					.read("src")
					.unwrap_err()
					.0
					.contains("is a directory")
			);
		}
		let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
		gz.write_all(&tar_bytes(&[("x.txt", "zz")])).unwrap();
		let archive =
			Archive::from_bytes("tar.gz", gz.finish().unwrap(), "x.tgz", Limits::default())?;
		assert_eq!(archive.read("x.txt")?, b"zz");
		assert!(Archive::from_bytes("rar", Vec::new(), "x.rar", Limits::default()).is_err());
		Ok(())
	}
}
