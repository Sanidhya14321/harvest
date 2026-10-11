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
