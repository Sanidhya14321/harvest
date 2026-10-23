//! Experimental, bounded JSONL v1/v2 adapter for the asynchronous Rust owner.
//!
//! This is an opt-in subset, not a replacement for the production RPC mode.
//! Unsupported commands fail explicitly. The caller owns approval policy;
//! the CLI denies mutations because stdin belongs exclusively to this protocol.
pub mod reader;
use std::{
	collections::BTreeMap,
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::Duration,
};

use anyhow::{Context, Result, bail};
use harvest_protocol::{
	rpc_frame::{self, Frames, MAX_LOGICAL_BYTES, Version},
	transcript::{self, Snapshot, StaleCursor},
};
use harvest_runtime::MAX_EVENT_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use tokio::{
	io::{AsyncRead, AsyncWrite, AsyncWriteExt},
	sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
};

use crate::{
	chat::ChatMessage,
	durable_session::stored_message,
	session_engine::{
		RunHandle, SessionEngineEventKind, SessionHandle, SessionSnapshot, SessionState,
	},
};

const IO_CAPACITY: usize = 32;
const _: () = assert!(MAX_EVENT_BYTES == rpc_frame::MAX_FRAME_BYTES);
const MAX_PENDING_ABORTS: usize = 64;
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// The model identity is used only to label provisional messages. Durable
/// messages retain the exact identity and timestamp written by the driver.
pub struct RpcOptions {
	pub session_file: PathBuf,
	pub model:        String,
}

/// Serve one client, then cancel work and release the session's writer lease.
///
/// Input/output queues and physical JSONL frames are bounded. A failed or slow
/// output sink closes the connection; it never silently drops transcript
/// events.
pub async fn serve<R, W>(
	session: SessionHandle,
	input: R,
	output: W,
	options: RpcOptions,
) -> Result<()>
where
	R: AsyncRead + Unpin + Send + 'static,
	W: AsyncWrite + Unpin + Send + 'static,
{
	let (input_tx, mut incoming) = mpsc::channel(IO_CAPACITY);
	let reader = tokio::spawn(read_frames(input, input_tx));
	let (output_tx, output_rx) = mpsc::channel(IO_CAPACITY);
	let (failed_tx, mut failed) = watch::channel(None::<String>);
	let writer = tokio::spawn(async move {
		let result = write_frames(output, output_rx).await;
		if let Err(error) = &result {
			failed_tx.send_replace(Some(error.to_string()));
		}
		result
	});
	let sink = Sink {
		output:  output_tx,
		v2:      AtomicBool::new(false),
		counter: AtomicU64::new(0),
		budget:  Arc::new(Semaphore::new(MAX_LOGICAL_BYTES)),
	};
	let result = async {
		let mut subscription = session.subscribe().await?;
		sink.send(&json!({"type":"ready", "protocolVersion":1,
			"supportedProtocolVersions":[1,2], "maxFrameBytes":MAX_EVENT_BYTES,
			"maxReassembledFrameBytes":MAX_LOGICAL_BYTES})).await?;
		let mut adapter = Adapter { session: &session, sink: &sink, options, runs: BTreeMap::new(), generation: 0, pending_aborts: Vec::new() };
		loop {
			tokio::select! {
				input = incoming.recv() => match input {
					Some(Input::Frame(bytes)) => match serde_json::from_slice::<Request>(&bytes) {
						Ok(request) => adapter.command(request).await?,
						Err(error) => sink.error(None, "parse", &format!("invalid RPC command: {error}")).await?,
					},
					Some(Input::Invalid(message)) => sink.error(None, "parse", &message).await?,
					Some(Input::Failed(message)) => bail!("RPC input failed: {message}"),
					None => break,
				},
				event = subscription.events.recv() => {
					let event = event.context("RPC session event stream interrupted; reconnect for a committed snapshot")?;
					if matches!(event.event, SessionEngineEventKind::Closed) { break; }
					if let Some(run) = event.run { adapter.event(run.generation, &event.event).await?; }
				},
				changed = failed.changed() => {
					changed.context("RPC output task stopped")?;
					bail!("RPC output failed: {}", failed.borrow().as_deref().unwrap_or("output closed"));
				},
			}
		}
		Ok(())
	}.await;
	reader.abort();
	let shutdown = session.shutdown().await;
	drop(sink);
	let output_result = writer.await.context("RPC output task failed")?;
	result?;
	shutdown?;
	output_result
}

#[derive(Deserialize)]
struct Request {
	#[serde(default)]
	id:      Option<String>,
	#[serde(rename = "type")]
	command: String,
	#[serde(flatten)]
	fields:  serde_json::Map<String, Value>,
}

#[derive(Serialize)]
struct Response<'a, T> {
	#[serde(skip_serializing_if = "Option::is_none")]
	id:      Option<&'a str>,
	#[serde(rename = "type")]
	kind:    &'static str,
	command: &'a str,
	success: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	data:    Option<T>,
	#[serde(skip_serializing_if = "Option::is_none")]
	error:   Option<&'a str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	code:    Option<&'a str>,
}

struct Output {
	frames:  Frames,
	_permit: OwnedSemaphorePermit,
}
struct Sink {
	output:  mpsc::Sender<Output>,
	v2:      AtomicBool,
	counter: AtomicU64,
	budget:  Arc<Semaphore>,
}
impl Sink {
	fn encode(&self, value: &impl Serialize) -> Result<Frames> {
		let version = if self.v2.load(Ordering::Relaxed) {
			Version::V2
		} else {
			Version::V1
		};
		let id = self.counter.fetch_add(1, Ordering::Relaxed);
		rpc_frame::encode(value, version, format!("rpc-{id}"))
	}

	async fn enqueue(&self, frames: Frames) -> Result<()> {
		let bytes = u32::try_from(frames.retained_bytes())?;
		let permit = self.budget.clone().acquire_many_owned(bytes).await?;
		self
			.output
			.send(Output { frames, _permit: permit })
			.await
			.context("RPC output closed")
	}

	async fn send<T: Serialize + Sync>(&self, value: &T) -> Result<()> {
		self.enqueue(self.encode(value)?).await
	}

	async fn success<T: Serialize + Send + Sync>(
		&self,
		id: Option<&str>,
		command: &str,
		data: Option<T>,
	) -> Result<()> {
		let response =
			Response { id, kind: "response", command, success: true, data, error: None, code: None };
		match self.encode(&response) {
			Ok(frames) => self.enqueue(frames).await,
			Err(_) => {
				self
					.error(
						id,
						command,
						"response exceeds the negotiated frame limit; no history was truncated",
					)
					.await
			},
		}
	}

	async fn error(&self, id: Option<&str>, command: &str, error: &str) -> Result<()> {
		self.coded_error(id, command, error, None).await
	}

	async fn coded_error(
		&self,
		id: Option<&str>,
		command: &str,
		error: &str,
		code: Option<&str>,
	) -> Result<()> {
		self
			.send(&Response::<()> {
				id,
				kind: "response",
				command,
				success: false,
				data: None,
				error: Some(error),
				code,
			})
			.await
	}
}

async fn write_frames<W: AsyncWrite + Unpin>(
	mut output: W,
	mut frames: mpsc::Receiver<Output>,
) -> Result<()> {
	while let Some(message) = frames.recv().await {
		tokio::time::timeout(WRITE_TIMEOUT, async {
			for frame in message.frames {
				output.write_all(&frame?).await?;
			}
			output.flush().await?;
			Ok::<(), anyhow::Error>(())
		})
		.await
		.context("RPC output backpressure deadline exceeded")??;
	}
	Ok(())
}

enum Input {
	Frame(Vec<u8>),
	Invalid(String),
	Failed(String),
}

async fn read_frames<R: AsyncRead + Unpin>(input: R, frames: mpsc::Sender<Input>) {
	let result = read_frames_inner(input, &frames).await;
	if let Err(error) = result {
		let _ = frames.send(Input::Failed(error.to_string())).await;
	}
}

async fn read_frames_inner<R: AsyncRead + Unpin>(
	input: R,
	frames: &mpsc::Sender<Input>,
) -> Result<()> {
	let mut input = reader::PhysicalReader::new(input);
	while let Some(frame) = input.next().await? {
		let frame = match frame {
			reader::Physical::Frame(bytes) => Input::Frame(bytes),
			reader::Physical::Invalid => Input::Invalid("RPC input exceeds frame limit".into()),
		};
		frames.send(frame).await?;
	}
	Ok(())
}

struct Accepted {
	id:       Option<String>,
	command:  String,
	handle:   RunHandle,
	messages: Vec<Box<RawValue>>,
	progress: Progress,
}
#[derive(Default)]
struct Progress {
	text:         String,
	timestamp:    i64,
	streaming:    bool,
	turn_started: bool,
	assistant:    Option<Box<RawValue>>,
	tools:        Vec<Box<RawValue>>,
}
struct Adapter<'a> {
	session:        &'a SessionHandle,
	sink:           &'a Sink,
	options:        RpcOptions,
	runs:           BTreeMap<u64, Accepted>,
	generation:     u64,
	pending_aborts: Vec<(u64, Option<String>)>,
}
impl Adapter<'_> {
	async fn command(&mut self, request: Request) -> Result<()> {
		let id = request.id.as_deref();
		let command = request.command.as_str();
		match command {
			"prompt" | "follow_up" => {
				if self.runs.len() >= IO_CAPACITY {
					return self
						.sink
						.error(id, command, "RPC pending run limit reached; wait for terminal events")
						.await;
				}
				let message = match request.fields.get("message").and_then(Value::as_str) {
					Some(message) if !message.trim().is_empty() => message,
					_ => {
						return self
							.sink
							.error(id, command, "message must be a nonempty string")
							.await;
					},
				};
				if message.trim_start().starts_with('/') {
					return self
						.sink
						.error(
							id,
							command,
							"slash commands are not implemented in the experimental Rust RPC adapter",
						)
						.await;
				}
				if request
					.fields
					.get("images")
					.is_some_and(|images| !images.as_array().is_some_and(Vec::is_empty))
				{
					return self
						.sink
						.error(
							id,
							command,
							"image prompts require a provider replay adapter; Rust RPC accepts text only",
						)
						.await;
				}
				let follow_up = command == "follow_up"
					|| match request.fields.get("streamingBehavior") {
						None => false,
						Some(Value::String(mode)) if mode == "followUp" => true,
						_ => {
							return self
								.sink
								.error(
									id,
									command,
									"steering is not implemented in the experimental Rust RPC adapter",
								)
								.await;
						},
					};
				let ticket = if follow_up {
					self.session.follow_up(message).await
				} else {
					self.session.submit(message).await
				};
				match ticket {
					Ok(ticket) => {
						let generation = ticket.handle.id().generation;
						self.runs.insert(generation, Accepted {
							id:       request.id.clone(),
							command:  request.command.clone(),
							handle:   ticket.handle.clone(),
							messages: Vec::new(),
							progress: Progress::default(),
						});
						// The engine event stream owns lifecycle completion; its terminal
						// events and durable receipts are ordered before ticket completion.
						drop(ticket);
						self
							.sink
							.success(
								id,
								command,
								(command == "prompt").then(|| json!({"agentInvoked":true})),
							)
							.await?;
					},
					Err(error) => self.sink.error(id, command, &error.to_string()).await?,
				}
			},
			"abort" => {
				let requested_generation = match request.fields.get("runGeneration").map(Value::as_u64)
				{
					None => None,
					Some(Some(generation)) => Some(generation),
					Some(None) => {
						return self
							.sink
							.error(id, command, "runGeneration must be an unsigned integer")
							.await;
					},
				};
				let snapshot = self.session.snapshot().await?;
				let target = snapshot.active.filter(|run| {
					requested_generation.is_none_or(|generation| run.generation == generation)
				});
				if let Some(run) = target {
					if self.pending_aborts.len() >= MAX_PENDING_ABORTS {
						return self
							.sink
							.error(id, command, "too many pending abort acknowledgements")
							.await;
					}
					if let Some(accepted) = self.runs.get(&run.generation)
						&& self.session.abort(&accepted.handle).await?
					{
						self.pending_aborts.push((run.generation, request.id));
						return Ok(());
					}
				}
				self.sink.success::<()>(id, command, None).await?;
			},
			// Native opt-in extension: explicitly resume work parked by abort.
			"resume_follow_ups" => match self.session.resume_follow_ups().await {
				Ok(resumed) => {
					self
						.sink
						.success(id, command, Some(json!({"resumed":resumed})))
						.await?;
				},
				Err(error) => self.sink.error(id, command, &error.to_string()).await?,
			},
			"get_state" => {
				let snapshot = self.session.snapshot().await?;
				self
					.sink
					.success(id, command, Some(self.state(&snapshot)))
					.await?;
			},
			"get_last_assistant_text" => {
				let snapshot = self.session.snapshot().await?;
				let text = transcript::last_assistant_text(&snapshot.stored_messages)?;
				let data = text.map_or_else(|| json!({}), |text| json!({"text":text}));
				self.sink.success(id, command, Some(data)).await?;
			},
			"get_messages_page" => {
				let snapshot = self.session.snapshot().await?;
				if snapshot.state != SessionState::Idle || !snapshot.queued.is_empty() {
					return self
						.sink
						.coded_error(
							id,
							command,
							"Cannot page messages while the session is changing",
							Some("session_busy"),
						)
						.await;
				}
				let cursor = match request.fields.get("cursor") {
					None => None,
					Some(Value::String(cursor)) => Some(cursor.as_str()),
					_ => {
						return self
							.sink
							.error(id, command, "Invalid RPC message cursor")
							.await;
					},
				};
				let limit = match request.fields.get("limit") {
					None | Some(Value::Null) => None,
					Some(value) => {
						match transcript::safe_integer(value).and_then(|n| usize::try_from(n).ok()) {
							Some(limit) => Some(limit),
							None => {
								return self
									.sink
									.error(id, command, "RPC message page limit must be between 1 and 256")
									.await;
							},
						}
					},
				};
				let identity = Snapshot {
					session_id:    snapshot.session_id,
					leaf_id:       snapshot.leaf,
					message_count: snapshot.stored_messages.len(),
				};
				match transcript::page(&snapshot.stored_messages, &identity, cursor, limit) {
					Ok(page) => self.sink.success(id, command, Some(page)).await?,
					Err(error) => {
						self
							.sink
							.coded_error(
								id,
								command,
								&error.to_string(),
								error.is::<StaleCursor>().then_some("stale_cursor"),
							)
							.await?;
					},
				}
			},
			"get_messages" => {
				#[derive(Serialize)]
				struct Messages {
					messages: Vec<Box<RawValue>>,
				}
				let snapshot = self.session.snapshot().await?;
				self
					.sink
					.success(id, command, Some(Messages { messages: snapshot.stored_messages }))
					.await?;
			},
			"negotiate_protocol" => {
				if request
					.fields
					.get("protocolVersion")
					.and_then(transcript::safe_integer)
					== Some(2)
				{
					// Queue acknowledgement before switching the ordered writer.
					self
						.sink
						.success(id, command, Some(json!({"protocolVersion":2})))
						.await?;
					self.sink.v2.store(true, Ordering::Relaxed);
				} else {
					self
						.sink
						.error(id, command, "Unsupported RPC protocol version; expected 2")
						.await?;
				}
			},
			_ if known_command(command) => {
				self
					.sink
					.error(
						id,
						command,
						"command is not implemented in the experimental Rust RPC adapter",
					)
					.await?;
			},
			_ => {
				self
					.sink
					.error(None, command, "unknown RPC command")
					.await?;
			},
		}
		Ok(())
	}

	fn state(&self, snapshot: &SessionSnapshot) -> Value {
		json!({"isStreaming":snapshot.state == SessionState::Running, "isCompacting":false,
			"steeringMode":"one-at-a-time", "followUpMode":"one-at-a-time", "interruptMode":"immediate",
			"sessionFile":self.options.session_file, "sessionId":snapshot.session_id,
			"autoCompactionEnabled":false, "fastModeEnabled":false, "fastModeActive":false,
			"tokensPerSecond":null, "messageCount":snapshot.messages.len(), "queuedMessageCount":snapshot.queued.len(),
			"todoPhases":[], "runGeneration":snapshot.active.map_or(self.generation, |run| run.generation)})
	}

	async fn event(&mut self, generation: u64, event: &SessionEngineEventKind) -> Result<()> {
		let has_other_runs = self.runs.keys().any(|other| *other != generation);
		let Some(run) = self.runs.get_mut(&generation) else {
			return Ok(());
		};
		match event {
			SessionEngineEventKind::Started => {
				self.generation = generation;
				self.sink.send(&json!({"type":"agent_start"})).await?;
				run.progress.start_turn(self.sink).await?;
			},
			SessionEngineEventKind::TextDelta { text } => {
				run.progress
					.delta(self.sink, &self.options.model, text)
					.await?;
			},
			SessionEngineEventKind::Committed { role, message, .. } => {
				run.messages.push(message.clone());
				if role == "assistant" {
					run.progress.commit(self.sink, message).await?;
				} else {
					self.sink.send(&Event::MessageStart { message }).await?;
					self.sink.send(&Event::MessageEnd { message }).await?;
					if role == "tool" {
						run.progress.tools.push(message.clone());
					}
				}
			},
			SessionEngineEventKind::Completed
			| SessionEngineEventKind::Cancelled
			| SessionEngineEventKind::Failed { .. } => {
				let failed = !matches!(event, SessionEngineEventKind::Completed);
				if failed {
					run.progress
						.interrupted(
							self.sink,
							&self.options.model,
							matches!(event, SessionEngineEventKind::Cancelled),
						)
						.await?;
				}
				run.progress.finish_turn(self.sink).await?;
				if let SessionEngineEventKind::Failed { message } = event {
					self
						.sink
						.error(run.id.as_deref(), &run.command, message)
						.await?;
				}
				let snapshot = self.session.snapshot().await?;
				let terminal =
					snapshot.follow_ups_paused || (!has_other_runs && snapshot.active.is_none());
				self
					.sink
					.send(&Event::AgentEnd { messages: &run.messages, is_terminal: terminal })
					.await?;
				self.runs.remove(&generation);
				let mut retained = Vec::new();
				for (target, id) in self.pending_aborts.drain(..) {
					if target == generation {
						self
							.sink
							.success::<()>(id.as_deref(), "abort", None)
							.await?;
					} else {
						retained.push((target, id));
					}
				}
				self.pending_aborts = retained;
			},
			_ => {},
		}
		Ok(())
	}
}

fn known_command(command: &str) -> bool {
	matches!(
		command,
		"steer"
			| "abort_and_prompt"
			| "new_session"
			| "set_fast_mode"
			| "get_available_commands"
			| "set_todos"
			| "set_host_tools"
			| "set_host_uri_schemes"
			| "set_subagent_subscription"
			| "get_subagents"
			| "get_subagent_messages"
			| "set_model"
			| "cycle_model"
			| "get_available_models"
			| "set_thinking_level"
			| "cycle_thinking_level"
			| "set_steering_mode"
			| "set_follow_up_mode"
			| "set_interrupt_mode"
			| "compact"
			| "set_auto_compaction"
			| "set_auto_retry"
			| "abort_retry"
			| "bash"
			| "abort_bash"
			| "get_session_stats"
			| "export_html"
			| "switch_session"
			| "branch"
			| "get_branch_messages"
			| "set_session_name"
			| "handoff"
			| "get_login_providers"
			| "login"
	)
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event<'a> {
	MessageStart {
		message: &'a RawValue,
	},
	MessageEnd {
		message: &'a RawValue,
	},
	MessageUpdate {
		message:                 &'a RawValue,
		#[serde(rename = "assistantMessageEvent")]
		assistant_message_event: AssistantEvent<'a>,
	},
	TurnEnd {
		message:      &'a RawValue,
		#[serde(rename = "toolResults")]
		tool_results: &'a [Box<RawValue>],
	},
	AgentEnd {
		messages:    &'a [Box<RawValue>],
		#[serde(rename = "isTerminal")]
		is_terminal: bool,
	},
}
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AssistantEvent<'a> {
	Start {
		partial: &'a RawValue,
	},
	TextStart {
		#[serde(rename = "contentIndex")]
		content_index: u8,
		partial:       &'a RawValue,
	},
	TextDelta {
		#[serde(rename = "contentIndex")]
		content_index: u8,
		delta:         &'a str,
		partial:       &'a RawValue,
	},
	TextEnd {
		#[serde(rename = "contentIndex")]
		content_index: u8,
		content:       &'a str,
		partial:       &'a RawValue,
	},
	Done {
		reason:  &'a str,
		message: &'a RawValue,
	},
	Error {
		reason: &'a str,
		error:  &'a RawValue,
	},
}
impl Progress {
	async fn start_turn(&mut self, sink: &Sink) -> Result<()> {
		if self.turn_started {
			self.finish_turn(sink).await?;
		}
		sink.send(&json!({"type":"turn_start"})).await?;
		self.turn_started = true;
		Ok(())
	}

	async fn finish_turn(&mut self, sink: &Sink) -> Result<()> {
		if let Some(message) = &self.assistant {
			sink
				.send(&Event::TurnEnd { message, tool_results: &self.tools })
				.await?;
		}
		self.assistant = None;
		self.tools.clear();
		self.text.clear();
		self.streaming = false;
		self.turn_started = false;
		Ok(())
	}

	fn partial(&self, model: &str) -> Result<Box<RawValue>> {
		Ok(serde_json::value::to_raw_value(&stored_message(
			&ChatMessage {
				role:         "assistant".into(),
				content:      Some(self.text.clone()),
				tool_calls:   None,
				tool_call_id: None,
			},
			model,
			self.timestamp,
		)?)?)
	}

	async fn delta(&mut self, sink: &Sink, model: &str, text: &str) -> Result<()> {
		if self.assistant.is_some() || !self.turn_started {
			self.start_turn(sink).await?;
		}
		if !self.streaming {
			self.timestamp = chrono::Utc::now().timestamp_millis();
			let partial = self.partial(model)?;
			sink
				.send(&Event::MessageStart { message: &partial })
				.await?;
			sink
				.send(&Event::MessageUpdate {
					message:                 &partial,
					assistant_message_event: AssistantEvent::Start { partial: &partial },
				})
				.await?;
			sink
				.send(&Event::MessageUpdate {
					message:                 &partial,
					assistant_message_event: AssistantEvent::TextStart {
						content_index: 0,
						partial:       &partial,
					},
				})
				.await?;
			self.streaming = true;
		}
		if text.len() > harvest_runtime::MAX_HTTP_BYTES.saturating_sub(self.text.len()) {
			bail!("RPC provisional message exceeds response limit");
		}
		self.text.push_str(text);
		let partial = self.partial(model)?;
		sink
			.send(&Event::MessageUpdate {
				message:                 &partial,
				assistant_message_event: AssistantEvent::TextDelta {
					content_index: 0,
					delta:         text,
					partial:       &partial,
				},
			})
			.await
	}

	async fn commit(&mut self, sink: &Sink, message: &RawValue) -> Result<()> {
		if self.assistant.is_some() || !self.turn_started {
			self.start_turn(sink).await?;
		}
		if self.streaming {
			sink
				.send(&Event::MessageUpdate {
					message,
					assistant_message_event: AssistantEvent::TextEnd {
						content_index: 0,
						content:       &self.text,
						partial:       message,
					},
				})
				.await?;
		} else {
			sink.send(&Event::MessageStart { message }).await?;
		}
		let parsed: Value = serde_json::from_str(message.get())?;
		let reason = parsed["stopReason"]
			.as_str()
			.context("committed assistant has no stop reason")?;
		sink
			.send(&Event::MessageUpdate {
				message,
				assistant_message_event: AssistantEvent::Done { reason, message },
			})
			.await?;
		sink.send(&Event::MessageEnd { message }).await?;
		self.assistant = Some(message.to_owned());
		self.streaming = false;
		Ok(())
	}

	async fn interrupted(&mut self, sink: &Sink, model: &str, cancelled: bool) -> Result<()> {
		if self.streaming {
			let reason = if cancelled { "aborted" } else { "error" };
			let mut partial: Value = serde_json::from_str(self.partial(model)?.get())?;
			partial["stopReason"] = json!(reason);
			let partial = serde_json::value::to_raw_value(&partial)?;
			sink
				.send(&Event::MessageUpdate {
					message:                 &partial,
					assistant_message_event: AssistantEvent::Error { reason, error: &partial },
				})
				.await?;
			sink.send(&Event::MessageEnd { message: &partial }).await?;
			self.assistant = Some(partial);
			self.streaming = false;
		}
		Ok(())
	}
}

#[cfg(test)]
mod transport_tests {
	use std::{
		io,
		pin::Pin,
		task::{Context, Poll},
	};

	use anyhow::Context as _;

	use super::*;

	fn bounded_sink(bytes: usize) -> (Sink, mpsc::Receiver<Output>) {
		let (output, receiver) = mpsc::channel(IO_CAPACITY);
		(
			Sink {
				output,
				v2: AtomicBool::new(true),
				counter: AtomicU64::new(0),
				budget: Arc::new(Semaphore::new(bytes)),
			},
			receiver,
		)
	}

	#[tokio::test]
	async fn output_queue_waits_for_bytes_even_when_message_slots_are_available() -> Result<()> {
		let (sink, mut receiver) = bounded_sink(2);
		let value = json!({});
		sink.send(&value).await?;
		let send = sink.send(&value);
		tokio::pin!(send);
		assert!(
			tokio::time::timeout(Duration::from_millis(20), send.as_mut())
				.await
				.is_err()
		);
		assert_eq!(sink.budget.available_permits(), 0);
		drop(receiver.recv().await.context("first queued message")?);
		tokio::time::timeout(Duration::from_secs(1), send).await??;
		drop(receiver.recv().await.context("second queued message")?);
		assert_eq!(sink.budget.available_permits(), 2);
		Ok(())
	}

	struct BlockedOutput;
	impl AsyncWrite for BlockedOutput {
		fn poll_write(
			self: Pin<&mut Self>,
			_: &mut Context<'_>,
			_: &[u8],
		) -> Poll<io::Result<usize>> {
			Poll::Pending
		}

		fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
			Poll::Ready(Ok(()))
		}

		fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
			Poll::Ready(Ok(()))
		}
	}

	#[tokio::test]
	async fn stalled_writer_times_out_and_releases_all_queued_byte_permits() -> Result<()> {
		let (sink, receiver) = bounded_sink(4);
		sink.send(&json!({})).await?;
		sink.send(&json!({})).await?;
		let failure =
			tokio::time::timeout(Duration::from_secs(12), write_frames(BlockedOutput, receiver))
				.await?
				.unwrap_err();
		assert!(
			failure
				.to_string()
				.contains("backpressure deadline exceeded")
		);
		assert_eq!(sink.budget.available_permits(), 4);
		assert!(sink.send(&json!({})).await.is_err());
		Ok(())
	}
}
