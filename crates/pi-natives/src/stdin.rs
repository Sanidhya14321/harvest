//! Compatibility bridge; framing, deadlines and recovery live in harvest-tui.
use std::time::{Duration, Instant};

use harvest_tui::{
	input::{InputBuffer, InputEvent, InputOptions, MAX_INPUT_CHUNK},
	keys::InputMode,
};
use napi::{
	Error, Result,
	bindgen_prelude::{Buffer, Either},
};
use napi_derive::napi;

use crate::terminal::{byte_count, milliseconds};

#[napi(object)]
pub struct NativeStdinBufferOptions {
	pub timeout:              Option<f64>,
	/// Explicit monotonic host clock, selected before the first input.
	pub host_time:            Option<f64>,
	pub partial_hold_timeout: Option<f64>,
	pub paste_timeout:        Option<f64>,
	pub paste_byte_limit:     Option<f64>,
}

#[napi(object)]
pub struct NativePasteEvent {
	pub content: String,
	pub enter:   Option<String>,
}

#[napi]
pub struct NativeStdinBuffer {
	buffer:      InputBuffer,
	clock:       Instant,
	host_time:   Option<Duration>,
	chunk_bytes: u32,
}

#[napi]
impl NativeStdinBuffer {
	#[napi(constructor)]
	pub fn new(options: Option<NativeStdinBufferOptions>) -> Result<Self> {
		let mut parsed = InputOptions::default();
		let mut host_time = None;
		if let Some(options) = options {
			host_time = options
				.host_time
				.map(|value| milliseconds(value, 9_007_199_254_740_991.0))
				.transpose()?;
			if let Some(value) = options.timeout {
				parsed.timeout = milliseconds(value, f64::from(i32::MAX))?;
			}
			if let Some(value) = options.partial_hold_timeout {
				parsed.partial_hold_timeout = milliseconds(value, f64::from(i32::MAX))?;
			}
			if let Some(value) = options.paste_timeout {
				parsed.paste_timeout = milliseconds(value, f64::from(i32::MAX))?;
			}
			if let Some(limit) = options.paste_byte_limit {
				parsed.paste_byte_limit = byte_count(limit)?;
			}
		}
		Ok(Self {
			buffer: InputBuffer::new(parsed, InputMode::default())
				.map_err(|error| Error::from_reason(error.to_string()))?,
			clock: Instant::now(),
			host_time,
			chunk_bytes: MAX_INPUT_CHUNK as u32,
		})
	}

	fn now(&self) -> Duration {
		self.host_time.unwrap_or_else(|| self.clock.elapsed())
	}

	/// Advance a host-owned monotonic clock; timers and framing share its
	/// domain.
	#[napi]
	pub fn set_time(&mut self, elapsed_ms: f64) -> Result<()> {
		let now = milliseconds(elapsed_ms, 9_007_199_254_740_991.0)?;
		let previous = self
			.host_time
			.ok_or_else(|| Error::from_reason("host clock must be selected at construction"))?;
		if now < previous {
			return Err(Error::from_reason("terminal host clock cannot move backwards"));
		}
		self.host_time = Some(now);
		Ok(())
	}

	#[napi(getter)]
	pub const fn chunk_bytes(&self) -> u32 {
		self.chunk_bytes
	}

	#[napi]
	pub fn process(
		&mut self,
		bytes: Buffer,
		kitty_active: bool,
		end_of_read: bool,
	) -> Result<Vec<Either<String, NativePasteEvent>>> {
		self
			.buffer
			.set_mode(InputMode { kitty_protocol_active: kitty_active, ..InputMode::default() });
		let events = self
			.buffer
			.feed(&bytes, self.now(), end_of_read)
			.map_err(|error| Error::from_reason(error.to_string()))?;
		Ok(map_events(events))
	}

	#[napi]
	pub fn tick(&mut self, kitty_active: bool) -> Vec<Either<String, NativePasteEvent>> {
		self
			.buffer
			.set_mode(InputMode { kitty_protocol_active: kitty_active, ..InputMode::default() });
		map_events(self.buffer.tick(self.now()))
	}

	#[napi]
	pub fn delay(&self) -> Option<f64> {
		self
			.buffer
			.next_delay(self.now())
			.map(|delay| delay.as_secs_f64() * 1000.0)
	}

	#[napi]
	pub fn flush(&mut self, kitty_active: bool) -> Vec<String> {
		self
			.buffer
			.set_mode(InputMode { kitty_protocol_active: kitty_active, ..InputMode::default() });
		self.buffer.flush().into_iter().map(owned_text).collect()
	}

	#[napi]
	pub fn clear(&mut self) {
		self.buffer.clear();
	}

	#[napi]
	pub fn get_buffer(&self) -> String {
		String::from_utf8_lossy(&self.buffer.buffered_input()).into_owned()
	}
}

fn map_events(events: Vec<InputEvent>) -> Vec<Either<String, NativePasteEvent>> {
	events
		.into_iter()
		.map(|event| match event {
			InputEvent::Data(bytes) => Either::A(owned_text(bytes)),
			InputEvent::Paste { content, enter } => Either::B(NativePasteEvent {
				content: owned_text(content),
				enter:   enter.map(owned_text),
			}),
		})
		.collect()
}

fn owned_text(bytes: Vec<u8>) -> String {
	String::from_utf8(bytes)
		.unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}
