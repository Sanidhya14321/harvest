//! Cancellation-safe bounded RPC reads for native SDK clients.
use anyhow::{Result, bail, ensure};
use harvest_protocol::rpc_frame::{Decoder, MAX_FRAME_BYTES, Version};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub(super) enum Physical {
	Frame(Vec<u8>),
	Invalid,
}

pub(super) struct PhysicalReader<R> {
	input:     BufReader<R>,
	partial:   Vec<u8>,
	oversized: bool,
}
impl<R: AsyncRead + Unpin> PhysicalReader<R> {
	pub(super) fn new(input: R) -> Self {
		Self { input: BufReader::new(input), partial: Vec::new(), oversized: false }
	}

	pub(super) async fn next(&mut self) -> Result<Option<Physical>> {
		loop {
			let buffer = self.input.fill_buf().await?;
			if buffer.is_empty() {
				return if std::mem::take(&mut self.oversized) {
					Ok(Some(Physical::Invalid))
				} else if self.partial.is_empty() {
					Ok(None)
				} else {
					Ok(Some(Physical::Frame(std::mem::take(&mut self.partial))))
				};
			}
			let newline = buffer.iter().position(|byte| *byte == b'\n');
			let count = newline.map_or(buffer.len(), |index| index + 1);
			if !self.oversized {
				if count > MAX_FRAME_BYTES.saturating_sub(self.partial.len()) {
					self.oversized = true;
					self.partial.clear();
				} else {
					self.partial.extend_from_slice(&buffer[..count]);
				}
			}
			self.input.consume(count);
			if newline.is_some() {
				return Ok(Some(if std::mem::take(&mut self.oversized) {
					Physical::Invalid
				} else {
					Physical::Frame(std::mem::take(&mut self.partial))
				}));
			}
		}
	}
}

/// Read ordered logical objects from a child process or socket without first
/// allocating an unbounded line.
///
/// The decoder starts in v1; enable v2 only after
/// the server acknowledges negotiation. Dropping a `next` future retains
/// partial physical/chunk input, so callers may use cancellation or timeouts
/// safely.
pub struct Reader<R> {
	physical: PhysicalReader<R>,
	decoder:  Decoder,
	finished: bool,
	failed:   bool,
}
impl<R: AsyncRead + Unpin> Reader<R> {
	pub fn new(input: R) -> Self {
		Self {
			physical: PhysicalReader::new(input),
			decoder:  Decoder::default(),
			finished: false,
			failed:   false,
		}
	}

	pub fn set_version(&mut self, version: Version) -> Result<()> {
		ensure!(
			!self.failed
				&& !self.finished
				&& self.physical.partial.is_empty()
				&& !self.physical.oversized,
			"RPC reader is not at a message boundary"
		);
		self.decoder.set_version(version)
	}

	pub async fn next(&mut self) -> Result<Option<Value>> {
		ensure!(!self.failed, "RPC reader failed; reconnect required");
		if self.finished {
			return Ok(None);
		}
		let result = self.next_inner().await;
		if result.is_err() {
			self.failed = true;
			// Release partial logical/physical data on a terminal transport error.
			self.decoder = Decoder::default();
			self.physical.partial.clear();
		}
		result
	}

	async fn next_inner(&mut self) -> Result<Option<Value>> {
		loop {
			match self.physical.next().await? {
				None => {
					self.decoder.finish()?;
					self.finished = true;
					return Ok(None);
				},
				Some(Physical::Invalid) => bail!("RPC input exceeds frame limit"),
				Some(Physical::Frame(line)) => {
					if let Some(frame) = self.decoder.push(&line)? {
						return Ok(Some(frame));
					}
				},
			}
		}
	}
}
