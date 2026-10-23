//! Shared bounded child ownership for JSONL and Content-Length protocols.
use std::{
	collections::BTreeMap,
	io::{BufRead, BufReader, Read, Write},
	path::Path,
	process::{Child, Command, Stdio},
	sync::{
		Arc, Mutex,
		mpsc::{self, Receiver, SyncSender, TrySendError},
	},
	thread,
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::cancel::CancelToken;
use serde::{Serialize, de::DeserializeOwned};

pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;
#[derive(Clone, Copy)]
pub(crate) enum Framing {
	JsonLines,
	ContentLength,
}

pub(crate) struct Process<T> {
	child:   Child,
	input:   Option<SyncSender<Vec<u8>>>,
	output:  Receiver<Result<T>>,
	stderr:  Arc<Mutex<Vec<u8>>>,
	closed:  bool,
	framing: Framing,
}
impl<T: DeserializeOwned + Send + 'static> Process<T> {
	pub fn launch(
		root: &Path,
		program: &Path,
		arguments: &[String],
		environment: &BTreeMap<String, String>,
		cancellation: &CancelToken,
	) -> Result<Self> {
		Self::launch_framed(root, program, arguments, environment, Framing::JsonLines, cancellation)
	}

	pub fn launch_framed(
		root: &Path,
		program: &Path,
		arguments: &[String],
		environment: &BTreeMap<String, String>,
		framing: Framing,
		cancellation: &CancelToken,
	) -> Result<Self> {
		cancellation.heartbeat()?;
		ensure!(
			arguments.len() <= 128 && environment.len() <= 128,
			"process configuration exceeds limit"
		);
		let mut child = Command::new(program)
			.args(arguments)
			.envs(environment)
			.current_dir(root)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.map_err(|_| anyhow::anyhow!("cannot start configured native process"))?;
		let mut stdin = child.stdin.take().context("process stdin unavailable")?;
		let stdout = child.stdout.take().context("process stdout unavailable")?;
		let mut error_stream = child.stderr.take().context("process stderr unavailable")?;
		let (input, incoming) = mpsc::sync_channel::<Vec<u8>>(8);
		let (outgoing, output) = mpsc::sync_channel(16);
		thread::spawn(move || {
			while let Ok(bytes) = incoming.recv() {
				if stdin
					.write_all(&bytes)
					.and_then(|()| stdin.flush())
					.is_err()
				{
					break;
				}
			}
		});
		thread::spawn(move || {
			let mut reader = BufReader::new(stdout);
			loop {
				let received = match framing {
					Framing::JsonLines => read_frame(&mut reader),
					Framing::ContentLength => read_content_length(&mut reader),
				};
				match received {
					Ok(Some(value)) => {
						if outgoing.send(Ok(value)).is_err() {
							break;
						}
					},
					Ok(None) => break,
					Err(error) => {
						let _ = outgoing.send(Err(error));
						break;
					},
				}
			}
		});
		let stderr = Arc::new(Mutex::new(Vec::new()));
		let tail = stderr.clone();
		thread::spawn(move || {
			let mut bytes = [0; 4096];
			while let Ok(count) = error_stream.read(&mut bytes) {
				if count == 0 {
					break;
				}
				let Ok(mut tail) = tail.lock() else {
					break;
				};
				tail.extend_from_slice(&bytes[..count]);
				if tail.len() > 8192 {
					let excess = tail.len() - 8192;
					tail.drain(..excess);
				}
			}
		});
		Ok(Self { child, input: Some(input), output, stderr, closed: false, framing })
	}

	pub fn send(&mut self, value: &impl Serialize, cancellation: &CancelToken) -> Result<()> {
		let mut bytes = encode_for(value, self.framing)?;
		let input = self.input.as_ref().context("native process is closed")?;
		loop {
			cancellation.heartbeat()?;
			match input.try_send(bytes) {
				Ok(()) => return Ok(()),
				Err(TrySendError::Disconnected(_)) => bail!("native process input closed"),
				Err(TrySendError::Full(pending)) => bytes = pending,
			}
			thread::sleep(Duration::from_millis(10));
		}
	}

	pub fn try_send(&mut self, value: &impl Serialize) {
		if let Some(input) = &self.input
			&& let Ok(bytes) = encode_for(value, self.framing)
		{
			let _ = input.try_send(bytes);
		}
	}

	pub fn receive(&mut self, cancellation: &CancelToken) -> Result<T> {
		loop {
			cancellation.heartbeat()?;
			match self.output.recv_timeout(Duration::from_millis(10)) {
				Ok(value) => return value,
				Err(mpsc::RecvTimeoutError::Disconnected) => {
					bail!("native process exited before its response")
				},
				Err(mpsc::RecvTimeoutError::Timeout) => {},
			}
		}
	}

	pub fn try_receive(&mut self) -> Result<Option<T>> {
		match self.output.try_recv() {
			Ok(value) => value.map(Some),
			Err(mpsc::TryRecvError::Empty) => Ok(None),
			Err(mpsc::TryRecvError::Disconnected) => Ok(None),
		}
	}

	pub fn stderr_tail(&self) -> Vec<u8> {
		self
			.stderr
			.lock()
			.map(|tail| tail.clone())
			.unwrap_or_default()
	}

	pub fn closed(&self) -> bool {
		self.closed
	}

	pub fn terminate(&mut self) {
		terminate(&mut self.child, &mut self.input, &mut self.closed);
	}
}
/// Shared framing also serves the Rust executable-plugin SDK's stdio reader.
pub(crate) fn read_frame<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
	let mut bytes = Vec::new();
	let count = reader
		.take(MAX_FRAME_BYTES as u64 + 1)
		.read_until(b'\n', &mut bytes)?;
	if count == 0 {
		return Ok(None);
	}
	ensure!(
		count <= MAX_FRAME_BYTES && bytes.last() == Some(&b'\n'),
		"invalid or oversized native JSONL frame"
	);
	let value =
		serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid native JSONL value"))?;
	Ok(Some(value))
}
pub(crate) fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
	struct Bounded(Vec<u8>);
	impl Write for Bounded {
		fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
			if bytes.len()
				> MAX_FRAME_BYTES
					.saturating_sub(1)
					.saturating_sub(self.0.len())
			{
				return Err(std::io::Error::other("native process frame exceeds limit"));
			}
			self.0.extend_from_slice(bytes);
			Ok(bytes.len())
		}

		fn flush(&mut self) -> std::io::Result<()> {
			Ok(())
		}
	}
	let mut output = Bounded(Vec::new());
	serde_json::to_writer(&mut output, value)
		.map_err(|_| anyhow::anyhow!("cannot encode native frame within byte limit"))?;
	output.0.push(b'\n');
	Ok(output.0)
}

fn encode_for(value: &impl Serialize, framing: Framing) -> Result<Vec<u8>> {
	let mut payload = encode(value)?;
	if matches!(framing, Framing::JsonLines) {
		return Ok(payload);
	}
	payload.pop();
	let header = format!("Content-Length: {}\r\n\r\n", payload.len());
	let mut frame = Vec::with_capacity(header.len() + payload.len());
	frame.extend_from_slice(header.as_bytes());
	frame.extend_from_slice(&payload);
	Ok(frame)
}
fn read_content_length<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
	let mut length = None;
	let mut header_bytes = 0;
	loop {
		let mut line = Vec::new();
		let count = (&mut *reader).take(8193).read_until(b'\n', &mut line)?;
		if count == 0 {
			ensure!(header_bytes == 0, "process exited inside a protocol header");
			return Ok(None);
		}
		header_bytes += count;
		ensure!(
			header_bytes <= 8192 && line.ends_with(b"\r\n"),
			"invalid or oversized Content-Length header"
		);
		if line == b"\r\n" {
			break;
		}
		let text = std::str::from_utf8(&line)
			.map_err(|_| anyhow::anyhow!("invalid protocol header encoding"))?;
		let (name, value) = text
			.trim_end_matches(['\r', '\n'])
			.split_once(':')
			.context("invalid protocol header field")?;
		if name.eq_ignore_ascii_case("Content-Length") {
			ensure!(length.is_none(), "duplicate Content-Length header");
			let value = value.trim();
			ensure!(
				!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
				"invalid protocol content length"
			);
			length = Some(
				value
					.parse::<usize>()
					.map_err(|_| anyhow::anyhow!("invalid protocol content length"))?,
			);
		}
	}
	let length = length.context("protocol frame has no Content-Length")?;
	ensure!(length > 0 && length < MAX_FRAME_BYTES, "protocol body exceeds frame limit");
	let mut body = vec![0; length];
	reader.read_exact(&mut body)?;
	Ok(Some(
		serde_json::from_slice(&body)
			.map_err(|_| anyhow::anyhow!("invalid Content-Length JSON body"))?,
	))
}

fn terminate(child: &mut Child, input: &mut Option<SyncSender<Vec<u8>>>, closed: &mut bool) {
	if *closed {
		return;
	}
	*closed = true;
	input.take();
	// Closing stdin gives cooperating native servers a brief bounded drain.
	let deadline = Instant::now() + Duration::from_millis(100);
	loop {
		match child.try_wait() {
			Ok(Some(_)) => return,
			Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
			_ => break,
		}
	}
	let _ = child.kill();
	let _ = child.wait();
}
impl<T> Drop for Process<T> {
	fn drop(&mut self) {
		terminate(&mut self.child, &mut self.input, &mut self.closed);
	}
}
