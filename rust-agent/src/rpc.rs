//! Experimental, bounded JSONL v1 adapter for the asynchronous Rust owner.
//!
//! This is an opt-in subset, not a replacement for the production RPC mode.
//! Unsupported commands fail explicitly. The caller owns approval policy;
//! the CLI denies mutations because stdin belongs exclusively to this protocol.
use std::{collections::BTreeMap, io, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use harvest_runtime::MAX_EVENT_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use tokio::{
	io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
	sync::{mpsc, watch},
};

use crate::{
	chat::ChatMessage,
	durable_session::stored_message,
	session_engine::{
		RunHandle, SessionEngineEventKind, SessionHandle, SessionSnapshot, SessionState,
	},
};

const IO_CAPACITY: usize = 32;
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
	let sink = Sink(output_tx);
	let result = async {
		let mut subscription = session.subscribe().await?;
		sink.send(&json!({"type":"ready", "protocolVersion":1,
			"supportedProtocolVersions":[1], "maxFrameBytes":MAX_EVENT_BYTES,
			"maxReassembledFrameBytes":MAX_EVENT_BYTES})).await?;
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
}

struct Sink(mpsc::Sender<Vec<u8>>);
impl Sink {
	async fn send<T: Serialize + Sync>(&self, value: &T) -> Result<()> {
		self
			.0
			.send(encode(value)?)
			.await
			.context("RPC output closed")
	}

	async fn success<T: Serialize + Send + Sync>(
		&self,
		id: Option<&str>,
		command: &str,
		data: Option<T>,
	) -> Result<()> {
		let response = Response { id, kind: "response", command, success: true, data, error: None };
		match encode(&response) {
			Ok(bytes) => self.0.send(bytes).await.context("RPC output closed"),
			Err(_) => {
				self
					.error(id, command, "response exceeds the v1 frame limit; no history was truncated")
					.await
			},
		}
	}

	async fn error(&self, id: Option<&str>, command: &str, error: &str) -> Result<()> {
		self
			.send(&Response::<()> {
				id,
				kind: "response",
				command,
				success: false,
				data: None,
				error: Some(error),
			})
			.await
	}
}

struct LimitedFrame(Vec<u8>);
impl io::Write for LimitedFrame {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if bytes.len() > (MAX_EVENT_BYTES - 1).saturating_sub(self.0.len()) {
			return Err(io::Error::other("RPC output exceeds frame limit"));
		}
		self.0.extend_from_slice(bytes);
		Ok(bytes.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
	let mut bytes = LimitedFrame(Vec::new());
	serde_json::to_writer(&mut bytes, value)?;
	bytes.0.push(b'\n');
	Ok(bytes.0)
}

async fn write_frames<W: AsyncWrite + Unpin>(
	mut output: W,
	mut frames: mpsc::Receiver<Vec<u8>>,
) -> Result<()> {
	while let Some(frame) = frames.recv().await {
		tokio::time::timeout(WRITE_TIMEOUT, async {
			output.write_all(&frame).await?;
			output.flush().await
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
	let mut input = BufReader::new(input);
	let mut frame = Vec::new();
	let mut oversized = false;
	loop {
		let buffer = input.fill_buf().await?;
		if buffer.is_empty() {
			if oversized {
				frames
					.send(Input::Invalid("RPC input exceeds frame limit".into()))
					.await?;
			} else if !frame.is_empty() {
				frames.send(Input::Frame(frame)).await?;
			}
			return Ok(());
		}
		let newline = buffer.iter().position(|byte| *byte == b'\n');
		let count = newline.map_or(buffer.len(), |index| index + 1);
		if !oversized {
			if count > MAX_EVENT_BYTES.saturating_sub(frame.len()) {
				oversized = true;
				frame.clear();
			} else {
				frame.extend_from_slice(&buffer[..count]);
			}
		}
		input.consume(count);
		if newline.is_some() {
			let next = if oversized {
				Input::Invalid("RPC input exceeds frame limit".into())
			} else {
				Input::Frame(std::mem::take(&mut frame))
			};
			frames.send(next).await?;
			oversized = false;
		}
	}
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
				self
					.sink
					.error(id, command, "experimental Rust RPC supports protocol v1 only")
					.await?;
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
			| "get_last_assistant_text"
			| "set_session_name"
			| "handoff"
			| "get_messages_page"
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
