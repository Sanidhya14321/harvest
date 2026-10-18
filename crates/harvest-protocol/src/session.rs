//! Bounded, lossless session JSONL decoding for the Rust migration.
//! This codec does not own persistence or rewrite provider replay payloads.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde_json::value::RawValue;

use crate::digest::content_hash;

/// Largest supported whole session.
pub const SESSION_LIMIT: usize = 64 * 1024 * 1024;
const ENTRY_LIMIT: usize = 100_000;
/// Maximum serialized record size, including opaque provider fields.
pub const SESSION_RECORD_LIMIT: usize = 1024 * 1024;
pub const TITLE_SLOT_BYTES: usize = 256;
pub type SessionObject = BTreeMap<String, Box<RawValue>>;

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
fn decoded_string(object: &SessionObject, key: &str) -> Result<String> {
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
			if line.len() > SESSION_RECORD_LIMIT {
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
			let source_hash = content_hash(bytes);
			let mut ids = BTreeSet::new();
			let mut previous: Option<String> = None;
			for (index, entry) in records.iter_mut().enumerate() {
				let digest = content_hash(format!("{source_hash}:{index}").as_bytes());
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
			if bytes.len() - start > SESSION_RECORD_LIMIT {
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
	if title.len() > SESSION_RECORD_LIMIT || updated_at.len() > TITLE_SLOT_BYTES {
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
