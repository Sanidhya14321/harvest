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

#[napi(object)]
pub struct NativeStdinBufferOptions {
	pub timeout:              Option<f64>,
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
	chunk_bytes: u32,
}

#[napi]
impl NativeStdinBuffer {
	#[napi(constructor)]
	pub fn new(options: Option<NativeStdinBufferOptions>) -> Result<Self> {
		let mut parsed = InputOptions::default();
		if let Some(options) = options {
			parsed.timeout = milliseconds(options.timeout, parsed.timeout)?;
			parsed.partial_hold_timeout =
				milliseconds(options.partial_hold_timeout, parsed.partial_hold_timeout)?;
			parsed.paste_timeout = milliseconds(options.paste_timeout, parsed.paste_timeout)?;
			if let Some(limit) = options.paste_byte_limit {
				if !limit.is_finite() || limit < 0.0 {
					return Err(Error::from_reason("paste byte limit must be finite and nonnegative"));
				}
				parsed.paste_byte_limit = limit as usize;
			}
		}
		Ok(Self {
			buffer:      InputBuffer::new(parsed, InputMode::default())
				.map_err(|error| Error::from_reason(error.to_string()))?,
			clock:       Instant::now(),
			chunk_bytes: MAX_INPUT_CHUNK as u32,
		})
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
			.feed(&bytes, self.clock.elapsed(), end_of_read)
			.map_err(|error| Error::from_reason(error.to_string()))?;
		Ok(map_events(events))
	}

	#[napi]
	pub fn tick(&mut self, kitty_active: bool) -> Vec<Either<String, NativePasteEvent>> {
		self
			.buffer
			.set_mode(InputMode { kitty_protocol_active: kitty_active, ..InputMode::default() });
		map_events(self.buffer.tick(self.clock.elapsed()))
	}

	#[napi]
	pub fn delay(&self) -> Option<f64> {
		self
			.buffer
			.next_delay(self.clock.elapsed())
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

fn milliseconds(value: Option<f64>, fallback: Duration) -> Result<Duration> {
	let Some(value) = value else {
		return Ok(fallback);
	};
	if !value.is_finite() || !(0.0..=f64::from(i32::MAX)).contains(&value) {
		return Err(Error::from_reason("terminal timeout must be within the finite timer range"));
	}
	Duration::try_from_secs_f64(value / 1000.0)
		.map_err(|_| Error::from_reason("terminal timeout is not representable"))
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
