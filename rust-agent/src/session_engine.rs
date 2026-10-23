//! Bounded asynchronous ownership for the experimental Rust SDK.
//!
//! One blocking driver retains the session's writer lease while the async owner
//! receives commands and publishes events. Existing model and approval
//! callbacks run on that driver, without a session mutex. Cooperative providers
//! and tools observe cancellation; an arbitrary blocking callback cannot be
//! forcibly interrupted. Shutdown waits for it rather than releasing ownership
//! early.

use std::{
	collections::{BTreeSet, VecDeque},
	fmt,
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{
	MAX_EVENT_BYTES, MAX_HTTP_BYTES, MAX_TOOL_BYTES,
	cancel::{AbortReason, AbortToken, CancelToken},
	truncate_bytes,
};
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::{
	agent::{self, RunOptions},
	chat::{self, ChatMessage},
	durable_session::DurableSession,
	provider::Model,
	tools::{self, Approval},
};

const MAX_CAPACITY: usize = 256;
const DELTA_BYTES: usize = 16 * 1024;
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Bounds are per session. The optional turn limit is an explicit safety
/// budget, not the scheduling mechanism; SDK runs have no fixed step count by
/// default.
#[derive(Debug, Clone)]
pub struct SessionEngineOptions {
	pub plugins:            Vec<crate::plugins::Configuration>,
	pub browser:            Option<harvest_browser::Options>,
	pub command_capacity:   usize,
	pub event_capacity:     usize,
	pub follow_up_capacity: usize,
	pub max_model_turns:    Option<u32>,
	pub user_agent_dir:     Option<PathBuf>,
	pub tags:               BTreeSet<String>,
}

impl Default for SessionEngineOptions {
	fn default() -> Self {
		Self {
			browser:            None,
			plugins:            Vec::new(),
			command_capacity:   32,
			event_capacity:     64,
			follow_up_capacity: 16,
			max_model_turns:    None,
			user_agent_dir:     None,
			tags:               BTreeSet::new(),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
	Closed,
	Busy,
	QueueFull,
	InvalidPrompt,
	StaleRun,
	Cancelled,
	Shutdown,
	Failed(String),
}

impl fmt::Display for SessionError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Closed => formatter.write_str("session owner is closed"),
			Self::Busy => formatter.write_str("session is busy; queue a follow-up instead"),
			Self::QueueFull => formatter.write_str("session follow-up queue is full"),
			Self::InvalidPrompt => {
				formatter.write_str("prompt must be nonempty and within the frame limit")
			},
			Self::StaleRun => formatter.write_str("run handle belongs to another session owner"),
			Self::Cancelled => formatter.write_str("run was cancelled"),
			Self::Shutdown => formatter.write_str("session is shutting down"),
			Self::Failed(message) => formatter.write_str(message),
		}
	}
}
impl std::error::Error for SessionError {}

type SessionResult<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RunId {
	pub owner:      u64,
	pub generation: u64,
}

/// Cancels only this accepted request, including while it is queued. Keeping an
/// old handle never grants cancellation authority over a successor run.
#[derive(Clone)]
pub struct RunHandle {
	id:    RunId,
	abort: AbortToken,
}
impl RunHandle {
	pub const fn id(&self) -> RunId {
		self.id
	}

	pub fn abort(&self) {
		self.abort.abort(AbortReason::User);
	}
}

pub struct RunTicket {
	pub handle: RunHandle,
	completion: oneshot::Receiver<SessionResult<String>>,
}
impl RunTicket {
	pub async fn wait(self) -> SessionResult<String> {
		self.completion.await.unwrap_or(Err(SessionError::Closed))
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
	Idle,
	Running,
	Stopping,
}

/// A consistent owner snapshot containing committed messages only.
///
/// Provisional text remains in the event stream. Cloning history is requested
/// explicitly rather than performed for every streamed delta or append.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
	pub session_id:        String,
	pub owner:             u64,
	pub sequence:          u64,
	pub state:             SessionState,
	pub active:            Option<RunId>,
	pub queued:            Vec<RunId>,
	pub follow_ups_paused: bool,
	pub leaf:              Option<String>,
	pub messages:          Vec<ChatMessage>,
	pub stored_messages:   Vec<Box<RawValue>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEngineEventKind {
	Queued,
	Started,
	TextDelta { text: String },
	Committed { entry_id: String, role: String, message: Box<RawValue> },
	Completed,
	Cancelled,
	Failed { message: String },
	Closed,
}

#[derive(Debug, Serialize)]
pub struct SessionEngineEvent {
	pub session_id: String,
	pub run:        Option<RunId>,
	pub sequence:   u64,
	pub event:      SessionEngineEventKind,
}

/// Subscription creation is serialized with events.
///
/// After a broadcast lag error, call `subscribe` again to obtain a fresh
/// committed snapshot and cursor; lag is surfaced, never silently treated as a
/// complete transcript.
pub struct SessionSubscription {
	pub snapshot: SessionSnapshot,
	pub events:   broadcast::Receiver<Arc<SessionEngineEvent>>,
}

#[derive(Clone)]
pub struct SessionHandle {
	owner:    u64,
	commands: mpsc::Sender<Command>,
	closed:   watch::Receiver<bool>,
}

impl SessionHandle {
	pub async fn submit(&self, prompt: impl Into<String>) -> SessionResult<RunTicket> {
		self.request(prompt.into(), false).await
	}

	/// FIFO follow-ups start when idle. After cancellation they stay parked
	/// until `resume_follow_ups` is explicitly called; cancelling cannot
	/// restart work.
	pub async fn follow_up(&self, prompt: impl Into<String>) -> SessionResult<RunTicket> {
		self.request(prompt.into(), true).await
	}

	async fn request(&self, prompt: String, follow_up: bool) -> SessionResult<RunTicket> {
		if prompt.trim().is_empty() || prompt.len() > MAX_EVENT_BYTES / 2 {
			return Err(SessionError::InvalidPrompt);
		}
		let (reply, result) = oneshot::channel();
		self
			.commands
			.send(Command::Submit { prompt, follow_up, reply })
			.await
			.map_err(|_| SessionError::Closed)?;
		result.await.unwrap_or(Err(SessionError::Closed))
	}

	pub async fn snapshot(&self) -> SessionResult<SessionSnapshot> {
		let (reply, result) = oneshot::channel();
		self
			.commands
			.send(Command::Snapshot(reply))
			.await
			.map_err(|_| SessionError::Closed)?;
		result.await.map_err(|_| SessionError::Closed)
	}

	pub async fn subscribe(&self) -> SessionResult<SessionSubscription> {
		let (reply, result) = oneshot::channel();
		self
			.commands
			.send(Command::Subscribe(reply))
			.await
			.map_err(|_| SessionError::Closed)?;
		result.await.map_err(|_| SessionError::Closed)
	}

	/// Returns whether a matching active or queued request was still owned.
	pub async fn abort(&self, handle: &RunHandle) -> SessionResult<bool> {
		if handle.id.owner != self.owner {
			return Err(SessionError::StaleRun);
		}
		let (reply, result) = oneshot::channel();
		self
			.commands
			.send(Command::Abort { id: handle.id, reply })
			.await
			.map_err(|_| SessionError::Closed)?;
		result.await.map_err(|_| SessionError::Closed)
	}

	pub async fn resume_follow_ups(&self) -> SessionResult<bool> {
		let (reply, result) = oneshot::channel();
		self
			.commands
			.send(Command::Resume(reply))
			.await
			.map_err(|_| SessionError::Closed)?;
		result.await.unwrap_or(Err(SessionError::Closed))
	}

	/// Cancel active work, reject queued work, drain callbacks and release the
	/// writer lease. Dropping the last handle requests the same draining
	/// shutdown.
	pub async fn shutdown(&self) -> SessionResult<()> {
		let _ = self.commands.send(Command::Shutdown).await;
		let mut closed = self.closed.clone();
		loop {
			if *closed.borrow() {
				return Ok(());
			}
			closed.changed().await.map_err(|_| SessionError::Closed)?;
		}
	}
}

pub struct SessionEngine;
impl SessionEngine {
	pub async fn open(
		workspace: &Path,
		path: &Path,
		leaf: Option<&str>,
		model: Arc<dyn Model + Send + Sync>,
		approval: Arc<dyn Approval + Send + Sync>,
		options: SessionEngineOptions,
	) -> Result<SessionHandle> {
		for capacity in [options.command_capacity, options.event_capacity, options.follow_up_capacity]
		{
			if !(1..=MAX_CAPACITY).contains(&capacity) {
				bail!("session channel capacities must be between 1 and {MAX_CAPACITY}");
			}
		}
		if options.max_model_turns == Some(0) {
			bail!("model turn limit must be positive");
		}
		let root = workspace.to_path_buf();
		let path = path.to_path_buf();
		let leaf = leaf.map(str::to_owned);
		let driver = tokio::task::spawn_blocking(move || -> Result<Driver> {
			let root = root.canonicalize()?;
			let log = DurableSession::open(&root, &path, leaf.as_deref())?;
			let mut tools = tools::ToolRuntime::default();
			tools.configure_plugins(&root, &options.plugins, &CancelToken::default())?;
			if let Some(browser) = &options.browser {
				tools.configure_browser(browser.clone());
			}
			Ok(Driver { tools, root, log, model, approval, options })
		})
		.await
		.context("session opening task failed")??;
		let owner_id = loop {
			let id = NEXT_OWNER.load(Ordering::Relaxed);
			let next = id
				.checked_add(1)
				.context("session owner identity exhausted")?;
			if NEXT_OWNER
				.compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed)
				.is_ok()
			{
				break id;
			}
		};
		let (commands, receiver) = mpsc::channel(driver.options.command_capacity);
		let (events, _) = broadcast::channel(driver.options.event_capacity);
		let (worker_events, incoming) = mpsc::channel(driver.options.command_capacity);
		let (closed, closure) = watch::channel(false);
		let owner = Owner {
			id: owner_id,
			session_id: driver.log.id().into(),
			leaf: driver.log.leaf().map(str::to_owned),
			messages: driver.log.messages().to_vec(),
			stored_messages: driver.log.stored_messages().to_vec(),
			queue_capacity: driver.options.follow_up_capacity,
			driver: Some(driver),
			commands: receiver,
			commands_drained: false,
			events,
			worker_events,
			incoming,
			closed,
			active: None,
			queued: VecDeque::new(),
			paused: false,
			stopping: false,
			generation: 0,
			sequence: 0,
		};
		tokio::spawn(owner.run());
		Ok(SessionHandle { owner: owner_id, commands, closed: closure })
	}
}

enum Command {
	Submit {
		prompt:    String,
		follow_up: bool,
		reply:     oneshot::Sender<SessionResult<RunTicket>>,
	},
	Snapshot(oneshot::Sender<SessionSnapshot>),
	Subscribe(oneshot::Sender<SessionSubscription>),
	Abort {
		id:    RunId,
		reply: oneshot::Sender<bool>,
	},
	Resume(oneshot::Sender<SessionResult<bool>>),
	Shutdown,
}

struct Request {
	prompt:       String,
	handle:       RunHandle,
	cancellation: CancelToken,
	completion:   oneshot::Sender<SessionResult<String>>,
}

struct Active {
	handle:     RunHandle,
	completion: oneshot::Sender<SessionResult<String>>,
	worker:     tokio::task::JoinHandle<()>,
}

struct Driver {
	tools:    tools::ToolRuntime,
	root:     PathBuf,
	log:      DurableSession,
	model:    Arc<dyn Model + Send + Sync>,
	approval: Arc<dyn Approval + Send + Sync>,
	options:  SessionEngineOptions,
}

enum WorkerPacket {
	Delta {
		run:  RunId,
		text: String,
	},
	Committed {
		run:      RunId,
		entry_id: String,
		message:  ChatMessage,
		stored:   Box<RawValue>,
	},
	Finished {
		run:      RunId,
		driver:   Box<Driver>,
		result:   SessionResult<String>,
		poisoned: bool,
	},
}

struct Owner {
	id:               u64,
	session_id:       String,
	leaf:             Option<String>,
	messages:         Vec<ChatMessage>,
	stored_messages:  Vec<Box<RawValue>>,
	queue_capacity:   usize,
	driver:           Option<Driver>,
	commands:         mpsc::Receiver<Command>,
	commands_drained: bool,
	events:           broadcast::Sender<Arc<SessionEngineEvent>>,
	worker_events:    mpsc::Sender<WorkerPacket>,
	incoming:         mpsc::Receiver<WorkerPacket>,
	closed:           watch::Sender<bool>,
	active:           Option<Active>,
	queued:           VecDeque<Request>,
	paused:           bool,
	stopping:         bool,
	generation:       u64,
	sequence:         u64,
}

impl Owner {
	fn snapshot(&self) -> SessionSnapshot {
		SessionSnapshot {
			session_id:        self.session_id.clone(),
			owner:             self.id,
			sequence:          self.sequence,
			state:             if self.stopping {
				SessionState::Stopping
			} else if self.active.is_some() {
				SessionState::Running
			} else {
				SessionState::Idle
			},
			active:            self.active.as_ref().map(|active| active.handle.id),
			queued:            self
				.queued
				.iter()
				.map(|request| request.handle.id)
				.collect(),
			follow_ups_paused: self.paused,
			leaf:              self.leaf.clone(),
			messages:          self.messages.clone(),
			stored_messages:   self.stored_messages.clone(),
		}
	}

	fn emit(&mut self, run: Option<RunId>, event: SessionEngineEventKind) -> Result<()> {
		self.sequence = self
			.sequence
			.checked_add(1)
			.context("session event sequence exhausted")?;
		let _ = self.events.send(Arc::new(SessionEngineEvent {
			session_id: self.session_id.clone(),
			run,
			sequence: self.sequence,
			event,
		}));
		Ok(())
	}

	async fn run(mut self) -> Result<()> {
		loop {
			if self.stopping && self.active.is_none() {
				let driver = self.driver.take();
				// Stateful tools can need bounded native-process shutdown. Reap them
				// off the async scheduler, then release the writer before Closed.
				tokio::task::spawn_blocking(move || drop(driver))
					.await
					.context("session resource shutdown failed")?;
				self.emit(None, SessionEngineEventKind::Closed)?;
				self.closed.send_replace(true);
				return Ok(());
			}
			if !self.stopping && !self.paused && self.active.is_none() {
				self.start_next()?;
			}
			tokio::select! {
				command = self.commands.recv(), if !self.commands_drained => {
					if let Some(command) = command { self.command(command)?; }
					else { self.commands_drained = true; self.stop(); }
				},
				packet = self.incoming.recv(), if self.active.is_some() => {
					if let Some(packet) = packet { self.packet(packet).await?; }
				},
			}
		}
	}

	fn command(&mut self, command: Command) -> Result<()> {
		match command {
			Command::Submit { prompt, follow_up, reply } => {
				if reply.is_closed() {
					return Ok(());
				}
				if self.stopping {
					let _ = reply.send(Err(SessionError::Shutdown));
				} else if !follow_up && (self.active.is_some() || !self.queued.is_empty()) {
					let _ = reply.send(Err(SessionError::Busy));
				} else if self.queued.len() >= self.queue_capacity {
					let _ = reply.send(Err(SessionError::QueueFull));
				} else {
					self.generation = self
						.generation
						.checked_add(1)
						.context("run generation exhausted")?;
					let mut cancellation = CancelToken::default();
					let handle = RunHandle {
						id:    RunId { owner: self.id, generation: self.generation },
						abort: cancellation.emplace_abort_token(),
					};
					let (completion, result) = oneshot::channel();
					self.emit(Some(handle.id), SessionEngineEventKind::Queued)?;
					self.queued.push_back(Request {
						prompt,
						handle: handle.clone(),
						cancellation,
						completion,
					});
					if let Err(Ok(ticket)) = reply.send(Ok(RunTicket { handle, completion: result })) {
						ticket.handle.abort();
					}
				}
			},
			Command::Snapshot(reply) => {
				let _ = reply.send(self.snapshot());
			},
			Command::Subscribe(reply) => {
				let subscription =
					SessionSubscription { snapshot: self.snapshot(), events: self.events.subscribe() };
				let _ = reply.send(subscription);
			},
			Command::Abort { id, reply } => {
				let owned = self
					.active
					.as_ref()
					.map(|active| &active.handle)
					.filter(|handle| handle.id == id)
					.or_else(|| {
						self
							.queued
							.iter()
							.find(|request| request.handle.id == id)
							.map(|request| &request.handle)
					});
				if let Some(handle) = owned {
					handle.abort();
				}
				let _ = reply.send(owned.is_some());
			},
			Command::Resume(reply) => {
				let result = if self.stopping {
					Err(SessionError::Shutdown)
				} else if self.active.is_some() {
					Err(SessionError::Busy)
				} else {
					let was_paused = self.paused;
					self.paused = false;
					Ok(was_paused)
				};
				let _ = reply.send(result);
			},
			Command::Shutdown => self.stop(),
		}
		Ok(())
	}

	fn stop(&mut self) {
		self.stopping = true;
		self.commands.close();
		if let Some(active) = &self.active {
			active.handle.abort.abort(AbortReason::Signal);
		}
		for request in self.queued.drain(..) {
			let _ = request.completion.send(Err(SessionError::Shutdown));
		}
	}

	fn start_next(&mut self) -> Result<()> {
		while let Some(request) = self.queued.pop_front() {
			if request.cancellation.aborted() {
				self.emit(Some(request.handle.id), SessionEngineEventKind::Cancelled)?;
				let _ = request.completion.send(Err(SessionError::Cancelled));
				continue;
			}
			let mut driver = self.driver.take().context("session driver is not owned")?;
			let events = self.worker_events.clone();
			let run = request.handle.id;
			self.emit(Some(run), SessionEngineEventKind::Started)?;
			let worker = tokio::task::spawn_blocking(move || {
				let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					driver.drive(run, &request.prompt, &request.cancellation, &events)
				}));
				let poisoned = outcome.is_err();
				let result = match outcome {
					Ok(Ok(answer)) => Ok(answer),
					Ok(Err(error))
						if request.cancellation.aborted()
							&& error.downcast_ref::<PersistenceFailure>().is_none() =>
					{
						Err(SessionError::Cancelled)
					},
					Ok(Err(error)) => Err(SessionError::Failed(
						truncate_bytes(&format!("{error:#}"), MAX_TOOL_BYTES).into(),
					)),
					Err(_) => Err(SessionError::Failed(
						"model or host callback panicked; session owner closed".into(),
					)),
				};
				let _ = events.blocking_send(WorkerPacket::Finished {
					run,
					driver: Box::new(driver),
					result,
					poisoned,
				});
			});
			self.active =
				Some(Active { handle: request.handle, completion: request.completion, worker });
			break;
		}
		Ok(())
	}

	async fn packet(&mut self, packet: WorkerPacket) -> Result<()> {
		match packet {
			WorkerPacket::Delta { run, text } => {
				self.emit(Some(run), SessionEngineEventKind::TextDelta { text })?;
			},
			WorkerPacket::Committed { run, entry_id, message, stored } => {
				let role = message.role.clone();
				self.messages.push(message);
				self.stored_messages.push(stored.clone());
				self.leaf = Some(entry_id.clone());
				self.emit(Some(run), SessionEngineEventKind::Committed {
					entry_id,
					role,
					message: stored,
				})?;
			},
			WorkerPacket::Finished { run, driver, result, poisoned } => {
				let active = self
					.active
					.take()
					.context("run completion has no active owner")?;
				if active.handle.id != run {
					bail!("stale run completion");
				}
				active.worker.await.context("session driver task failed")?;
				self.driver = Some(*driver);
				let event = match &result {
					Ok(_) => SessionEngineEventKind::Completed,
					Err(SessionError::Cancelled) => {
						self.paused = true;
						SessionEngineEventKind::Cancelled
					},
					Err(error) => {
						self.paused = true;
						SessionEngineEventKind::Failed { message: error.to_string() }
					},
				};
				self.emit(Some(run), event)?;
				let _ = active.completion.send(result);
				if poisoned {
					self.stop();
				}
			},
		}
		Ok(())
	}
}

impl Drop for Owner {
	fn drop(&mut self) {
		if let Some(active) = &self.active {
			active.handle.abort.abort(AbortReason::Signal);
		}
	}
}

#[derive(Debug)]
struct PersistenceFailure(anyhow::Error);
impl fmt::Display for PersistenceFailure {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "session persistence failed: {:#}", self.0)
	}
}
impl std::error::Error for PersistenceFailure {}

impl Driver {
	fn commit(
		&mut self,
		run: RunId,
		message: ChatMessage,
		events: &mpsc::Sender<WorkerPacket>,
	) -> Result<()> {
		self.commit_with_usage(run, message, events, None)
	}

	fn commit_with_usage(
		&mut self,
		run: RunId,
		message: ChatMessage,
		events: &mpsc::Sender<WorkerPacket>,
		usage: Option<&crate::usage::UsageReport>,
	) -> Result<()> {
		let receipt = self
			.log
			.commit_with_usage_receipt(&message, self.model.model_id(), usage)
			.map_err(PersistenceFailure)?;
		events
			.blocking_send(WorkerPacket::Committed {
				run,
				entry_id: receipt.entry_id,
				message,
				stored: receipt.message,
			})
			.map_err(|_| anyhow::anyhow!("session event owner closed"))
	}

	fn drive(
		&mut self,
		run: RunId,
		prompt: &str,
		cancellation: &CancelToken,
		events: &mpsc::Sender<WorkerPacket>,
	) -> Result<String> {
		cancellation.heartbeat()?;
		self.log.ensure_ready()?;
		let context = agent::context(&self.root, prompt, &RunOptions {
			max_steps:      self.options.max_model_turns.unwrap_or(u32::MAX),
			user_agent_dir: self.options.user_agent_dir.clone(),
			tags:           self.options.tags.clone(),
			cancellation:   cancellation.clone(),
		})?;
		cancellation.heartbeat()?;
		self.commit(run, chat::user(prompt), events)?;
		let mut messages = vec![agent::system_message()];
		if !context.is_empty() {
			messages.push(chat::system(&context));
		}
		messages.extend_from_slice(self.log.messages());
		let mut turns = 0_u32;
		loop {
			cancellation.heartbeat()?;
			if self
				.options
				.max_model_turns
				.is_some_and(|limit| turns >= limit)
			{
				bail!("model turn limit reached");
			}
			turns = turns
				.checked_add(1)
				.context("model turn counter exhausted")?;
			let mut streamed = 0_usize;
			let response = self.model.stream_response(
				&messages,
				&self.tools.specs(),
				cancellation,
				&mut |text| {
					streamed = streamed
						.checked_add(text.len())
						.context("provider text size overflow")?;
					if streamed > MAX_HTTP_BYTES {
						bail!("provider text exceeds response limit");
					}
					let mut remaining = text;
					while !remaining.is_empty() {
						cancellation.heartbeat()?;
						let chunk = truncate_bytes(remaining, DELTA_BYTES);
						events
							.blocking_send(WorkerPacket::Delta { run, text: chunk.into() })
							.map_err(|_| anyhow::anyhow!("session event owner closed"))?;
						remaining = &remaining[chunk.len()..];
					}
					Ok(())
				},
			)?;
			cancellation.heartbeat()?;
			let usage = response.usage;
			let response = response.message;
			if response.role != "assistant" {
				bail!("provider returned a non-assistant message");
			}
			let calls = response.tool_calls.clone().unwrap_or_default();
			let answer = response.content.clone().unwrap_or_default();
			self.commit_with_usage(run, response.clone(), events, usage.as_ref())?;
			messages.push(response);
			if calls.is_empty() {
				return Ok(answer);
			}
			// Once an assistant tool batch is durable, every call must get a result,
			// including calls skipped by cancellation. Never strand replay pairs.
			for call in calls {
				let output = self
					.tools
					.invoke(&self.root, &call, self.approval.as_ref(), cancellation)
					.unwrap_or_else(|error| format!("error: {error:#}"));
				let message = chat::tool_result(&call.id, output);
				self.commit(run, message.clone(), events)?;
				messages.push(message);
			}
			cancellation.heartbeat()?;
		}
	}
}
