//! Shared bounded SSE framing. Provider state machines own event semantics.
use std::io::{BufRead, BufReader, Read};

use anyhow::{Context, Result, bail};
use harvest_runtime::{MAX_EVENT_BYTES, MAX_HTTP_BYTES, cancel::CancelToken};

pub(crate) struct Events<R: Read> {
	reader:   BufReader<std::io::Take<R>>,
	consumed: usize,
}
impl<R: Read> Events<R> {
	pub fn new(reader: R) -> Self {
		Self { reader: BufReader::new(reader.take(MAX_HTTP_BYTES as u64 + 1)), consumed: 0 }
	}

	pub fn next(&mut self, cancellation: &CancelToken) -> Result<Option<String>> {
		let mut event = String::new();
		let mut line = Vec::new();
		loop {
			cancellation.heartbeat()?;
			line.clear();
			let count = (&mut self.reader)
				.take(MAX_EVENT_BYTES as u64 + 1)
				.read_until(b'\n', &mut line)?;
			self.consumed += count;
			if self.consumed > MAX_HTTP_BYTES {
				bail!("stream exceeds response limit");
			}
			if line.len() > MAX_EVENT_BYTES {
				bail!("stream line exceeds frame limit");
			}
			if count == 0 {
				if !event.is_empty() {
					bail!("stream ended inside an event");
				}
				return Ok(None);
			}
			let text = std::str::from_utf8(&line)
				.context("stream is not UTF-8")?
				.trim_end_matches(['\r', '\n']);
			if text.is_empty() {
				if !event.is_empty() {
					return Ok(Some(event));
				}
			} else if let Some(data) = text.strip_prefix("data:") {
				if !event.is_empty() {
					event.push('\n');
				}
				event.push_str(data.strip_prefix(' ').unwrap_or(data));
				if event.len() > MAX_EVENT_BYTES {
					bail!("stream event exceeds frame limit");
				}
			}
		}
	}
}
