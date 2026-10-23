//! Bounded I/O utilities shared by providers, tools and persistence.

use std::io::Read;

use anyhow::{Result, bail};

pub const MAX_TOOL_BYTES: usize = 80_000;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Read at most `limit + 1` bytes and reject oversized input before decoding
/// it.
pub fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
	let mut bytes = Vec::new();
	reader
		.take(u64::try_from(limit)?.saturating_add(1))
		.read_to_end(&mut bytes)?;
	if bytes.len() > limit {
		bail!("input exceeds the {limit}-byte limit");
	}
	Ok(bytes)
}

/// Bound a UTF-8 preview without splitting a codepoint.
pub fn truncate_bytes(text: &str, limit: usize) -> &str {
	let mut end = text.len().min(limit);
	while !text.is_char_boundary(end) {
		end = end.saturating_sub(1);
	}
	&text[..end]
}

/// Serialize into a bounded buffer without first allocating an oversized
/// result. A failed write leaves its input entirely unwritten; callers may
/// discard the buffer on error and publish only after the serializer has
/// finished.
pub struct BoundedWriter {
	bytes: Vec<u8>,
	limit: usize,
}
impl BoundedWriter {
	pub fn new(limit: usize) -> Self {
		Self { bytes: Vec::new(), limit }
	}

	pub fn into_inner(self) -> Vec<u8> {
		self.bytes
	}
}
impl std::io::Write for BoundedWriter {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
			return Err(std::io::Error::other("serialized output exceeds byte limit"));
		}
		self.bytes.extend_from_slice(bytes);
		Ok(bytes.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}
