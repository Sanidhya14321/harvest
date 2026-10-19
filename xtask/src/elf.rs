//! Repair relocated GNU version-definition pointers without partial
//! publication.
use std::{
	fs::File,
	io::{self, Read, Seek, SeekFrom},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;

const ELF_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
const TABLE_LIMIT: u64 = 4 * 1024 * 1024;
const DT_VERDEF: u64 = 0x6fff_fffc;
const SHT_DYNAMIC: u32 = 6;
const SHT_GNU_VERDEF: u32 = 0x6fff_fffd;

#[derive(Parser)]
#[command(name = "cargo xtask fix-elf-verdef")]
struct Arguments {
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
	#[arg(required=true,num_args=1..)]
	files:     Vec<PathBuf>,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = match Arguments::try_parse_from(
		std::iter::once("fix-elf-verdef".to_owned()).chain(arguments),
	) {
		Ok(args) => args,
		Err(help) if !help.use_stderr() => {
			help.print()?;
			return Ok(());
		},
		Err(error) => return Err(error.into()),
	};
	let fs = JailedFs::open(&args.workspace)?;
	for file in args.files {
		let relative = fs.relative(&file)?;
		let outcome = repair(&fs, &relative)
			.with_context(|| format!("could not repair {}", relative.display()))?;
		match outcome {
			Plan::NoDefinitions => {
				println!("fix-dt-verdef: {}: no .gnu.version_d, nothing to do", relative.display());
			},
			Plan::Current(address) => {
				println!("fix-dt-verdef: {}: DT_VERDEF already 0x{address:x}", relative.display());
			},
			Plan::Patch { old, address, .. } => {
				println!("fix-dt-verdef: {}: DT_VERDEF 0x{old:x} -> 0x{address:x}", relative.display());
			},
		}
	}
	Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
	NoDefinitions,
	Current(u64),
	Patch { offset: u64, old: u64, address: u64 },
}

fn repair(fs: &JailedFs, path: &Path) -> Result<Plan> {
	let mut input = fs.open_read(path, ELF_LIMIT)?;
	let length = input.metadata()?.len();
	let plan = inspect(&mut input, length)?;
	if let Plan::Patch { offset, address, .. } = plan {
		input.seek(SeekFrom::Start(0))?;
		let reader = PatchedReader { input, position: 0, offset, replacement: address.to_le_bytes() };
		fs.atomic_write_from(path, reader, ELF_LIMIT)?;
	}
	Ok(plan)
}

fn inspect(input: &mut File, length: u64) -> Result<Plan> {
	let header = block(input, 0, 64, length)?;
	if &header[..4] != b"\x7fELF" {
		bail!("not an ELF file");
	}
	if header[4] != 2 || header[5] != 1 {
		bail!("not ELF64 little-endian");
	}
	let offset = le64(&header, 40)?;
	let entry = u64::from(le16(&header, 58)?);
	let mut count = u64::from(le16(&header, 60)?);
	if count == 0 && offset == 0 {
		return Ok(Plan::NoDefinitions);
	}
	if offset < 64 || entry < 64 {
		bail!("invalid ELF section table");
	}
	if count == 0 {
		count = le64(&block(input, offset, 64, length)?, 32)?;
	}
	if count == 0 {
		bail!("invalid empty extended ELF section table");
	}
	let size = entry
		.checked_mul(count)
		.context("ELF section table size overflow")?;
	if size > TABLE_LIMIT {
		bail!("ELF section table exceeds its byte limit");
	}
	let sections = block(input, offset, size, length)?;
	let mut definition = None;
	let mut dynamic = None;
	for index in 0..count {
		let base = usize::try_from(index * entry)?;
		match le32(&sections, base + 4)? {
			SHT_GNU_VERDEF => {
				if definition.replace(le64(&sections, base + 16)?).is_some() {
					bail!("ambiguous GNU version-definition sections");
				}
			},
			SHT_DYNAMIC => {
				let previous =
					dynamic.replace((le64(&sections, base + 24)?, le64(&sections, base + 32)?));
				if previous.is_some() {
					bail!("ambiguous ELF dynamic sections");
				}
			},
			_ => {},
		}
	}
	let Some(address) = definition else {
		return Ok(Plan::NoDefinitions);
	};
	let (offset, size) = dynamic.context("no SHT_DYNAMIC section")?;
	if size == 0 || size % 16 != 0 || size > TABLE_LIMIT {
		bail!("invalid or oversized ELF dynamic table");
	}
	let entries = block(input, offset, size, length)?;
	let mut target = None;
	let mut terminated = false;
	for (index, entry) in entries.as_chunks::<16>().0.iter().enumerate() {
		let tag = le64(entry, 0)?;
		if tag == 0 {
			terminated = true;
			break;
		}
		if tag == DT_VERDEF {
			let value = le64(entry, 8)?;
			if target
				.replace((offset + u64::try_from(index)? * 16 + 8, value))
				.is_some()
			{
				bail!("ambiguous DT_VERDEF entries");
			}
		}
	}
	if !terminated {
		bail!("ELF dynamic table has no DT_NULL terminator");
	}
	let (offset, old) = target.context(".gnu.version_d present but no DT_VERDEF entry")?;
	Ok(if old == address {
		Plan::Current(address)
	} else {
		Plan::Patch { offset, old, address }
	})
}

fn block(input: &mut File, offset: u64, size: u64, length: u64) -> Result<Vec<u8>> {
	let end = offset.checked_add(size).context("ELF range overflow")?;
	if end > length {
		bail!("ELF range is outside the file");
	}
	let mut bytes = vec![0; usize::try_from(size)?];
	input.seek(SeekFrom::Start(offset))?;
	input
		.read_exact(&mut bytes)
		.context("truncated ELF input")?;
	Ok(bytes)
}
fn le16(bytes: &[u8], offset: usize) -> Result<u16> {
	Ok(u16::from_le_bytes(
		bytes
			.get(offset..)
			.and_then(|tail| tail.get(..2))
			.context("truncated ELF field")?
			.try_into()?,
	))
}
fn le32(bytes: &[u8], offset: usize) -> Result<u32> {
	Ok(u32::from_le_bytes(
		bytes
			.get(offset..)
			.and_then(|tail| tail.get(..4))
			.context("truncated ELF field")?
			.try_into()?,
	))
}
fn le64(bytes: &[u8], offset: usize) -> Result<u64> {
	Ok(u64::from_le_bytes(
		bytes
			.get(offset..)
			.and_then(|tail| tail.get(..8))
			.context("truncated ELF field")?
			.try_into()?,
	))
}

struct PatchedReader {
	input:       File,
	position:    u64,
	offset:      u64,
	replacement: [u8; 8],
}
impl Read for PatchedReader {
	fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
		let count = self.input.read(output)?;
		let end = self
			.position
			.checked_add(count as u64)
			.ok_or(io::ErrorKind::InvalidData)?;
		let begin = self.position.max(self.offset);
		let patch_end = end.min(self.offset + 8);
		if begin < patch_end {
			let target =
				usize::try_from(begin - self.position).map_err(|_| io::ErrorKind::InvalidData)?;
			let source =
				usize::try_from(begin - self.offset).map_err(|_| io::ErrorKind::InvalidData)?;
			let length = usize::try_from(patch_end - begin).map_err(|_| io::ErrorKind::InvalidData)?;
			output[target..target + length]
				.copy_from_slice(&self.replacement[source..source + length]);
		}
		self.position = end;
		Ok(count)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	const DYNAMIC: usize = 256;
	const ADDRESS: u64 = 0x1234_5678;
	const STALE: u64 = 0xdead_beef;
	fn put16(bytes: &mut [u8], offset: usize, value: u16) {
		bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
	}
	fn put32(bytes: &mut [u8], offset: usize, value: u32) {
		bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}
	fn put64(bytes: &mut [u8], offset: usize, value: u64) {
		bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
	}
	fn fixture() -> Vec<u8> {
		let mut bytes = vec![0; 320];
		bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
		put64(&mut bytes, 40, 64);
		put16(&mut bytes, 58, 64);
		put16(&mut bytes, 60, 3);
		put32(&mut bytes, 128 + 4, SHT_DYNAMIC);
		put64(&mut bytes, 128 + 24, DYNAMIC as u64);
		put64(&mut bytes, 128 + 32, 64);
		put32(&mut bytes, 192 + 4, SHT_GNU_VERDEF);
		put64(&mut bytes, 192 + 16, ADDRESS);
		put64(&mut bytes, DYNAMIC, DT_VERDEF);
		put64(&mut bytes, DYNAMIC + 8, STALE);
		bytes.extend_from_slice(b"compiled runtime payload\0\xff");
		bytes
	}
	fn expected(mut bytes: Vec<u8>) -> Vec<u8> {
		put64(&mut bytes, DYNAMIC + 8, ADDRESS);
		bytes
	}
	#[test]
	fn repair_only_repoints_the_stale_pointer_preserving_all_payload_bytes() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let path = Path::new("binary");
		let bytes = fixture();
		std::fs::write(root.path().join(path), &bytes)?;
		assert!(matches!(repair(&fs, path)?, Plan::Patch { .. }));
		let updated = std::fs::read(root.path().join(path))?;
		assert_eq!(updated, expected(bytes));
		assert_eq!(repair(&fs, path)?, Plan::Current(ADDRESS));
		assert_eq!(std::fs::read(root.path().join(path))?, updated);
		Ok(())
	}
	#[test]
	fn ordinary_and_sectionless_elf_without_definitions_remain_unchanged() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let path = Path::new("binary");
		let mut ordinary = fixture();
		put32(&mut ordinary, 192 + 4, 0);
		let mut sectionless = ordinary[..64].to_vec();
		put64(&mut sectionless, 40, 0);
		put16(&mut sectionless, 60, 0);
		for bytes in [ordinary, sectionless] {
			std::fs::write(root.path().join(path), &bytes)?;
			assert_eq!(repair(&fs, path)?, Plan::NoDefinitions);
			assert_eq!(std::fs::read(root.path().join(path))?, bytes);
		}
		Ok(())
	}
	#[test]
	fn extended_section_counts_are_resolved_before_locating_the_pointer() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let path = Path::new("binary");
		let mut bytes = fixture();
		put16(&mut bytes, 60, 0);
		put64(&mut bytes, 64 + 32, 3);
		std::fs::write(root.path().join(path), &bytes)?;
		repair(&fs, path)?;
		assert_eq!(std::fs::read(root.path().join(path))?, expected(bytes));
		Ok(())
	}
	#[test]
	fn malformed_or_ambiguous_metadata_cannot_publish_a_partial_repair() -> Result<()> {
		let mut cases = vec![("truncated header", fixture()[..6].to_vec())];
		let mut bytes = fixture();
		bytes[0] = 0;
		cases.push(("not ELF", bytes));
		let mut bytes = fixture();
		bytes[4] = 1;
		cases.push(("ELF32", bytes));
		let mut bytes = fixture();
		bytes[5] = 2;
		cases.push(("big endian", bytes));
		let mut bytes = fixture();
		put16(&mut bytes, 58, 0);
		cases.push(("short section entry", bytes));
		let mut bytes = fixture();
		put16(&mut bytes, 58, u16::MAX);
		put16(&mut bytes, 60, u16::MAX);
		cases.push(("oversized table", bytes));
		let mut bytes = fixture();
		put64(&mut bytes, 40, u64::MAX - 8);
		cases.push(("overflowing range", bytes));
		let mut bytes = fixture();
		put16(&mut bytes, 60, 0);
		cases.push(("zero extended count", bytes));
		let mut bytes = fixture();
		put32(&mut bytes, 128 + 4, 0);
		cases.push(("missing dynamic", bytes));
		let mut bytes = fixture();
		put64(&mut bytes, DYNAMIC, 1);
		cases.push(("missing tag", bytes));
		let mut bytes = fixture();
		put64(&mut bytes, 128 + 32, 17);
		cases.push(("truncated dynamic entry", bytes));
		let mut bytes = fixture();
		put64(&mut bytes, 128 + 32, TABLE_LIMIT + 16);
		cases.push(("oversized dynamic", bytes));
		let mut bytes = fixture();
		let length = bytes.len() as u64;
		put64(&mut bytes, 128 + 24, length);
		cases.push(("dynamic outside file", bytes));
		let mut bytes = fixture();
		for index in 1..4 {
			put64(&mut bytes, DYNAMIC + index * 16, 1);
		}
		cases.push(("missing terminator", bytes));
		let mut bytes = fixture();
		put64(&mut bytes, DYNAMIC + 16, DT_VERDEF);
		cases.push(("duplicate pointer", bytes));
		let mut bytes = fixture();
		put32(&mut bytes, 64 + 4, SHT_GNU_VERDEF);
		cases.push(("duplicate definition", bytes));
		let mut bytes = fixture();
		put32(&mut bytes, 64 + 4, SHT_DYNAMIC);
		cases.push(("duplicate dynamic", bytes));
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let path = Path::new("binary");
		for (name, bytes) in cases {
			std::fs::write(root.path().join(path), &bytes)?;
			assert!(repair(&fs, path).is_err(), "{name}");
			assert_eq!(std::fs::read(root.path().join(path))?, bytes, "{name}");
		}
		Ok(())
	}
	#[test]
	fn a_verdef_after_dt_null_is_not_a_live_dynamic_entry() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let path = Path::new("binary");
		let mut bytes = fixture();
		put64(&mut bytes, DYNAMIC, 0);
		put64(&mut bytes, DYNAMIC + 16, DT_VERDEF);
		std::fs::write(root.path().join(path), &bytes)?;
		assert!(repair(&fs, path).is_err());
		assert_eq!(std::fs::read(root.path().join(path))?, bytes);
		Ok(())
	}
	#[test]
	fn replacement_bytes_cross_short_read_boundaries_without_losing_overlay_data() -> Result<()> {
		let root = tempfile::tempdir()?;
		let path = root.path().join("binary");
		let bytes = fixture();
		std::fs::write(&path, &bytes)?;
		let mut reader = PatchedReader {
			input:       File::open(path)?,
			position:    0,
			offset:      (DYNAMIC + 8) as u64,
			replacement: ADDRESS.to_le_bytes(),
		};
		let mut output = Vec::new();
		let mut buffer = [0; 3];
		loop {
			let count = reader.read(&mut buffer)?;
			if count == 0 {
				break;
			}
			output.extend_from_slice(&buffer[..count]);
		}
		assert_eq!(output, expected(bytes));
		Ok(())
	}
	#[cfg(unix)]
	#[test]
	fn repair_cannot_follow_an_escape_or_mutate_an_outside_hardlink_alias() -> Result<()> {
		let root = tempfile::tempdir()?;
		let outside = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let original = fixture();
		let target = outside.path().join("binary");
		std::fs::write(&target, &original)?;
		std::os::unix::fs::symlink(&target, root.path().join("escape"))?;
		assert!(repair(&fs, Path::new("escape")).is_err());
		std::fs::hard_link(&target, root.path().join("alias"))?;
		repair(&fs, Path::new("alias"))?;
		assert_eq!(std::fs::read(&target)?, original);
		assert_eq!(std::fs::read(root.path().join("alias"))?, expected(original));
		Ok(())
	}
}
