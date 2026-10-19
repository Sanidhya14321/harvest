//! Native terminal adapter; the bounded queue and pump live in the Rust SDK.
#[cfg(unix)]
use std::{fs::File, os::fd::FromRawFd};
use std::{num::NonZeroUsize, time::Duration};

use harvest_tui::writer::{MAX_QUEUED_BYTES, TtyWriter as TerminalWriter};
use napi::{Error, JsString, Result};
use napi_derive::napi;

use crate::{js, terminal::byte_count};

/// Dedicated output pump owning a duplicate of the supplied Unix descriptor.
#[napi]
pub struct TtyWriter {
	writer:  TerminalWriter,
	maximum: usize,
}
#[napi]
impl TtyWriter {
	/// Start an output thread. Descriptor duplication does not make it
	/// inheritable by child processes; the thread owns and closes the duplicate.
	#[napi(constructor)]
	pub fn new(fd: i32, max_queued_bytes: Option<f64>) -> Result<Self> {
		let maximum = byte_count(max_queued_bytes.unwrap_or(MAX_QUEUED_BYTES as f64))?;
		let limit = NonZeroUsize::new(maximum)
			.ok_or_else(|| Error::from_reason("terminal queue must be positive"))?;
		#[cfg(unix)]
		{
			// SAFETY: fcntl validates the integer descriptor. Only a successful
			// duplicate becomes a Rust-owned File, including on spawn failure.
			let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
			if duplicate < 0 {
				return Err(Error::from_reason(format!(
					"duplicate terminal descriptor failed: {}",
					std::io::Error::last_os_error()
				)));
			}
			// SAFETY: fcntl returned a fresh valid descriptor owned by this call.
			let file = unsafe { File::from_raw_fd(duplicate) };
			let writer = TerminalWriter::with_limit(file, limit).map_err(|error| {
				Error::from_reason(format!("start terminal writer failed: {error}"))
			})?;
			Ok(Self { writer, maximum })
		}
		#[cfg(not(unix))]
		{
			let _ = (fd, limit);
			Err(Error::from_reason("TtyWriter is unix-only"))
		}
	}

	/// Enqueue terminal output without waiting for the terminal. Queue overflow
	/// rejects the whole frame. Warm UTF-16 transcoding uses the scratch arena.
	#[napi]
	pub fn write(&self, data: JsString) -> Result<u32> {
		if self.writer.dead() {
			return Ok(self.pending());
		}
		let units = js::utf16_bounded(data, self.maximum)?;
		let length = xutf::transcoded_len::<xutf::Utf16, xutf::Utf8>(&units);
		self
			.writer
			.enqueue_with(length, |output| {
				let (_, written) = xutf::transcode_into::<xutf::Utf16, xutf::Utf8>(
					&units,
					output,
					xutf::AsciiCase::Preserve,
				);
				Ok(written)
			})
			.map(|pending| pending as u32)
			.map_err(|error| Error::from_reason(format!("terminal output rejected: {error}")))
	}

	/// Bytes accepted but not yet written to the terminal.
	#[napi]
	pub fn pending(&self) -> u32 {
		self.writer.pending() as u32
	}

	/// A failed backend has discarded its remaining queued output.
	#[napi(getter)]
	pub fn dead(&self) -> bool {
		self.writer.dead()
	}

	/// Wait for an empty queue; `dead` distinguishes discarded output from
	/// delivery.
	#[napi]
	pub fn flush_sync(&self, timeout_ms: u32) -> bool {
		self
			.writer
			.flush_sync(Duration::from_millis(u64::from(timeout_ms)))
	}

	/// Stop accepting frames and flush within the deadline. A blocked operation
	/// detaches; its descriptor closes after that operation returns.
	#[napi]
	pub fn stop(&mut self, flush_timeout_ms: u32) {
		self
			.writer
			.stop(Duration::from_millis(u64::from(flush_timeout_ms)));
	}
}
