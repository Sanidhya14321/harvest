//! Async network I/O behind the synchronous model interface. Cancellation drops
//! the pending request/body future, without moving host callbacks off their
//! thread.
use std::{
	io::{self, Read},
	sync::{
		OnceLock,
		mpsc::{self, Receiver, SyncSender, TrySendError},
	},
	time::Duration,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{MAX_HTTP_BYTES, cancel::CancelToken};
use tokio::{
	runtime::{Builder, Runtime},
	task::JoinHandle,
};

const CHANNEL_DEPTH: usize = 4;
const CHUNK_BYTES: usize = 16 * 1024;
const CANCEL_POLL: Duration = Duration::from_millis(10);

enum Packet {
	Headers(bool),
	Data(Vec<u8>),
	Failure(anyhow::Error),
}

fn runtime() -> Result<&'static Runtime> {
	static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
	match RUNTIME.get_or_init(|| {
		Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()
			.map_err(|error| error.to_string())
	}) {
		Ok(runtime) => Ok(runtime),
		Err(error) => bail!("could not initialize provider transport: {error}"),
	}
}

async fn send(sender: &SyncSender<Packet>, cancellation: &CancelToken, mut packet: Packet) -> bool {
	loop {
		if cancellation.aborted() {
			return false;
		}
		match sender.try_send(packet) {
			Ok(()) => return true,
			Err(TrySendError::Disconnected(_)) => return false,
			Err(TrySendError::Full(pending)) => packet = pending,
		}
		tokio::select! {
			_ = cancellation.wait() => return false,
			() = tokio::time::sleep(CANCEL_POLL) => {},
		}
	}
}

async fn request(
	url: String,
	headers: Vec<(String, String)>,
	body: Vec<u8>,
	stream: bool,
	cancellation: &CancelToken,
	sender: &SyncSender<Packet>,
) -> Result<()> {
	let client = reqwest::Client::builder()
		.timeout(Duration::from_secs(60))
		.redirect(reqwest::redirect::Policy::custom(|attempt| {
			// Custom API-key headers are not automatically stripped by HTTP
			// clients on cross-origin redirects. Never forward credentials.
			let same_origin = attempt.previous().first().is_some_and(|original| {
				original.scheme() == attempt.url().scheme()
					&& original.host_str() == attempt.url().host_str()
					&& original.port_or_known_default() == attempt.url().port_or_known_default()
			});
			if same_origin && attempt.previous().len() < 5 {
				attempt.follow()
			} else {
				attempt.stop()
			}
		}))
		.build()
		.map_err(|_| crate::provider::ProviderRequestError::Transport(ureq::ErrorKind::Io))?;
	let mut request = client
		.post(&url)
		.header("Content-Type", "application/json")
		.body(body);
	if stream {
		request = request.header("Accept", "text/event-stream");
	}
	for (name, value) in headers {
		request = request.header(name, value);
	}
	let mut response = tokio::select! {
		_ = cancellation.wait() => return Ok(()),
		response = request.send() => response.map_err(|_| crate::provider::ProviderRequestError::Transport(ureq::ErrorKind::Io))?,
	};
	if !response.status().is_success() {
		return Err(crate::provider::ProviderRequestError::Status(response.status().as_u16()).into());
	}
	let sse = response
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| {
			value
				.split(';')
				.next()
				.is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
		});
	if !send(sender, cancellation, Packet::Headers(sse)).await {
		return Ok(());
	}
	let mut consumed = 0_usize;
	loop {
		let chunk = tokio::select! {
			_ = cancellation.wait() => return Ok(()),
			chunk = response.chunk() => chunk.map_err(|_| crate::provider::ProviderRequestError::Transport(ureq::ErrorKind::Io))?,
		};
		let Some(chunk) = chunk else {
			return Ok(());
		};
		consumed = consumed
			.checked_add(chunk.len())
			.context("provider response size overflow")?;
		if consumed > MAX_HTTP_BYTES {
			bail!("provider response exceeds response limit");
		}
		for bytes in chunk.chunks(CHUNK_BYTES) {
			if !send(sender, cancellation, Packet::Data(bytes.to_vec())).await {
				return Ok(());
			}
		}
	}
}

pub struct Response {
	pub sse:      bool,
	receiver:     Receiver<Packet>,
	cancellation: CancelToken,
	worker:       JoinHandle<()>,
	pending:      Vec<u8>,
	position:     usize,
}

impl Response {
	pub fn open(
		url: String,
		headers: Vec<(String, String)>,
		body: Vec<u8>,
		stream: bool,
		cancellation: &CancelToken,
	) -> Result<Self> {
		cancellation.heartbeat()?;
		if body.len() > MAX_HTTP_BYTES {
			bail!("provider request exceeds response limit");
		}
		let (sender, receiver) = mpsc::sync_channel(CHANNEL_DEPTH);
		let token = cancellation.clone();
		let worker = runtime()?.spawn(async move {
			if let Err(error) = request(url, headers, body, stream, &token, &sender).await {
				let _ = send(&sender, &token, Packet::Failure(error)).await;
			}
		});
		let mut response = Self {
			sse: false,
			receiver,
			cancellation: cancellation.clone(),
			worker,
			pending: Vec::new(),
			position: 0,
		};
		match response.receive()? {
			Some(Packet::Headers(sse)) => response.sse = sse,
			_ => bail!("provider response ended before headers"),
		}
		Ok(response)
	}

	fn receive(&self) -> Result<Option<Packet>> {
		loop {
			self.cancellation.heartbeat().map_err(io::Error::other)?;
			match self.receiver.recv_timeout(CANCEL_POLL) {
				Ok(Packet::Failure(message)) => return Err(message),
				Ok(packet) => return Ok(Some(packet)),
				Err(mpsc::RecvTimeoutError::Timeout) => {},
				Err(mpsc::RecvTimeoutError::Disconnected) => {
					self.cancellation.heartbeat().map_err(io::Error::other)?;
					return Ok(None);
				},
			}
		}
	}
}

impl Read for Response {
	fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
		if buffer.is_empty() {
			return Ok(0);
		}
		self.cancellation.heartbeat().map_err(io::Error::other)?;
		while self.position == self.pending.len() {
			match self.receive().map_err(io::Error::other)? {
				Some(Packet::Data(bytes)) => {
					self.pending = bytes;
					self.position = 0;
				},
				Some(Packet::Headers(_)) => return Err(io::Error::other("duplicate provider headers")),
				Some(Packet::Failure(message)) => return Err(io::Error::other(message)),
				None => return Ok(0),
			}
		}
		let count = buffer.len().min(self.pending.len() - self.position);
		buffer[..count].copy_from_slice(&self.pending[self.position..self.position + count]);
		self.position += count;
		Ok(count)
	}
}

impl Drop for Response {
	fn drop(&mut self) {
		self.worker.abort();
	}
}
