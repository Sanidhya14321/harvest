//! Legacy host adapters for shared terminal output policy.
use std::num::NonZeroUsize;

use harvest_tui::output::{CONPTY_WRITE_BYTES, ConptyChunks, StallWatchdog};
use napi::{Error, JsString, Result};
use napi_derive::napi;

use crate::{
	js,
	terminal::{byte_count, milliseconds},
};

/// Return UTF-16 end offsets so JavaScript slices preserve original code units,
/// including isolated surrogates that the terminal itself encodes as U+FFFD.
#[napi]
pub fn conpty_chunk_ends(data: JsString, max_chunk_bytes: Option<f64>) -> Result<Vec<u32>> {
	let limit = byte_count(max_chunk_bytes.unwrap_or(CONPTY_WRITE_BYTES as f64))?;
	let limit = NonZeroUsize::new(limit)
		.ok_or_else(|| Error::from_reason("terminal chunk size must be positive"))?;
	let units = js::utf16(data)?;
	let data = String::from_utf16_lossy(&units);
	if data.is_empty() {
		return Ok(vec![0]);
	}
	let mut offset = 0;
	Ok(ConptyChunks::new(&data, limit)
		.map(|chunk| {
			offset += chunk.encode_utf16().count() as u32;
			offset
		})
		.collect())
}

#[napi]
pub struct NativeStdoutStallWatchdog {
	watchdog: StallWatchdog,
}
#[napi]
impl NativeStdoutStallWatchdog {
	#[napi(constructor)]
	pub fn new(
		arm_bytes: Option<f64>,
		clear_bytes: Option<f64>,
		stall_ms: Option<f64>,
	) -> Result<Self> {
		Ok(Self {
			watchdog: StallWatchdog::new(
				byte_count(arm_bytes.unwrap_or(64.0 * 1024.0 * 1024.0))?,
				byte_count(clear_bytes.unwrap_or(256.0 * 1024.0))?,
				milliseconds(stall_ms.unwrap_or(2000.0), f64::MAX)?,
			),
		})
	}

	#[napi(getter)]
	pub const fn armed(&self) -> bool {
		self.watchdog.armed()
	}

	#[napi]
	pub fn sample(&mut self, pending: f64, now_ms: f64) -> Result<bool> {
		Ok(self
			.watchdog
			.sample(byte_count(pending)?, milliseconds(now_ms, f64::MAX)?))
	}

	#[napi]
	pub const fn reset(&mut self) {
		self.watchdog.reset();
	}
}
