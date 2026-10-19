//! Bounded terminal read framing with a host-owned monotonic clock.
//!
//! Ported from Harvest's `StdinBuffer`, based on `OpenTUI`
//! (<https://github.com/anomalyco/opentui>), MIT License, copyright (c) 2025 opentui.
use std::{borrow::Cow, fmt, mem, time::Duration};

use crate::keys::{InputMode, parse_kitty_sequence};

/// Maximum bytes accepted by one `feed` call, before any state changes.
pub const MAX_INPUT_CHUNK: usize = 1024 * 1024;
const MAX_PASTE: usize = 64 * 1024 * 1024;
const MAX_STRING: usize = 16 * 1024 * 1024;
const MAX_CSI: usize = 4096;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const RAW_WINDOW: Duration = Duration::from_millis(10);
const DEDUP_WINDOW: Duration = Duration::from_millis(25);
const DISCARD_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct InputOptions {
	pub timeout:              Duration,
	pub partial_hold_timeout: Duration,
	pub paste_timeout:        Duration,
	pub paste_byte_limit:     usize,
	pub csi_byte_limit:       usize,
	pub string_byte_limit:    usize,
}
impl Default for InputOptions {
	fn default() -> Self {
		Self {
			timeout:              Duration::from_millis(75),
			partial_hold_timeout: Duration::from_millis(150),
			paste_timeout:        Duration::from_secs(1),
			paste_byte_limit:     MAX_PASTE,
			csi_byte_limit:       MAX_CSI,
			string_byte_limit:    MAX_STRING,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
	Data(Vec<u8>),
	Paste { content: Vec<u8>, enter: Option<Vec<u8>> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
	ChunkTooLarge,
	InvalidLimits,
}
impl fmt::Display for InputError {
	fn fmt(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
		sink.write_str(match self {
			Self::ChunkTooLarge => {
				"terminal chunks must not exceed 1 MiB; split a read using end_of_read=false"
			},
			Self::InvalidLimits => "terminal limits exceed the supported bounded range",
		})
	}
}
impl std::error::Error for InputError {}

#[derive(Debug)]
struct TimedBytes {
	bytes: Vec<u8>,
	at:    Duration,
}
#[derive(Debug)]
struct RawCandidate {
	bytes:        Vec<u8>,
	at:           Duration,
	line_breaks:  u8,
	preceding_cr: bool,
}
impl RawCandidate {
	const fn new(at: Duration) -> Self {
		Self { bytes: Vec::new(), at, line_breaks: 0, preceding_cr: false }
	}

	fn append(&mut self, bytes: &[u8]) -> bool {
		self.bytes.extend_from_slice(bytes);
		for &byte in bytes {
			if byte == b'\n' && self.preceding_cr {
				self.preceding_cr = false;
				continue;
			}
			self.preceding_cr = byte == b'\r';
			if byte == b'\r' || byte == b'\n' {
				self.line_breaks = self.line_breaks.saturating_add(1);
			} else if self.line_breaks >= 2 {
				return true;
			}
		}
		false
	}
}

#[derive(Debug)]
struct PendingPaste {
	content: Vec<u8>,
	suffix:  Vec<u8>,
	at:      Duration,
}
#[derive(Debug)]
struct Discard {
	bytes:       usize,
	held_escape: bool,
	at:          Duration,
}

/// Terminal protocol state without ownership of a terminal or timer runtime.
///
/// Supply nondecreasing elapsed times to `feed`, `tick` and `next_delay`.
/// Deliver queued input before a second zero-delay timer turn; Escape expiry
/// deliberately defers once to avoid turning queued protocol tails into text.
pub struct InputBuffer {
	options:            InputOptions,
	mode:               InputMode,
	buffer:             Vec<u8>,
	search_offset:      usize,
	paste_start_offset: usize,
	flush_at:           Option<Duration>,
	flush_deferred:     bool,
	hold_since:         Option<Duration>,
	paste:              Option<TimedBytes>,
	pending_paste:      Option<PendingPaste>,
	raw:                Option<RawCandidate>,
	printable:          Option<(u32, Duration)>,
	discard:            Option<Discard>,
}

impl fmt::Debug for InputBuffer {
	fn fmt(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
		sink
			.debug_struct("InputBuffer")
			.field("mode", &self.mode)
			.field("retained_bytes", &self.retained_bytes())
			.field("paste_active", &self.paste.is_some())
			.field("discard_active", &self.discard.is_some())
			.finish_non_exhaustive()
	}
}

impl InputBuffer {
	pub fn new(options: InputOptions, mode: InputMode) -> Result<Self, InputError> {
		if options.paste_byte_limit > MAX_PASTE
			|| !(6..=MAX_CSI).contains(&options.csi_byte_limit)
			|| !(4..=MAX_STRING).contains(&options.string_byte_limit)
		{
			return Err(InputError::InvalidLimits);
		}
		Ok(Self {
			options,
			mode,
			buffer: Vec::new(),
			search_offset: 0,
			paste_start_offset: 0,
			flush_at: None,
			flush_deferred: false,
			hold_since: None,
			paste: None,
			pending_paste: None,
			raw: None,
			printable: None,
			discard: None,
		})
	}

	pub const fn set_mode(&mut self, mode: InputMode) {
		self.mode = mode;
	}

	/// Pending key/raw-burst bytes; active bracketed paste payloads stay
	/// private.
	pub fn buffered_input(&self) -> Vec<u8> {
		let mut bytes = self
			.raw
			.as_ref()
			.map_or_else(Vec::new, |raw| raw.bytes.clone());
		bytes.extend_from_slice(&self.buffer);
		bytes
	}

	/// Payload bytes retained across all protocol and paste states.
	pub fn retained_bytes(&self) -> usize {
		self.buffer.len()
			+ self.raw.as_ref().map_or(0, |raw| raw.bytes.len())
			+ self.paste.as_ref().map_or(0, |paste| paste.bytes.len())
			+ self
				.pending_paste
				.as_ref()
				.map_or(0, |pending| pending.content.len() + pending.suffix.len())
	}

	pub fn clear(&mut self) {
		drop(self.take_buffer());
		self.paste = None;
		self.pending_paste = None;
		self.raw = None;
		self.printable = None;
		self.discard = None;
	}

	fn take_buffer(&mut self) -> Vec<u8> {
		self.search_offset = 0;
		self.paste_start_offset = 0;
		self.flush_at = None;
		self.flush_deferred = false;
		self.hold_since = None;
		mem::take(&mut self.buffer)
	}
}

impl InputBuffer {
	fn scan_buffer(
		&mut self,
		now: Duration,
		end_of_read: bool,
		events: &mut Vec<InputEvent>,
	) -> Option<Vec<u8>> {
		// The existing terminal gives bracketed paste priority over an
		// unfinished prefix, including a bare Escape immediately before paste.
		// Search only new bytes plus the marker overlap, even inside a large
		// buffered OSC, so this compatibility rule stays linear over chunks.
		let start = self
			.paste_start_offset
			.saturating_sub(PASTE_START.len() - 1);
		if let Some(marker) = self.buffer[start..]
			.windows(PASTE_START.len())
			.position(|window| window == PASTE_START)
		{
			let marker = start + marker;
			let bytes = self.take_buffer();
			self.buffer = bytes[..marker].to_vec();
			self.search_offset = 0;
			self.paste_start_offset = 0;
			let _ignored = self.scan_protocol_buffer(now, events);
			drop(self.take_buffer());
			self.discard = None;
			self.printable = None;
			self.paste = Some(TimedBytes { bytes: Vec::new(), at: now });
			return self.consume_paste(&bytes[marker + PASTE_START.len()..], now, end_of_read, events);
		}
		self.paste_start_offset = self.buffer.len();
		self.scan_protocol_buffer(now, events)
	}

	fn scan_protocol_buffer(
		&mut self,
		now: Duration,
		events: &mut Vec<InputEvent>,
	) -> Option<Vec<u8>> {
		let mut position = 0;
		while position < self.buffer.len() {
			if self.buffer[position] != 0x1b {
				let Some(length) = scalar_length(&self.buffer[position..]) else {
					break;
				};
				let bytes = self.buffer[position..position + length].to_vec();
				self.emit_data(bytes, now, events);
				position += length;
				self.search_offset = 0;
				continue;
			}
			let double_escape = self.buffer.get(position + 1) == Some(&0x1b);
			if double_escape {
				let Some(&third) = self.buffer.get(position + 2) else {
					break;
				};
				if third != b'[' && third != b'O' {
					self.emit_data(vec![0x1b], now, events);
					position += 1;
					self.search_offset = 0;
					continue;
				}
			}
			let start = position + usize::from(double_escape);
			let hint = if position == 0 && !double_escape {
				self.search_offset
			} else {
				0
			};
			match escape_end(&self.buffer, start, hint, &self.options) {
				EscapeEnd::Incomplete => {
					self.search_offset = if is_string(&self.buffer[start..]) {
						self.buffer.len() - position
					} else {
						0
					};
					break;
				},
				EscapeEnd::Capped if !double_escape && is_string(&self.buffer[position..]) => {
					let bytes = self.take_buffer();
					self.search_offset = 0;
					self.paste_start_offset = 0;
					self.hold_since = None;
					self.discard =
						Some(Discard { bytes: 0, held_escape: false, at: now });
					return self
						.consume_discard(&bytes[position..], now)
						.and_then(|consumed| {
							(position + consumed < bytes.len())
								.then(|| bytes[position + consumed..].to_vec())
						});
				},
				EscapeEnd::Capped => {
					let end = self
						.buffer
						.len()
						.min(position + self.options.csi_byte_limit);
					self.emit_data(self.buffer[position..end].to_vec(), now, events);
					position = end;
					self.search_offset = 0;
				},
				EscapeEnd::Complete(end) => {
					if double_escape && self.buffer.get(position + 3) == Some(&b'<') {
						self.emit_data(vec![0x1b], now, events);
						self.emit_data(self.buffer[start..end].to_vec(), now, events);
					} else {
						self.emit_data(self.buffer[position..end].to_vec(), now, events);
					}
					position = end;
					self.search_offset = 0;
				},
			}
		}
		if position > 0 {
			self.buffer.drain(..position);
			self.paste_start_offset = self.paste_start_offset.saturating_sub(position);
		}
		if self.buffer.is_empty() {
			self.hold_since = None;
			self.flush_at = None;
			self.flush_deferred = false;
			// A large completed OSC must not retain its allocation forever.
			if self.buffer.capacity() > MAX_INPUT_CHUNK {
				self.buffer.shrink_to_fit();
			}
		} else {
			self.flush_at = Some(now.saturating_add(self.options.timeout));
		}
		None
	}
}

enum EscapeEnd {
	Incomplete,
	Capped,
	Complete(usize),
}
fn escape_end(buffer: &[u8], position: usize, resume: usize, options: &InputOptions) -> EscapeEnd {
	let Some(&next) = buffer.get(position + 1) else {
		return EscapeEnd::Incomplete;
	};
	match next {
		0x1b => EscapeEnd::Incomplete,
		b'[' => {
			let Some(&third) = buffer.get(position + 2) else {
				return EscapeEnd::Incomplete;
			};
			if third == b'M' {
				return if buffer.len() >= position + 6 {
					EscapeEnd::Complete(position + 6)
				} else {
					EscapeEnd::Incomplete
				};
			}
			let limit = buffer.len().min(position + options.csi_byte_limit);
			for (index, &byte) in buffer.iter().enumerate().take(limit).skip(position + 2) {
				if (0x40..=0x7e).contains(&byte)
					&& (third != b'<'
						|| ((byte == b'M' || byte == b'm') && sgr_body(&buffer[position + 3..index])))
				{
					return EscapeEnd::Complete(index + 1);
				}
			}
			if buffer.len() - position >= options.csi_byte_limit {
				EscapeEnd::Capped
			} else {
				EscapeEnd::Incomplete
			}
		},
		b']' | b'P' | b'_' => {
			let start = (position + 2).max(resume.saturating_sub(1));
			let limit = buffer.len().min(position + options.string_byte_limit);
			for index in start..limit {
				if next == b']' && buffer[index] == 7 {
					return EscapeEnd::Complete(index + 1);
				}
				if buffer[index] == 0x1b && index + 1 < limit && buffer[index + 1] == b'\\' {
					return EscapeEnd::Complete(index + 2);
				}
			}
			if buffer.len() - position >= options.string_byte_limit {
				EscapeEnd::Capped
			} else {
				EscapeEnd::Incomplete
			}
		},
		b'O' => {
			if buffer.len() >= position + 3 {
				EscapeEnd::Complete(position + 3)
			} else {
				EscapeEnd::Incomplete
			}
		},
		_ => scalar_length(&buffer[position + 1..])
			.map_or(EscapeEnd::Incomplete, |length| EscapeEnd::Complete(position + 1 + length)),
	}
}

fn sgr_body(body: &[u8]) -> bool {
	let mut fields = body.split(|byte| *byte == b';');
	for _ in 0..3 {
		let Some(field) = fields.next() else {
			return false;
		};
		if field.is_empty() || !field.iter().all(u8::is_ascii_digit) {
			return false;
		}
	}
	fields.next().is_none()
}

fn is_string(buffer: &[u8]) -> bool {
	buffer.first() == Some(&0x1b) && matches!(buffer.get(1), Some(b']' | b'P' | b'_'))
}

fn scalar_length(bytes: &[u8]) -> Option<usize> {
	let &first = bytes.first()?;
	let length = match first {
		0..=0x7f => 1,
		0xc2..=0xdf => 2,
		0xe0..=0xef => 3,
		0xf0..=0xf4 => 4,
		_ => return Some(1),
	};
	let available = bytes.len().min(length);
	if bytes[1..available]
		.iter()
		.any(|byte| !(0x80..=0xbf).contains(byte))
	{
		return Some(1);
	}
	if bytes.len() < length {
		return None;
	}
	Some(if std::str::from_utf8(&bytes[..length]).is_ok() {
		length
	} else {
		1
	})
}

fn split_scalars(mut bytes: &[u8]) -> Vec<Vec<u8>> {
	let mut output = Vec::new();
	while !bytes.is_empty() {
		let length = scalar_length(bytes).unwrap_or(bytes.len());
		output.push(bytes[..length].to_vec());
		bytes = &bytes[length..];
	}
	output
}

fn unmodified_printable(bytes: &[u8]) -> Option<u32> {
	if !bytes.ends_with(b"u") || bytes.contains(&b';') {
		return None;
	}
	let codepoint = u32::try_from(parse_kitty_sequence(bytes)?.codepoint).ok()?;
	(codepoint >= 32).then_some(codepoint)
}

enum Enter {
	Pending,
	Complete(Option<usize>),
}
fn leading_enter(bytes: &[u8], end_of_read: bool) -> Enter {
	if matches!(bytes.first(), Some(b'\r' | b'\n')) {
		return Enter::Complete(Some(1));
	}
	for sequence in [b"\x1b[13u".as_slice(), b"\x1b[13;1u"] {
		if bytes.starts_with(sequence) {
			return Enter::Complete(Some(sequence.len()));
		}
	}
	if !end_of_read
		&& [b"\x1b[13u".as_slice(), b"\x1b[13;1u"]
			.iter()
			.any(|sequence| sequence.starts_with(bytes))
	{
		Enter::Pending
	} else {
		Enter::Complete(None)
	}
}

impl InputBuffer {
	/// A transport read may be split into bounded chunks; only its final chunk
	/// sets `end_of_read`. This preserves paste-and-Enter atomicity across
	/// chunks.
	/// Feed one bounded piece of a read. Only its final piece sets
	/// `end_of_read`. Oversized pieces are rejected without consuming input or
	/// expiring timers.
	pub fn feed(
		&mut self,
		data: &[u8],
		now: Duration,
		end_of_read: bool,
	) -> Result<Vec<InputEvent>, InputError> {
		if data.len() > MAX_INPUT_CHUNK {
			return Err(InputError::ChunkTooLarge);
		}
		let mut events = Vec::new();
		let mut input = Cow::Borrowed(data);
		if self.pending_paste.is_some() {
			let pending = self.pending_paste.take().unwrap();
			let mut suffix = pending.suffix;
			suffix.extend_from_slice(data);
			match self.finish_paste(pending.content, &suffix, now, end_of_read, &mut events) {
				Some(rest) => input = Cow::Owned(rest),
				None => return Ok(events),
			}
		}
		loop {
			if self.discard.is_some() {
				let Some(consumed) = self.consume_discard(&input, now) else {
					break;
				};
				input = Cow::Owned(input[consumed..].to_vec());
				if input.is_empty() {
					break;
				}
			}
			let fresh_escape = self.flush_deferred
				&& !self.buffer.is_empty()
				&& input.starts_with(b"\x1b")
				&& !(input.starts_with(b"\x1b\\") && is_string(&self.buffer));
			self.flush_at = None;
			self.flush_deferred = false;
			if fresh_escape {
				self.flush_partial(now, false, &mut events);
			}
			if input.is_empty() && self.buffer.is_empty() && self.raw.is_none() {
				self.emit_data(Vec::new(), now, &mut events);
				break;
			}
			if self.paste.is_some() {
				match self.consume_paste(&input, now, end_of_read, &mut events) {
					Some(rest) => {
						input = Cow::Owned(rest);
						continue;
					},
					None => break,
				}
			}
			if self.raw.is_some() {
				if input.contains(&0x1b) {
					self.flush_raw(now, false, &mut events);
				} else {
					let raw = self.raw.as_mut().unwrap();
					let classified = raw.append(&input);
					if raw.bytes.len() > self.options.paste_byte_limit || classified {
						self.flush_raw(now, true, &mut events);
					}
					break;
				}
			}
			if self.buffer.is_empty()
				&& !input.contains(&0x1b)
				&& (input.contains(&b'\r') || input.contains(&b'\n'))
			{
				let mut raw = RawCandidate::new(now);
				let classified = raw.append(&input);
				self.raw = Some(raw);
				if input.len() > self.options.paste_byte_limit || classified {
					self.flush_raw(now, true, &mut events);
				}
				break;
			}
			self.buffer.extend_from_slice(&input);
			match self.scan_buffer(now, end_of_read, &mut events) {
				Some(rest) => {
					input = Cow::Owned(rest);
				},
				None => break,
			}
		}
		Ok(events)
	}

	/// Run after queued input has had a chance to arrive. Escape expiry takes
	/// two timer turns; the first returns a zero delay for an I/O-poll deferral.
	pub fn tick(&mut self, now: Duration) -> Vec<InputEvent> {
		let mut events = Vec::new();
		self.expire_auxiliary(now, &mut events);
		if self.flush_at.is_some_and(|deadline| now >= deadline) {
			if self.flush_deferred {
				self.flush_at = None;
				self.flush_deferred = false;
				self.flush_partial(now, true, &mut events);
			} else {
				self.flush_deferred = true;
			}
		}
		events
	}

	/// Delay until the next required host timer turn, or `None` when idle.
	pub fn next_delay(&self, now: Duration) -> Option<Duration> {
		[
			self.flush_at,
			self
				.raw
				.as_ref()
				.map(|raw| raw.at.saturating_add(RAW_WINDOW)),
			self
				.paste
				.as_ref()
				.map(|paste| paste.at.saturating_add(self.options.paste_timeout)),
			self
				.pending_paste
				.as_ref()
				.map(|pending| pending.at.saturating_add(self.options.paste_timeout)),
			self
				.discard
				.as_ref()
				.map(|discard| discard.at.saturating_add(DISCARD_TIMEOUT)),
		]
		.into_iter()
		.flatten()
		.min()
		.map(|deadline| deadline.saturating_sub(now))
	}

	/// Explicit flush returns data for the caller to dispatch. Paste recovery
	/// remains on its separate watchdog, matching the existing terminal API.
	/// Return pending keys, preserving active paste and torn-string recovery.
	pub fn flush(&mut self) -> Vec<Vec<u8>> {
		self.drain_buffer()
	}

	fn expire_auxiliary(&mut self, now: Duration, events: &mut Vec<InputEvent>) {
		if self
			.raw
			.as_ref()
			.is_some_and(|raw| now >= raw.at.saturating_add(RAW_WINDOW))
		{
			self.flush_raw(now, false, events);
		}
		if self
			.paste
			.as_ref()
			.is_some_and(|paste| now >= paste.at.saturating_add(self.options.paste_timeout))
		{
			let content = self.paste.take().unwrap().bytes;
			self.printable = None;
			events.push(InputEvent::Paste { content, enter: None });
		}
		if self
			.pending_paste
			.as_ref()
			.is_some_and(|pending| now >= pending.at.saturating_add(self.options.paste_timeout))
		{
			let pending = self.pending_paste.take().unwrap();
			events.push(InputEvent::Paste { content: pending.content, enter: None });
			if !pending.suffix.is_empty() {
				self.buffer.extend_from_slice(&pending.suffix);
				self.flush_at = Some(now.saturating_add(self.options.timeout));
			}
		}
		if self
			.discard
			.as_ref()
			.is_some_and(|discard| now >= discard.at.saturating_add(DISCARD_TIMEOUT))
		{
			self.discard = None;
		}
	}

	fn flush_raw(&mut self, now: Duration, paste: bool, events: &mut Vec<InputEvent>) {
		let Some(raw) = self.raw.take() else {
			return;
		};
		if paste {
			self.printable = None;
			events.push(InputEvent::Paste { content: raw.bytes, enter: None });
		} else {
			for scalar in split_scalars(&raw.bytes) {
				self.emit_data(scalar, now, events);
			}
		}
	}

	fn emit_data(&mut self, bytes: Vec<u8>, now: Duration, events: &mut Vec<InputEvent>) {
		let scalar = std::str::from_utf8(&bytes).ok().and_then(|text| {
			let mut chars = text.chars();
			let first = chars.next()?;
			(chars.next().is_none() && first.len_utf16() == 1).then_some(u32::from(first))
		});
		if self.printable.is_some_and(|(pending, at)| {
			Some(pending) == scalar && now.saturating_sub(at) <= DEDUP_WINDOW
		}) {
			self.printable = None;
			return;
		}
		self.printable = unmodified_printable(&bytes).map(|codepoint| (codepoint, now));
		events.push(InputEvent::Data(bytes));
	}

	fn consume_paste(
		&mut self,
		input: &[u8],
		now: Duration,
		end_of_read: bool,
		events: &mut Vec<InputEvent>,
	) -> Option<Vec<u8>> {
		let mut paste = self.paste.take().unwrap();
		let overlap = paste.bytes.len().min(PASTE_END.len() - 1);
		let mut probe = paste.bytes[paste.bytes.len() - overlap..].to_vec();
		probe.extend_from_slice(input);
		if let Some(marker) = probe
			.windows(PASTE_END.len())
			.position(|window| window == PASTE_END)
		{
			let end = paste.bytes.len() - overlap + marker;
			if end < paste.bytes.len() {
				paste.bytes.truncate(end);
			} else {
				paste
					.bytes
					.extend_from_slice(&input[..end - paste.bytes.len()]);
			}
			let remaining = &input[marker + PASTE_END.len() - overlap..];
			self.printable = None;
			return self.finish_paste(paste.bytes, remaining, now, end_of_read, events);
		}
		paste.bytes.extend_from_slice(input);
		paste.at = now;
		if paste.bytes.len() > self.options.paste_byte_limit {
			self.printable = None;
			events.push(InputEvent::Paste { content: paste.bytes, enter: None });
		} else {
			self.paste = Some(paste);
		}
		None
	}

	fn finish_paste(
		&mut self,
		content: Vec<u8>,
		remaining: &[u8],
		now: Duration,
		end_of_read: bool,
		events: &mut Vec<InputEvent>,
	) -> Option<Vec<u8>> {
		match leading_enter(remaining, end_of_read) {
			Enter::Pending => {
				self.pending_paste =
					Some(PendingPaste { content, suffix: remaining.to_vec(), at: now });
				None
			},
			Enter::Complete(length) => {
				let consumed = length.unwrap_or(0);
				let enter = length.map(|length| remaining[..length].to_vec());
				events.push(InputEvent::Paste { content, enter });
				(remaining.len() > consumed).then(|| remaining[consumed..].to_vec())
			},
		}
	}

	fn consume_discard(&mut self, input: &[u8], now: Duration) -> Option<usize> {
		let mut discard = self.discard.take().unwrap();
		if discard.held_escape && input.first() == Some(&b'\\') {
			return Some(1);
		}
		if !input.is_empty() {
			discard.held_escape = false;
		}
		for (position, &byte) in input.iter().enumerate() {
			if byte == 7 {
				return Some(position + 1);
			}
			if byte == 0x1b {
				if input.get(position + 1) == Some(&b'\\') {
					return Some(position + 2);
				}
				if position + 1 == input.len() {
					discard.held_escape = true;
				}
			}
		}
		discard.bytes = discard.bytes.saturating_add(input.len());
		discard.at = now;
		if discard.bytes <= self.options.string_byte_limit.saturating_mul(2) {
			self.discard = Some(discard);
		}
		None
	}

	fn flush_partial(&mut self, now: Duration, discard_torn: bool, events: &mut Vec<InputEvent>) {
		if self.buffer.is_empty() {
			self.hold_since = None;
			return;
		}
		let mouse = self.buffer.starts_with(b"\x1b[<")
			&& self.buffer[3..]
				.iter()
				.all(|byte| byte.is_ascii_digit() || *byte == b';');
		if mouse || self.mode.kitty_protocol_active {
			let since = *self.hold_since.get_or_insert(now);
			if now.saturating_sub(since) < self.options.partial_hold_timeout {
				self.flush_at = Some(now.saturating_add(self.options.timeout));
				return;
			}
		}
		let torn = discard_torn && self.mode.kitty_protocol_active && is_string(&self.buffer);
		for data in self.drain_buffer() {
			self.emit_data(data, now, events);
		}
		if torn {
			self.discard = Some(Discard { bytes: 0, held_escape: false, at: now });
		}
	}

	fn drain_buffer(&mut self) -> Vec<Vec<u8>> {
		self.flush_at = None;
		self.flush_deferred = false;
		self.hold_since = None;
		self.search_offset = 0;
		self.paste_start_offset = 0;
		self.printable = None;
		let mut output = self
			.raw
			.take()
			.map_or_else(Vec::new, |raw| split_scalars(&raw.bytes));
		let bytes = self.take_buffer();
		if bytes == b"\x1b\x1b" {
			output.push(vec![0x1b]);
			output.push(vec![0x1b]);
		} else if !(bytes.is_empty() || self.mode.kitty_protocol_active && is_string(&bytes)) {
			output.push(bytes);
		}
		output
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn at(milliseconds: u64) -> Duration {
		Duration::from_millis(milliseconds)
	}
	fn decoder() -> InputBuffer {
		InputBuffer::new(InputOptions::default(), InputMode::default()).unwrap()
	}
	fn data(bytes: &[u8]) -> InputEvent {
		InputEvent::Data(bytes.to_vec())
	}
	fn paste(bytes: &[u8], enter: Option<&[u8]>) -> InputEvent {
		InputEvent::Paste { content: bytes.to_vec(), enter: enter.map(<[u8]>::to_vec) }
	}

	#[test]
	fn fragmented_terminal_protocols_are_never_replayed_as_tail_keystrokes() {
		for sequence in [
			b"\x1b[<35;20;5m".as_slice(),
			b"\x1b[M !!",
			b"\x1b]52;text\x07",
			b"\x1bPdata\x1b\\",
			b"\x1b_graphics\x1b\\",
			b"\x1bOP",
		] {
			for boundary in 1..sequence.len() {
				let mut input = decoder();
				assert_eq!(input.feed(&sequence[..boundary], at(0), true).unwrap(), vec![]);
				assert_eq!(
					input.feed(&sequence[boundary..], at(1), true).unwrap(),
					vec![data(sequence)],
					"split at {boundary} in {sequence:?}"
				);
				assert_eq!(input.retained_bytes(), 0);
			}
		}
	}

	#[test]
	fn fragmented_utf8_produces_one_scalar_without_replacement_bytes() {
		let mut input = decoder();
		let encoded = "😀".as_bytes();
		for byte in &encoded[..3] {
			assert!(input.feed(&[*byte], at(0), true).unwrap().is_empty());
		}
		assert_eq!(input.feed(&encoded[3..], at(1), true).unwrap(), vec![data(encoded)]);
	}

	#[test]
	fn paste_priority_discards_an_incomplete_escape_prefix_without_turning_paste_into_keys() {
		let mut input = decoder();
		assert!(input.feed(b"\x1b", at(0), true).unwrap().is_empty());
		assert_eq!(input.feed(b"\x1b[200~text\x1b[201~", at(1), true).unwrap(), vec![paste(
			b"text", None
		)]);
	}

	#[test]
	fn explicit_zero_caps_deliver_paste_immediately_and_zero_timeouts_still_defer_io_poll() {
		let mut input = InputBuffer::new(
			InputOptions {
				timeout: Duration::ZERO,
				partial_hold_timeout: Duration::ZERO,
				paste_byte_limit: 0,
				..InputOptions::default()
			},
			InputMode::default(),
		)
		.unwrap();
		assert!(input.feed(b"\x1b", at(0), true).unwrap().is_empty());
		assert!(input.tick(at(0)).is_empty());
		assert_eq!(input.tick(at(0)), vec![data(b"\x1b")]);
		assert_eq!(input.feed(b"\x1b[200~x", at(1), true).unwrap(), vec![paste(b"x", None)]);
	}

	#[test]
	fn completed_meta_sequences_cancel_the_old_partial_hold_watchdog() {
		let mut input = InputBuffer::new(InputOptions::default(), InputMode {
			kitty_protocol_active: true,
			..InputMode::default()
		})
		.unwrap();
		input.feed(b"\x1b", at(0), true).unwrap();
		input.tick(at(75));
		assert_eq!(input.feed(b"\x1b[A", at(75), true).unwrap(), vec![data(b"\x1b\x1b[A")]);
		assert_eq!(input.next_delay(at(75)), None);
	}

	#[test]
	fn synchronous_reads_can_complete_paste_before_a_zero_delay_watchdog_turn() {
		let mut input = InputBuffer::new(
			InputOptions { paste_timeout: Duration::ZERO, ..InputOptions::default() },
			InputMode::default(),
		)
		.unwrap();
		assert!(input.feed(b"\x1b[200~one", at(0), true).unwrap().is_empty());
		assert_eq!(input.feed(b"two\x1b[201~", at(1), true).unwrap(), vec![paste(b"onetwo", None)]);
		assert!(input.tick(at(1)).is_empty());
	}

	#[test]
	fn double_escape_distinguishes_two_keys_meta_arrows_and_mouse_reports() {
		let mut input = decoder();
		assert!(input.feed(b"\x1b\x1b", at(0), true).unwrap().is_empty());
		assert!(input.tick(at(75)).is_empty());
		assert_eq!(input.tick(at(75)), vec![data(b"\x1b"), data(b"\x1b")]);
		assert_eq!(input.feed(b"\x1b\x1bd", at(76), true).unwrap(), vec![
			data(b"\x1b"),
			data(b"\x1bd")
		]);
		assert_eq!(input.feed(b"\x1b\x1b[A", at(77), true).unwrap(), vec![data(b"\x1b\x1b[A")]);
		assert_eq!(input.feed(b"\x1b\x1b[<1;2;3M", at(78), true).unwrap(), vec![
			data(b"\x1b"),
			data(b"\x1b[<1;2;3M")
		]);
	}

	#[test]
	fn paste_and_same_read_enter_remain_atomic_across_bounded_chunks() {
		let mut input = decoder();
		assert!(
			input
				.feed(b"\x1b[200~one\x1b[20", at(0), false)
				.unwrap()
				.is_empty()
		);
		assert!(input.feed(b"1~\x1b[13", at(0), false).unwrap().is_empty());
		assert_eq!(input.feed(b"uafter", at(0), true).unwrap(), vec![
			paste(b"one", Some(b"\x1b[13u")),
			data(b"a"),
			data(b"f"),
			data(b"t"),
			data(b"e"),
			data(b"r")
		]);
		assert!(input.feed(b"\r", at(1), true).unwrap().is_empty());
		assert_eq!(input.tick(at(11)), vec![data(b"\r")]);
		assert_eq!(
			input
				.feed(b"\x1b[200~two\x1b[201~\x1b[13;2u", at(12), true)
				.unwrap(),
			vec![paste(b"two", None), data(b"\x1b[13;2u")]
		);
	}

	#[test]
	fn lost_paste_markers_recover_without_losing_payload_or_eating_future_keys() {
		let mut input = InputBuffer::new(
			InputOptions { paste_timeout: at(10), paste_byte_limit: 8, ..InputOptions::default() },
			InputMode::default(),
		)
		.unwrap();
		assert!(
			input
				.feed(b"\x1b[200~12345", at(0), true)
				.unwrap()
				.is_empty()
		);
		assert!(input.feed(b"67", at(9), true).unwrap().is_empty());
		assert!(input.tick(at(10)).is_empty());
		assert_eq!(input.tick(at(19)), vec![paste(b"1234567", None)]);
		assert_eq!(input.feed(b"x", at(20), true).unwrap(), vec![data(b"x")]);
		assert_eq!(input.feed(b"\x1b[200~012345678", at(30), true).unwrap(), vec![paste(
			b"012345678",
			None
		)]);
		assert_eq!(input.feed(b"q", at(31), true).unwrap(), vec![data(b"q")]);
	}

	#[test]
	fn paste_caps_count_real_utf8_bytes() {
		let mut input = InputBuffer::new(
			InputOptions { paste_byte_limit: 5, ..InputOptions::default() },
			InputMode::default(),
		)
		.unwrap();
		assert_eq!(input.feed("\x1b[200~你好".as_bytes(), at(0), true).unwrap(), vec![paste(
			"你好".as_bytes(),
			None
		)]);
		assert_eq!(input.retained_bytes(), 0);
	}

	#[test]
	fn raw_multiline_bursts_coalesce_but_a_single_enter_keeps_its_fixed_deadline() {
		let mut input = decoder();
		assert!(input.feed(b"a\rb\r", at(0), true).unwrap().is_empty());
		assert_eq!(input.feed(b"c", at(6), true).unwrap(), vec![paste(b"a\rb\rc", None)]);
		assert!(input.feed(b"\r", at(20), true).unwrap().is_empty());
		assert!(input.feed(b"x", at(29), true).unwrap().is_empty());
		assert_eq!(input.tick(at(30)), vec![data(b"\r"), data(b"x")]);
		assert!(input.feed(b"a\r", at(40), true).unwrap().is_empty());
		assert_eq!(input.feed(b"\x1b[A", at(41), true).unwrap(), vec![
			data(b"a"),
			data(b"\r"),
			data(b"\x1b[A")
		]);
	}

	#[test]
	fn kitty_duplicate_filter_only_swallows_one_immediate_bare_report() {
		let mut input = decoder();
		assert_eq!(input.feed(b"\x1b[97u", at(0), true).unwrap(), vec![data(b"\x1b[97u")]);
		assert!(input.feed(b"a", at(20), true).unwrap().is_empty());
		assert_eq!(input.feed(b"a", at(21), true).unwrap(), vec![data(b"a")]);
		assert_eq!(input.feed(b"\x1b[98u", at(30), true).unwrap(), vec![data(b"\x1b[98u")]);
		assert_eq!(input.feed(b"b", at(56), true).unwrap(), vec![data(b"b")]);
	}

	#[test]
	fn timeout_deferral_joins_queued_tails_but_keeps_a_fresh_escape_separate() {
		let mut input = decoder();
		assert!(input.feed(b"\x1b[?", at(0), true).unwrap().is_empty());
		assert!(input.tick(at(75)).is_empty());
		assert_eq!(input.next_delay(at(75)), Some(Duration::ZERO));
		assert_eq!(input.feed(b"1;2c", at(75), true).unwrap(), vec![data(b"\x1b[?1;2c")]);
		assert_eq!(input.next_delay(at(75)), None);
		assert!(input.feed(b"\x1b[?", at(100), true).unwrap().is_empty());
		assert!(input.tick(at(175)).is_empty());
		assert_eq!(input.feed(b"\x1b[B", at(175), true).unwrap(), vec![
			data(b"\x1b[?"),
			data(b"\x1b[B")
		]);
	}

	#[test]
	fn unambiguous_mouse_partials_get_a_bounded_hold_window() {
		let mut input = InputBuffer::new(
			InputOptions { timeout: at(10), partial_hold_timeout: at(20), ..InputOptions::default() },
			InputMode::default(),
		)
		.unwrap();
		assert!(input.feed(b"\x1b[<35;", at(0), true).unwrap().is_empty());
		assert!(input.tick(at(10)).is_empty());
		assert!(input.tick(at(10)).is_empty());
		assert_eq!(input.feed(b"8;9M", at(15), true).unwrap(), vec![data(b"\x1b[<35;8;9M")]);
		assert!(input.feed(b"\x1b[<", at(20), true).unwrap().is_empty());
		for time in [30, 40] {
			assert!(input.tick(at(time)).is_empty());
			assert!(input.tick(at(time)).is_empty());
		}
		assert!(input.tick(at(50)).is_empty());
		assert_eq!(input.tick(at(50)), vec![data(b"\x1b[<")]);
	}

	#[test]
	fn oversized_strings_discard_their_tail_through_a_split_terminator() {
		let mut input = InputBuffer::new(
			InputOptions { string_byte_limit: 16, ..InputOptions::default() },
			InputMode::default(),
		)
		.unwrap();
		assert!(
			input
				.feed(b"\x1b]52;01234567890", at(0), true)
				.unwrap()
				.is_empty()
		);
		assert_eq!(input.retained_bytes(), 0);
		assert!(input.feed(b"payload\x1b", at(1), true).unwrap().is_empty());
		assert_eq!(input.feed(b"\\x", at(2), true).unwrap(), vec![data(b"x")]);
	}

	#[test]
	fn discard_budget_and_watchdog_recover_a_terminator_less_stream() {
		let mut input = InputBuffer::new(
			InputOptions { string_byte_limit: 8, ..InputOptions::default() },
			InputMode::default(),
		)
		.unwrap();
		assert!(input.feed(b"\x1b_123456", at(0), true).unwrap().is_empty());
		assert!(
			input
				.feed(b"more than the remaining budget", at(1), true)
				.unwrap()
				.is_empty()
		);
		assert_eq!(input.feed(b"x", at(2), true).unwrap(), vec![data(b"x")]);
		assert!(input.feed(b"\x1b_123456", at(3), true).unwrap().is_empty());
		assert!(input.tick(at(1003)).is_empty());
		assert_eq!(input.feed(b"y", at(1003), true).unwrap(), vec![data(b"y")]);
	}

	#[test]
	fn timed_out_kitty_strings_drop_protocol_payload_until_the_terminator() {
		let mut input = InputBuffer::new(
			InputOptions { partial_hold_timeout: Duration::ZERO, ..InputOptions::default() },
			InputMode { kitty_protocol_active: true, ..InputMode::default() },
		)
		.unwrap();
		assert!(
			input
				.feed(b"\x1b]52;image", at(0), true)
				.unwrap()
				.is_empty()
		);
		assert!(input.tick(at(75)).is_empty());
		assert!(input.tick(at(75)).is_empty());
		assert_eq!(input.feed(b"base64\x1b\\z", at(76), true).unwrap(), vec![data(b"z")]);
	}

	#[test]
	fn a_rejected_oversized_chunk_cannot_consume_or_expire_buffered_input() {
		let mut input = decoder();
		input.feed(b"\x1b[?", at(0), true).unwrap();
		assert_eq!(
			input.feed(&vec![b'x'; MAX_INPUT_CHUNK + 1], at(100), true),
			Err(InputError::ChunkTooLarge)
		);
		assert_eq!(input.buffered_input(), b"\x1b[?");
		assert_eq!(input.feed(b"1c", at(100), true).unwrap(), vec![data(b"\x1b[?1c")]);
	}

	#[test]
	fn diagnostics_do_not_disclose_unsubmitted_text_or_paste_payloads() {
		let mut buffer = decoder();
		buffer.feed(b"secret-draft\r", at(0), true).unwrap();
		assert!(!format!("{buffer:?}").contains("secret-draft"));
		buffer.clear();
		buffer.feed(b"\x1b[200~secret-paste", at(1), true).unwrap();
		assert!(!format!("{buffer:?}").contains("secret-paste"));
		assert_eq!(buffer.tick(at(1001)), [paste(b"secret-paste", None)]);
	}

	#[test]
	fn clearing_active_paste_releases_data_and_all_watchdogs() {
		let mut input = decoder();
		input.feed(b"\x1b[200~payload", at(0), true).unwrap();
		input.clear();
		assert_eq!(input.retained_bytes(), 0);
		assert_eq!(input.next_delay(at(0)), None);
		assert_eq!(input.feed(b"x", at(1), true).unwrap(), vec![data(b"x")]);
	}
}
