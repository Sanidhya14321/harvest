//! Bounded JSONL transport for native RPC clients and servers.
//!
//! v1 has one physical frame; negotiated v2 carries a lossless logical object
//! across ordered base64 chunks. No display shortening is applied to history.
use std::io;

use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_LOGICAL_BYTES: usize = 64 * 1024 * 1024;
pub const CHUNK_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Version {
	#[default]
	V1,
	V2,
}

struct LimitedJson {
	bytes: Vec<u8>,
	limit: usize,
}
impl io::Write for LimitedJson {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
			return Err(io::Error::other("RPC output exceeds negotiated frame limit"));
		}
		self.bytes.extend_from_slice(bytes);
		Ok(bytes.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePart {
	#[serde(rename = "type")]
	kind:        String,
	chunk_id:    String,
	index:       usize,
	count:       usize,
	byte_length: usize,
	data:        String,
}

/// One bounded logical message, emitted lazily one physical line at a time.
/// Exhaust this iterator before sending the next message to avoid interleaving.
pub struct Frames {
	bytes:    Vec<u8>,
	chunk_id: String,
	index:    usize,
	count:    usize,
}
impl Frames {
	pub const fn retained_bytes(&self) -> usize {
		self.bytes.len()
	}
}
impl Iterator for Frames {
	type Item = Result<Vec<u8>>;

	fn next(&mut self) -> Option<Self::Item> {
		if self.index >= self.count {
			return None;
		}
		let result = if self.count == 1 {
			let mut bytes = std::mem::take(&mut self.bytes);
			bytes.push(b'\n');
			Ok(bytes)
		} else {
			let start = self.index * CHUNK_BYTES;
			let end = self.bytes.len().min(start + CHUNK_BYTES);
			serde_json::to_vec(&WirePart {
				kind:        "rpc_chunk".into(),
				chunk_id:    self.chunk_id.clone(),
				index:       self.index,
				count:       self.count,
				byte_length: self.bytes.len(),
				data:        STANDARD.encode(&self.bytes[start..end]),
			})
			.map(|mut bytes| {
				bytes.push(b'\n');
				bytes
			})
			.map_err(Into::into)
		};
		self.index += 1;
		Some(result)
	}
}

/// Check the byte ceiling during serialization, before retaining the entire
/// payload. v2's logical ceiling excludes the transport newline, as in legacy.
pub fn encode(value: &impl Serialize, version: Version, chunk_id: String) -> Result<Frames> {
	let limit = match version {
		Version::V1 => MAX_FRAME_BYTES - 1,
		Version::V2 => MAX_LOGICAL_BYTES,
	};
	let mut output = LimitedJson { bytes: Vec::new(), limit };
	serde_json::to_writer(&mut output, value)?;
	let chunked = output.bytes.len() >= MAX_FRAME_BYTES;
	if chunked {
		ensure!(
			!chunk_id.is_empty() && chunk_id.encode_utf16().count() <= 128,
			"invalid rpc chunk metadata"
		);
	}
	let count = if chunked {
		output.bytes.len().div_ceil(CHUNK_BYTES)
	} else {
		1
	};
	Ok(Frames { bytes: output.bytes, chunk_id, index: 0, count })
}

struct Pending {
	id:     String,
	count:  usize,
	length: usize,
	next:   usize,
	bytes:  Vec<u8>,
}

/// A decoder belongs to one ordered stream. Any framing error poisons it;
/// discard it and reconnect rather than accepting an ambiguous partial history.
#[derive(Default)]
pub struct Decoder {
	version: Version,
	pending: Option<Pending>,
	failed:  bool,
}
impl Decoder {
	pub fn new(version: Version) -> Self {
		Self { version, ..Self::default() }
	}

	/// Only change protocol after receiving the negotiation acknowledgement.
	pub fn set_version(&mut self, version: Version) -> Result<()> {
		ensure!(!self.failed && self.pending.is_none(), "RPC decoder is not at a message boundary");
		self.version = version;
		Ok(())
	}

	/// Includes the physical newline in the hard transport budget. A complete
	/// final line without newline is also accepted, matching JSONL EOF behavior.
	pub fn push(&mut self, line: &[u8]) -> Result<Option<Value>> {
		ensure!(!self.failed, "RPC decoder failed; reconnect required");
		let result = self.push_inner(line);
		if result.is_err() {
			self.failed = true;
			self.pending = None;
		}
		result
	}

	fn push_inner(&mut self, line: &[u8]) -> Result<Option<Value>> {
		ensure!(line.len() <= MAX_FRAME_BYTES, "RPC input exceeds frame limit");
		let value: Value = serde_json::from_slice(line)?;
		ensure!(value.is_object(), "rpc frame must be an object");
		if value["type"] != "rpc_chunk" {
			ensure!(self.pending.is_none(), "rpc chunk sequence interrupted");
			return Ok(Some(value));
		}
		ensure!(self.version == Version::V2, "RPC chunk received before protocol negotiation");
		let mut value = value;
		for field in ["index", "count", "byteLength"] {
			let n = crate::transcript::safe_integer(&value[field])
				.ok_or_else(|| anyhow::anyhow!("invalid rpc chunk metadata"))?;
			value[field] = Value::from(n);
		}
		let chunk: WirePart = serde_json::from_value(value)
			.map_err(|_| anyhow::anyhow!("invalid rpc chunk metadata"))?;
		ensure!(
			!chunk.chunk_id.is_empty()
				&& chunk.chunk_id.encode_utf16().count() <= 128
				&& (2..=MAX_LOGICAL_BYTES / CHUNK_BYTES).contains(&chunk.count)
				&& chunk.index < chunk.count
				&& (MAX_FRAME_BYTES..=MAX_LOGICAL_BYTES).contains(&chunk.byte_length),
			"invalid rpc chunk metadata"
		);
		// Validate encoded length before allocating decoded bytes. Strict base64
		// rejects whitespace, omitted padding and noncanonical trailing bits.
		ensure!(
			!chunk.data.is_empty() && chunk.data.len() <= CHUNK_BYTES.div_ceil(3) * 4,
			"invalid rpc chunk data"
		);
		let bytes = STANDARD
			.decode(&chunk.data)
			.map_err(|_| anyhow::anyhow!("invalid rpc chunk data"))?;
		ensure!(STANDARD.encode(&bytes) == chunk.data, "invalid rpc chunk data");
		ensure!(bytes.len() <= CHUNK_BYTES, "rpc chunk payload exceeds the transport limit");
		if self.pending.is_none() {
			ensure!(chunk.index == 0, "rpc chunk sequence must start at index 0");
			self.pending = Some(Pending {
				id:     chunk.chunk_id.clone(),
				count:  chunk.count,
				length: chunk.byte_length,
				next:   0,
				bytes:  Vec::new(),
			});
		}
		let pending = self.pending.as_mut().expect("initialized above");
		ensure!(
			(pending.id.as_str(), pending.count, pending.length, pending.next)
				== (chunk.chunk_id.as_str(), chunk.count, chunk.byte_length, chunk.index),
			"rpc chunk sequence mismatch"
		);
		ensure!(
			bytes.len() <= pending.length.saturating_sub(pending.bytes.len()),
			"rpc chunk sequence exceeds declared length"
		);
		pending.bytes.extend_from_slice(&bytes);
		pending.next += 1;
		if pending.next < pending.count {
			return Ok(None);
		}
		ensure!(pending.bytes.len() == pending.length, "rpc chunk sequence length mismatch");
		let pending = self.pending.take().expect("initialized above");
		let value: Value = serde_json::from_slice(&pending.bytes)?;
		ensure!(value.is_object(), "rpc frame must be an object");
		Ok(Some(value))
	}

	/// Call at EOF, including after unexpected process exit.
	pub fn finish(&mut self) -> Result<()> {
		if self.failed || self.pending.is_some() {
			self.failed = true;
			self.pending = None;
			bail!("RPC stream ended without a complete message");
		}
		Ok(())
	}
}
