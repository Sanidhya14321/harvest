//! Bounded, lossless session JSONL decoding for the Rust migration.
//! This codec does not replace legacy persistence or provider replay yet.
use std::{
	collections::{BTreeMap, BTreeSet},
	path::Path,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, content_hash};
use serde::Serialize;
use serde_json::value::RawValue;

pub const SESSION_LIMIT: usize = 64 * 1024 * 1024;
const ENTRY_LIMIT: usize = 100_000;
pub const TITLE_SLOT_BYTES: usize = 256;
pub type SessionObject = BTreeMap<String, Box<RawValue>>;

/// A recovery receipt describes the exact preserved prefix and excluded tail.
/// The source remains untouched; excluded bytes remain available in it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
	pub source_hash:         String,
	pub recovered_bytes:     usize,
	pub excluded_tail_bytes: usize,
}

pub struct SessionDocument {
	original:          Vec<u8>,
	header:            SessionObject,
	entries:           Vec<SessionObject>,
	malformed_records: usize,
	migrated:          bool,
	by_id:             BTreeMap<String, usize>,
}

fn has_string(object: &SessionObject, key: &str, expected: &str) -> bool {
	decoded_string(object, key).is_ok_and(|value| value == expected)
}
pub(crate) fn decoded_string(object: &SessionObject, key: &str) -> Result<String> {
	serde_json::from_str(
		object
			.get(key)
			.with_context(|| format!("missing session {key}"))?
			.get(),
	)
	.with_context(|| format!("invalid session {key}"))
}
fn put(object: &mut SessionObject, key: &str, value: impl serde::Serialize) -> Result<()> {
	object.insert(key.to_owned(), serde_json::value::to_raw_value(&value)?);
	Ok(())
}

impl SessionDocument {
	pub fn decode(bytes: &[u8]) -> Result<Self> {
		if bytes.len() > SESSION_LIMIT {
			bail!("session exceeds size limit");
		}
		let mut records = Vec::new();
		let mut malformed_records = 0;
		let mut title_slot = None;
		for (line_index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
			if line.len() > harvest_runtime::MAX_EVENT_BYTES {
				bail!("session record exceeds frame limit");
			}
			if line.iter().all(u8::is_ascii_whitespace) {
				continue;
			}
			let Ok(record) = serde_json::from_slice::<SessionObject>(line) else {
				malformed_records += 1;
				continue;
			};
			if line_index == 0 && valid_title_slot(&record) {
				title_slot = Some(record);
				continue;
			}
			if records.len() == ENTRY_LIMIT {
				bail!("session exceeds entry limit");
			}
			records.push(record);
		}
		let mut header = if records.is_empty() {
			bail!("session has no header");
		} else {
			records.remove(0)
		};
		if !has_string(&header, "type", "session") {
			bail!("invalid session header; headerless prototype logs require explicit import");
		}
		decoded_string(&header, "id")?;
		let version = header
			.get("version")
			.map(|raw| serde_json::from_str::<u32>(raw.get()))
			.transpose()?
			.unwrap_or(1);
		if !(1..=3).contains(&version) {
			bail!("unsupported session version {version}");
		}
		if let Some(slot) = title_slot {
			header.insert("title".into(), slot["title"].clone());
			if let Some(source) = slot.get("source") {
				header.insert("titleSource".into(), source.clone());
			} else {
				header.remove("titleSource");
			}
		}
		if version == 1 {
			let source_hash = harvest_runtime::content_hash(bytes);
			let mut ids = BTreeSet::new();
			let mut previous: Option<String> = None;
			for (index, entry) in records.iter_mut().enumerate() {
				let digest = harvest_runtime::content_hash(format!("{source_hash}:{index}").as_bytes());
				let mut id = digest[..8].to_owned();
				if !ids.insert(id.clone()) {
					id = digest;
					if !ids.insert(id.clone()) {
						bail!("session migration ID collision");
					}
				}
				put(entry, "id", &id)?;
				put(entry, "parentId", &previous)?;
				previous = Some(id);
			}
			// Old indexes refer to the logical file, including its header.
			let ids: Vec<String> = records
				.iter()
				.map(|entry| decoded_string(entry, "id"))
				.collect::<Result<_>>()?;
			for entry in &mut records {
				if has_string(entry, "type", "compaction")
					&& let Some(raw) = entry.remove("firstKeptEntryIndex")
				{
					let index: usize = serde_json::from_str(raw.get())?;
					if let Some(id) = index.checked_sub(1).and_then(|index| ids.get(index)) {
						put(entry, "firstKeptEntryId", id)?;
					}
				}
			}
		}
		if version < 3 {
			put(&mut header, "version", 3)?;
			for entry in &mut records {
				if has_string(entry, "type", "message")
					&& let Some(raw) = entry.get("message")
				{
					let mut message: SessionObject = serde_json::from_str(raw.get())?;
					if has_string(&message, "role", "hookMessage") {
						put(&mut message, "role", "custom")?;
						entry.insert("message".into(), serde_json::value::to_raw_value(&message)?);
					}
				}
			}
		}
		let mut by_id = BTreeMap::new();
		for (index, entry) in records.iter().enumerate() {
			let id = decoded_string(entry, "id")?;
			if by_id.insert(id, index).is_some() {
				bail!("duplicate session entry ID");
			}
		}
		Ok(Self {
			original: bytes.to_vec(),
			header,
			entries: records,
			malformed_records,
			migrated: version < 3,
			by_id,
		})
	}

	pub const fn migrated(&self) -> bool {
		self.migrated
	}

	pub const fn malformed_records(&self) -> usize {
		self.malformed_records
	}

	pub const fn header(&self) -> &SessionObject {
		&self.header
	}

	pub fn entries(&self) -> &[SessionObject] {
		&self.entries
	}

	/// Follow only the selected branch, preserving raw provider message blocks.
	pub fn branch(&self, leaf: &str) -> Result<Vec<&SessionObject>> {
		let mut branch = Vec::new();
		let mut cursor = Some(leaf.to_owned());
		let mut seen = BTreeSet::new();
		while let Some(id) = cursor {
			if !seen.insert(id.clone()) {
				bail!("cyclic session parent chain");
			}
			let entry = &self.entries[*self.by_id.get(&id).context("missing branch entry")?];
			branch.push(entry);
			let raw = entry.get("parentId").context("missing session parentId")?;
			cursor = serde_json::from_str::<Option<String>>(raw.get())?;
		}
		branch.reverse();
		Ok(branch)
	}

	/// Original bytes remain available even after in-memory legacy migration.
	pub fn original_bytes(&self) -> &[u8] {
		&self.original
	}

	/// Replace only the physical title line, retaining header/provider bytes.
	/// A caller must hold the session writer lease before reading this snapshot.
	pub fn retitled(&self, title: &str, source: Option<&str>, updated_at: &str) -> Result<Vec<u8>> {
		if self.migrated || self.malformed_records != 0 || !self.original.ends_with(b"\n") {
			bail!("title changes require an intact v3 session; import or recover it first");
		}
		self.validate_tree()?;
		let slot = title_slot(title, source, updated_at)?;
		let first_end = self
			.original
			.iter()
			.position(|byte| *byte == b'\n')
			.context("missing session first line")?;
		let has_slot = serde_json::from_slice::<SessionObject>(&self.original[..first_end])
			.is_ok_and(|record| valid_title_slot(&record));
		let body = if has_slot {
			&self.original[first_end + 1..]
		} else {
			&self.original
		};
		let length = slot
			.len()
			.checked_add(body.len())
			.context("session size overflow")?;
		if length > SESSION_LIMIT {
			bail!("session exceeds size limit after title update");
		}
		let mut bytes = Vec::with_capacity(length);
		bytes.extend_from_slice(&slot);
		bytes.extend_from_slice(body);
		Ok(bytes)
	}

	pub(crate) fn validate_tree(&self) -> Result<()> {
		let mut visited = BTreeSet::new();
		for start in 0..self.entries.len() {
			let mut chain = BTreeSet::new();
			let mut cursor = Some(start);
			while let Some(index) = cursor {
				if visited.contains(&index) {
					break;
				}
				if !chain.insert(index) {
					bail!("cyclic session parent chain");
				}
				let raw = self.entries[index]
					.get("parentId")
					.context("missing session parentId")?;
				let parent: Option<String> = serde_json::from_str(raw.get())?;
				cursor = parent
					.map(|id| self.by_id.get(&id).copied().context("missing branch entry"))
					.transpose()?;
			}
			visited.extend(chain);
		}
		Ok(())
	}

	/// Publish an explicit, non-destructive v3 import through the workspace
	/// jail. Inspection may tolerate malformed rows; imports must never drop
	/// them.
	pub fn import_to(&self, fs: &JailedFs, destination: &Path) -> Result<()> {
		if self.malformed_records != 0 {
			bail!(
				"damaged session requires explicit recovery before import; no records were discarded"
			);
		}
		self.validate_tree()?;
		let mut bytes = self.export_v3()?;
		if !bytes.ends_with(b"\n") {
			bytes.push(b'\n');
		}
		if bytes.len() > SESSION_LIMIT {
			bail!("imported session exceeds size limit");
		}
		fs.atomic_create(destination, &bytes)
	}

	/// Recover a v3 log whose final JSON record was interrupted during append.
	/// Only an incomplete final record is excluded; interior damage, complete
	/// records and malformed syntax require a different explicit action.
	pub fn recover_to(&self, fs: &JailedFs, destination: &Path) -> Result<RecoveryReport> {
		if self.migrated {
			bail!(
				"tail recovery requires a v3 session; legacy recovery needs an explicit migration \
				 review"
			);
		}
		if self.original.ends_with(b"\n") {
			bail!("session has no unterminated final record; use --import-v3 for a complete session");
		}
		let boundary = self
			.original
			.iter()
			.rposition(|byte| *byte == b'\n')
			.context("session has no complete header before its interrupted record")?
			+ 1;
		let (prefix, tail) = self.original.split_at(boundary);
		let textual_tail = match std::str::from_utf8(tail) {
			Ok(_) => tail,
			Err(error) if error.error_len().is_none() => &tail[..error.valid_up_to()],
			Err(_) => bail!("final record contains invalid UTF-8; manual recovery is required"),
		};
		if textual_tail.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
			bail!("final fragment is not an interrupted JSON record; manual recovery is required");
		}
		match serde_json::from_slice::<SessionObject>(textual_tail) {
			Err(error) if error.is_eof() => {},
			Ok(_) => bail!("final record is complete; use --import-v3 to add its missing delimiter"),
			Err(_) => {
				bail!("final record is malformed rather than truncated; manual recovery is required")
			},
		}
		// The incomplete tail must be the only row the original decoder could
		// not parse. Reuse its complete prefix tree without allocating and
		// decoding another full session snapshot.
		if self.malformed_records != 1 {
			bail!(
				"damaged session has malformed records before its interrupted tail; no records were \
				 discarded"
			);
		}
		self.validate_tree()?;
		fs.atomic_create(destination, prefix)?;
		Ok(RecoveryReport {
			source_hash:         content_hash(&self.original),
			recovered_bytes:     prefix.len(),
			excluded_tail_bytes: tail.len(),
		})
	}

	/// Produce a separate v3 import body without changing the original file.
	/// Unknown values (including signed/encrypted payloads and large numbers)
	/// stay raw JSON. A current document is returned byte-for-byte unchanged.
	pub fn export_v3(&self) -> Result<Vec<u8>> {
		if !self.migrated {
			return Ok(self.original.clone());
		}
		let mut bytes = Vec::new();
		for record in std::iter::once(&self.header).chain(self.entries.iter()) {
			let start = bytes.len();
			serde_json::to_writer(&mut bytes, record)?;
			if bytes.len() - start > harvest_runtime::MAX_EVENT_BYTES {
				bail!("migrated session record exceeds frame limit");
			}
			bytes.push(b'\n');
			if bytes.len() > SESSION_LIMIT {
				bail!("migrated session exceeds size limit");
			}
		}
		Ok(bytes)
	}
}

fn valid_title_slot(slot: &SessionObject) -> bool {
	has_string(slot, "type", "title")
		&& slot.get("v").is_some_and(|raw| raw.get() == "1")
		&& ["title", "updatedAt", "pad"]
			.iter()
			.all(|key| decoded_string(slot, key).is_ok())
		&& slot
			.get("source")
			.is_none_or(|_| matches!(decoded_string(slot, "source").as_deref(), Ok("auto" | "user")))
}

/// Write the legacy fixed-width first line without splitting Unicode
/// codepoints.
pub fn title_slot(title: &str, source: Option<&str>, updated_at: &str) -> Result<Vec<u8>> {
	if source.is_some_and(|source| !matches!(source, "auto" | "user")) {
		bail!("invalid title source");
	}
	if title.len() > harvest_runtime::MAX_EVENT_BYTES || updated_at.len() > TITLE_SLOT_BYTES {
		bail!("title input exceeds limit");
	}
	let mut slot = SessionObject::new();
	put(&mut slot, "type", "title")?;
	put(&mut slot, "v", 1)?;
	put(&mut slot, "updatedAt", updated_at)?;
	if let Some(source) = source {
		put(&mut slot, "source", source)?;
	}
	put(&mut slot, "pad", "")?;
	let mut boundaries: Vec<usize> = title.char_indices().map(|(index, _)| index).collect();
	boundaries.push(title.len());
	let mut low = 0;
	let mut high = boundaries.len();
	let mut best = None;
	while low < high {
		let middle = low + (high - low) / 2;
		put(&mut slot, "title", &title[..boundaries[middle]])?;
		let size = serde_json::to_vec(&slot)?.len() + 1;
		if size <= TITLE_SLOT_BYTES {
			best = Some((boundaries[middle], size));
			low = middle + 1;
		} else {
			high = middle;
		}
	}
	let (length, size) = best.context("title metadata exceeds fixed slot size")?;
	put(&mut slot, "title", &title[..length])?;
	put(&mut slot, "pad", " ".repeat(TITLE_SLOT_BYTES - size))?;
	let mut bytes = serde_json::to_vec(&slot)?;
	bytes.push(b'\n');
	Ok(bytes)
}
