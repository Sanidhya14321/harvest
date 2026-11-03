//! Rust executable-plugin authoring API. Diagnostic output belongs on stderr.
use std::{
	io::{BufReader, Read, Write},
	sync::{
		Arc, Mutex,
		mpsc::{self, Receiver},
	},
	time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};
use harvest_runtime::cancel::{AbortReason, AbortToken, CancelToken};
use serde_json::Value;

use super::protocol::{self, Event, Frame, ProtocolError};
use crate::chat::{ToolCall, ToolSpec};

type Incoming = (Frame, Option<CancelToken>);
type Active = Arc<Mutex<Option<(u64, AbortToken)>>>;

pub trait Handler {
	fn name(&self) -> &str;
	fn tools(&self) -> Vec<ToolSpec>;
	fn call(&mut self, name: &str, arguments: Value, context: &mut Context<'_>) -> Result<Value>;
}

pub struct Context<'a> {
	id:               u64,
	pub cancellation: CancelToken,
	input:            &'a Receiver<Result<Incoming>>,
	output:           &'a mut dyn Write,
	next_host_id:     u64,
	events:           usize,
	event_bytes:      usize,
}
impl Context<'_> {
	pub fn event(&mut self, event: Event) -> Result<()> {
		self.cancellation.heartbeat()?;
		self.events += 1;
		ensure!(self.events <= protocol::MAX_EVENTS, "too many plugin events");
		self.event_bytes += protocol::event_bytes(&event)?;
		ensure!(
			self.event_bytes <= protocol::MAX_EVENT_TOTAL_BYTES,
			"plugin event stream exceeds byte limit"
		);
		write_frame(self.output, &Frame::Event { id: self.id, event })
	}

	pub fn host_call(&mut self, call: ToolCall) -> Result<String> {
		self.cancellation.heartbeat()?;
		let id = self.next_host_id;
		ensure!(id < protocol::MAX_HOST_CALLS as u64, "too many plugin host calls");
		self.next_host_id += 1;
		write_frame(self.output, &Frame::HostCall { id, parent_id: self.id, call })?;
		loop {
			self.cancellation.heartbeat()?;
			match self.input.recv_timeout(Duration::from_millis(10)) {
				Ok(Ok((Frame::HostResult { id: response_id, value: Some(value), error: None }, _)))
					if response_id == id =>
				{
					return Ok(value);
				},
				Ok(Ok((Frame::HostResult { id: response_id, error: Some(_), .. }, _)))
					if response_id == id =>
				{
					bail!("host tool denied or failed")
				},
				Ok(Ok(_)) => bail!("unexpected plugin host response"),
				Ok(Err(error)) => return Err(error),
				Err(mpsc::RecvTimeoutError::Disconnected) => bail!("plugin host disconnected"),
				Err(mpsc::RecvTimeoutError::Timeout) => {},
			}
		}
	}
}
fn write_frame(output: &mut dyn Write, frame: &Frame) -> Result<()> {
	output.write_all(&protocol::encode(frame)?)?;
	output.flush()?;
	Ok(())
}
fn abort(active: &Active, id: Option<u64>) {
	if let Ok(active) = active.lock()
		&& let Some((current, abort)) = active.as_ref()
		&& id.is_none_or(|id| id == *current)
	{
		abort.abort(AbortReason::User);
	}
}

/// Use native process stdio; no separate worker script or language runtime.
pub fn serve_stdio(handler: &mut impl Handler) -> Result<()> {
	serve(std::io::stdin(), std::io::stdout(), handler)
}
pub fn serve(
	input: impl Read + Send + 'static,
	mut output: impl Write,
	handler: &mut impl Handler,
) -> Result<()> {
	let (sender, incoming) = mpsc::sync_channel(8);
	let active: Active = Arc::new(Mutex::new(None));
	let reader_active = active.clone();
	std::thread::spawn(move || {
		let mut reader = BufReader::new(input);
		loop {
			let received = protocol::read(&mut reader);
			let frame = match received {
				Ok(Some(Frame::Cancel { id })) => {
					abort(&reader_active, Some(id));
					continue;
				},
				Ok(Some(frame)) => frame,
				Ok(None) => {
					abort(&reader_active, None);
					break;
				},
				Err(error) => {
					abort(&reader_active, None);
					let _ = sender.send(Err(error));
					break;
				},
			};
			let token = if let Frame::Call { id, .. } = &frame {
				let mut token = CancelToken::default();
				let abort_token = token.emplace_abort_token();
				let Ok(mut active) = reader_active.lock() else {
					break;
				};
				if active.is_some() {
					drop(active);
					abort(&reader_active, None);
					let _ = sender.send(Err(anyhow::anyhow!("concurrent plugin calls are unsupported")));
					break;
				}
				*active = Some((*id, abort_token));
				Some(token)
			} else {
				None
			};
			let shutdown = matches!(&frame, Frame::Shutdown);
			if shutdown {
				abort(&reader_active, None);
			}
			if sender.send(Ok((frame, token))).is_err() || shutdown {
				break;
			}
		}
	});
	let (initialize, _) = incoming
		.recv()
		.context("plugin host exited before initialization")??;
	let Frame::Initialize { id, versions } = initialize else {
		bail!("plugin requires initialization first");
	};
	if !versions.contains(&protocol::VERSION) {
		write_frame(&mut output, &Frame::Error {
			id,
			error: ProtocolError {
				code:    "version_mismatch".into(),
				message: "native plugin protocol version 1 required".into(),
			},
		})?;
		return Ok(());
	}
	let tools = handler.tools();
	ensure!(tools.len() <= protocol::MAX_TOOLS, "too many plugin tools");
	for tool in &tools {
		tool.validate()?;
	}
	write_frame(&mut output, &Frame::Initialized {
		id,
		version: protocol::VERSION,
		name: handler.name().into(),
		tools: tools.clone(),
	})?;
	while let Ok(received) = incoming.recv() {
		let (frame, token) = received?;
		match frame {
			Frame::Call { id, name, arguments } => {
				let token = token.context("plugin cancellation owner missing")?;
				let mut context = Context {
					id,
					cancellation: token,
					input: &incoming,
					output: &mut output,
					next_host_id: 0,
					events: 0,
					event_bytes: 0,
				};
				let result = if tools.iter().any(|tool| tool.name() == name) && arguments.is_object() {
					handler.call(&name, arguments, &mut context)
				} else {
					Err(anyhow::anyhow!("unknown plugin tool or invalid arguments"))
				};
				let response = match result.and_then(|value| {
					context.cancellation.heartbeat()?;
					Ok(value)
				}) {
					Ok(value) => Frame::Result { id, value },
					Err(_) => Frame::Error {
						id,
						error: ProtocolError {
							code:    "plugin_call_failed".into(),
							message: "Plugin request failed or was cancelled; diagnostics belong on \
							          stderr"
								.into(),
						},
					},
				};
				// Clear ownership before publishing completion so a fast next call
				// cannot collide with the previous generation.
				*active
					.lock()
					.map_err(|_| anyhow::anyhow!("plugin owner poisoned"))? = None;
				write_frame(&mut output, &response)?;
			},
			Frame::Shutdown => break,
			_ => bail!("unexpected plugin request"),
		}
	}
	Ok(())
}
