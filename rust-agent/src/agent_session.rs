//! Synchronous session host with durable message events and provisional deltas.
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use harvest_runtime::cancel::{AbortReason, AbortToken, CancelToken};
use serde::Serialize;

use crate::{
	agent::{self, RunOptions},
	chat,
	durable_session::DurableSession,
	provider::Model,
	tools,
};

#[derive(Clone)]
pub struct RunHandle {
	pub generation: u64,
	abort:          AbortToken,
	identity:       u64,
}
impl RunHandle {
	/// Each handle owns only its generation's cancellation flag.
	pub fn abort(&self) {
		self.abort.abort(AbortReason::User);
	}
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
	Started,
	TextDelta { text: String },
	Committed { entry_id: String, role: String },
	Completed,
	Failed,
}
#[derive(Debug, Serialize)]
pub struct SessionEvent {
	pub session_id: String,
	pub generation: u64,
	pub sequence:   u64,
	pub event:      EventKind,
}

pub struct AgentSession {
	root:       PathBuf,
	log:        DurableSession,
	generation: u64,
	sequence:   u64,
	active:     Option<(RunHandle, CancelToken)>,
}
impl AgentSession {
	pub fn open(root: &Path, path: &Path, leaf: Option<&str>) -> Result<Self> {
		let log = DurableSession::open(root, path, leaf)?;
		Ok(Self { root: root.canonicalize()?, log, generation: 0, sequence: 0, active: None })
	}

	pub fn session_id(&self) -> &str {
		self.log.id()
	}

	pub fn begin(&mut self, mut cancellation: CancelToken) -> Result<RunHandle> {
		if self.active.is_some() {
			bail!("session already has an active run");
		}
		self.generation = self
			.generation
			.checked_add(1)
			.ok_or_else(|| anyhow::anyhow!("run generation exhausted"))?;
		static HANDLE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
		let handle = RunHandle {
			generation: self.generation,
			abort:      cancellation.emplace_abort_token(),
			identity:   HANDLE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
		};
		self.active = Some((handle.clone(), cancellation));
		Ok(handle)
	}

	pub fn submit(
		&mut self,
		prompt: &str,
		model: &dyn Model,
		approval: &dyn tools::Approval,
		options: &RunOptions,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<String> {
		let handle = self.begin(options.cancellation.clone())?;
		self.execute(&handle, prompt, model, approval, options, sink)
	}

	pub fn execute(
		&mut self,
		handle: &RunHandle,
		prompt: &str,
		model: &dyn Model,
		approval: &dyn tools::Approval,
		options: &RunOptions,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<String> {
		let Some((active, cancellation)) = &self.active else {
			bail!("run is not active");
		};
		if active.generation != handle.generation || active.identity != handle.identity {
			bail!("stale run generation");
		}
		let cancellation = cancellation.clone();
		let result = self.drive(prompt, model, approval, options, &cancellation, sink);
		self.active = None;
		// Failure to deliver the terminal event is surfaced too, never hidden.
		self.emit(
			if result.is_ok() {
				EventKind::Completed
			} else {
				EventKind::Failed
			},
			sink,
		)?;
		result
	}

	fn emit(
		&mut self,
		event: EventKind,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<()> {
		self.sequence = self
			.sequence
			.checked_add(1)
			.ok_or_else(|| anyhow::anyhow!("event sequence exhausted"))?;
		sink(SessionEvent {
			session_id: self.log.id().into(),
			generation: self.generation,
			sequence: self.sequence,
			event,
		})
	}

	fn commit(
		&mut self,
		message: &chat::ChatMessage,
		model: &dyn Model,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<()> {
		let entry_id = self.log.commit(message, model.model_id())?;
		self.emit(EventKind::Committed { entry_id, role: message.role.clone() }, sink)
	}

	fn drive(
		&mut self,
		prompt: &str,
		model: &dyn Model,
		approval: &dyn tools::Approval,
		options: &RunOptions,
		cancellation: &CancelToken,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<String> {
		if prompt.is_empty() || options.max_steps == 0 {
			bail!("prompt and positive max steps are required");
		}
		cancellation.heartbeat()?;
		self.log.ensure_ready()?;
		// Resolve reference context before modifying the session.
		let context = agent::context(&self.root, prompt, options)?;
		self.emit(EventKind::Started, sink)?;
		self.commit(&chat::user(prompt), model, sink)?;
		let mut messages = vec![agent::system_message()];
		if !context.is_empty() {
			messages.push(chat::system(&context));
		}
		messages.extend_from_slice(self.log.messages());
		for _ in 0..options.max_steps {
			cancellation.heartbeat()?;
			let response = model.stream(&messages, &chat::specs(), cancellation, &mut |text| {
				cancellation.heartbeat()?;
				self.emit(EventKind::TextDelta { text: text.into() }, sink)
			})?;
			cancellation.heartbeat()?;
			if response.role != "assistant" {
				bail!("provider returned a non-assistant message");
			}
			let calls = response.tool_calls.clone().unwrap_or_default();
			let answer = response.content.clone().unwrap_or_default();
			self.commit(&response, model, sink)?;
			messages.push(response);
			if calls.is_empty() {
				return Ok(answer);
			}
			for call in calls {
				cancellation.heartbeat()?;
				let output =
					match tools::invoke_with_cancellation(&self.root, &call, approval, cancellation) {
						Ok(output) => output,
						Err(error) => format!("error: {error:#}"),
					};
				let result = chat::tool_result(&call.id, output);
				self.commit(&result, model, sink)?;
				messages.push(result);
			}
		}
		bail!("stopped after {} steps", options.max_steps)
	}
}
