//! Thin terminal transport for the existing bounded framing/key owners.
//!
//! Unix crossterm events accumulate a bracketed paste before returning it. Read
//! fixed-size byte chunks instead, so InputBuffer applies its cap while input
//! is arriving. Windows crossterm reads a fixed console-record batch and has no
//! corresponding unbounded byte/paste accumulator.

use std::{
	io,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	thread::JoinHandle,
	time::Duration,
};
#[cfg(unix)]
use std::{
	os::fd::{AsRawFd, RawFd},
	time::Instant,
};

#[cfg(unix)]
use harvest_tui::terminal::TerminalGuard;
use harvest_tui::{editor::MAX_EDITOR_BYTES, keyparse::Key};
#[cfg(any(unix, test))]
use harvest_tui::{
	input::{InputBuffer, InputEvent, InputOptions},
	keys::InputMode,
};
use tokio::sync::mpsc;

use super::Input;

#[cfg(any(unix, test))]
const READ_BYTES: usize = 4096;
const IDLE_POLL: Duration = Duration::from_millis(100);

#[cfg(any(unix, test))]
struct Decoder {
	buffer: InputBuffer,
	mode:   InputMode,
}

#[cfg(any(unix, test))]
impl Decoder {
	fn new() -> io::Result<Self> {
		let mode = InputMode::default();
		let mut buffer = InputBuffer::new(
			InputOptions {
				paste_byte_limit: MAX_EDITOR_BYTES,
				string_byte_limit: harvest_tui::MAX_CSI_BYTES,
				..InputOptions::default()
			},
			mode,
		)
		.map_err(io::Error::other)?;
		buffer.require_complete_pastes();
		Ok(Self { buffer, mode })
	}

	fn feed(&mut self, bytes: &[u8], now: Duration) -> io::Result<Vec<Input>> {
		if bytes.len() > READ_BYTES {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"terminal read exceeds fixed input chunk limit",
			));
		}
		let events = self
			.buffer
			.feed(bytes, now, true)
			.map_err(io::Error::other)?;
		self.decode(events)
	}

	fn tick(&mut self, now: Duration) -> io::Result<Vec<Input>> {
		let events = self.buffer.tick(now);
		self.decode(events)
	}

	fn decode(&self, events: Vec<InputEvent>) -> io::Result<Vec<Input>> {
		// Inspect the entire bounded batch before publishing any input. On
		// overflow the native reader closes, so no rejected-paste tail or paired
		// Enter can submit a stale draft or approve a pending tool call.
		if let Some(reason) = self.buffer.paste_failure() {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				format!("{reason}; interactive input closed without submitting the paste"),
			));
		}
		if events.iter().any(
			|event| matches!(event, InputEvent::Paste { content, .. } if content.len() > MAX_EDITOR_BYTES),
		) {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"paste exceeds the native editor byte limit; interactive input closed without \
				 submitting it",
			));
		}
		let mut output = Vec::new();
		let mut text = String::new();
		for event in events {
			match event {
				InputEvent::Data(bytes) => match Key::from_bytes(&bytes, self.mode) {
					Some(Key::Char(character)) => text.push(character),
					other => {
						flush_text(&mut text, &mut output);
						if let Some(key) = other {
							output.push(Input::Key(key));
						}
					},
				},
				InputEvent::Paste { content, enter } => {
					flush_text(&mut text, &mut output);
					let text = String::from_utf8(content).map_err(|_| {
						io::Error::new(
							io::ErrorKind::InvalidData,
							"terminal paste contains invalid UTF-8; interactive input closed without \
							 submitting it",
						)
					})?;
					output.push(Input::Paste { text, enter: enter.is_some() });
				},
			}
		}
		flush_text(&mut text, &mut output);
		Ok(output)
	}

	fn delay(&self, now: Duration) -> Duration {
		self
			.buffer
			.next_delay(now)
			.unwrap_or(IDLE_POLL)
			.min(IDLE_POLL)
	}
}

#[cfg(any(unix, test))]
fn flush_text(text: &mut String, output: &mut Vec<Input>) {
	if text.is_empty() {
		return;
	}
	let value = std::mem::take(text);
	let mut characters = value.chars();
	let first = characters.next().expect("nonempty text");
	if characters.next().is_none() {
		output.push(Input::Key(Key::Char(first)));
	} else {
		// A whole printable burst inserts atomically and cannot type an
		// accidental approval response one character at a time.
		output.push(Input::Paste { text: value, enter: false });
	}
}

pub(super) fn spawn_reader(
	sender: mpsc::Sender<Input>,
	stop: Arc<AtomicBool>,
) -> io::Result<JoinHandle<()>> {
	std::thread::Builder::new()
		.name("harvest-terminal-input".into())
		.spawn(move || {
			#[cfg(unix)]
			let result = read_unix(io::stdin().as_raw_fd(), &sender, &stop);
			#[cfg(windows)]
			let result = read_windows(&sender, &stop);
			#[cfg(not(any(unix, windows)))]
			let result: io::Result<()> = Err(io::Error::new(
				io::ErrorKind::Unsupported,
				"native terminal input is supported on Unix and Windows",
			));
			if let Err(error) = result {
				let _ = sender.blocking_send(Input::Fatal(error.to_string()));
			}
		})
}

#[cfg(unix)]
fn read_unix(fd: RawFd, sender: &mpsc::Sender<Input>, stop: &AtomicBool) -> io::Result<()> {
	let clock = Instant::now();
	let mut decoder = Decoder::new()?;
	let mut bytes = [0; READ_BYTES];
	let mut size = TerminalGuard::size();
	while !stop.load(Ordering::Acquire) {
		let events = match read_ready(fd, &mut bytes, decoder.delay(clock.elapsed()))? {
			Some(0) => return Ok(()),
			Some(count) => decoder.feed(&bytes[..count], clock.elapsed())?,
			None => decoder.tick(clock.elapsed())?,
		};
		for input in events {
			if sender.blocking_send(input).is_err() {
				return Ok(());
			}
		}
		let current = TerminalGuard::size();
		if current != size {
			size = current;
			if sender.blocking_send(Input::Resize).is_err() {
				return Ok(());
			}
		}
	}
	Ok(())
}

#[cfg(unix)]
fn read_ready(fd: RawFd, bytes: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
	let milliseconds = timeout.as_nanos().saturating_add(999_999) / 1_000_000;
	let timeout = i32::try_from(milliseconds).unwrap_or(i32::MAX);
	let mut descriptor = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
	// SAFETY: descriptor points to one initialized pollfd for a borrowed live
	// descriptor. Poll neither transfers its ownership nor changes its flags.
	let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
	if ready < 0 {
		let error = io::Error::last_os_error();
		return if error.kind() == io::ErrorKind::Interrupted {
			Ok(None)
		} else {
			Err(error)
		};
	}
	if ready == 0 {
		return Ok(None);
	}
	if descriptor.revents & (libc::POLLNVAL | libc::POLLERR) != 0 {
		return Err(io::Error::other("terminal input descriptor failed"));
	}
	// SAFETY: bytes is writable for its full length. The interactive UI is the
	// sole stdin reader, so readiness cannot be consumed by a competing reader.
	// A direct read avoids a hidden stdio buffer bypassing subsequent poll calls.
	let count = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
	if count < 0 {
		let error = io::Error::last_os_error();
		if matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) {
			return Ok(None);
		}
		return Err(error);
	}
	Ok(Some(count as usize))
}

#[cfg(windows)]
fn read_windows(sender: &mpsc::Sender<Input>, stop: &AtomicBool) -> io::Result<()> {
	use crossterm::event::{self, Event};
	while !stop.load(Ordering::Acquire) {
		if !event::poll(IDLE_POLL)? {
			continue;
		}
		let input = match event::read()? {
			Event::Key(key) => Key::from_event(&key).map(Input::Key),
			Event::Paste(text) if text.len() <= MAX_EDITOR_BYTES => {
				Some(Input::Paste { text, enter: false })
			},
			Event::Paste(_) => {
				return Err(io::Error::other(
					"paste exceeds the native editor byte limit; interactive input closed without \
					 submitting it",
				));
			},
			Event::Resize(..) => Some(Input::Resize),
			_ => None,
		};
		if let Some(input) = input
			&& sender.blocking_send(input).is_err()
		{
			return Ok(());
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn split_unicode_keys_and_cancel_use_shared_framing_without_protocol_leaks() {
		let mut decoder = Decoder::new().unwrap();
		assert!(decoder.feed(&[0xe7], Duration::ZERO).unwrap().is_empty());
		assert_eq!(decoder.feed(&[0x95, 0x8c], Duration::ZERO).unwrap(), [Input::Key(Key::Char(
			'界'
		))]);
		assert!(decoder.feed(b"\x1b[", Duration::ZERO).unwrap().is_empty());
		assert_eq!(decoder.feed(b"D\x03", Duration::ZERO).unwrap(), [
			Input::Key(Key::Left),
			Input::Key(Key::Ctrl('c'))
		]);
		assert!(
			decoder
				.feed(b"\x1b[97;1:3u\x1b]52;clipboard\x07", Duration::ZERO)
				.unwrap()
				.is_empty()
		);
	}

	#[test]
	fn paste_and_same_read_enter_remain_one_editor_transaction() {
		let mut decoder = Decoder::new().unwrap();
		assert_eq!(
			decoder
				.feed(b"\x1b[200~one\r\ntwo\x1b[201~\r", Duration::ZERO)
				.unwrap(),
			[Input::Paste { text: "one\r\ntwo".into(), enter: true }]
		);
		assert_eq!(decoder.feed(b"yes", Duration::ZERO).unwrap(), [Input::Paste {
			text:  "yes".into(),
			enter: false,
		}]);
	}

	#[test]
	fn oversized_fragmented_paste_cannot_publish_a_prefix_or_its_submit_suffix() {
		let mut decoder = Decoder::new().unwrap();
		assert!(
			decoder
				.feed(b"\x1b[200~", Duration::ZERO)
				.unwrap()
				.is_empty()
		);
		let chunk = [b'x'; READ_BYTES];
		for _ in 0..MAX_EDITOR_BYTES / READ_BYTES {
			assert!(decoder.feed(&chunk, Duration::ZERO).unwrap().is_empty());
			assert!(decoder.buffer.retained_bytes() <= MAX_EDITOR_BYTES);
		}
		let error = decoder.feed(b"x\x1b[201~\ry", Duration::ZERO).unwrap_err();
		assert_eq!(error.kind(), io::ErrorKind::InvalidData);
		assert_eq!(decoder.buffer.retained_bytes(), 0);
	}

	#[cfg(unix)]
	#[tokio::test]
	async fn cancel_is_delivered_and_stopping_releases_an_idle_raw_reader() {
		use std::{io::Write, os::unix::net::UnixStream};
		let (reader, mut writer) = UnixStream::pair().unwrap();
		let (sender, mut receiver) = mpsc::channel(1);
		let stop = Arc::new(AtomicBool::new(false));
		let reader_stop = Arc::clone(&stop);
		let thread = std::thread::spawn(move || read_unix(reader.as_raw_fd(), &sender, &reader_stop));
		writer.write_all(b"\x03").unwrap();
		let received = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await;
		let started = Instant::now();
		stop.store(true, Ordering::Release);
		drop(receiver);
		thread.join().unwrap().unwrap();
		assert_eq!(received.unwrap(), Some(Input::Key(Key::Ctrl('c'))));
		assert!(
			started.elapsed() < Duration::from_secs(1),
			"idle terminal read did not honor shutdown"
		);
	}
}
