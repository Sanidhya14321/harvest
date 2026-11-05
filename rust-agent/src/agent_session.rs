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
	MemoryWarning { operation: String },
	PluginEvent { tool_call_id: String, event: crate::plugins::protocol::Event },
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
	tools:      tools::ToolRuntime,
	root:       PathBuf,
	log:        DurableSession,
	generation: u64,
	sequence:   u64,
	active:     Option<(RunHandle, CancelToken)>,
}
impl AgentSession {
	pub fn open(root: &Path, path: &Path, leaf: Option<&str>) -> Result<Self> {
		Self::open_with_storage(root, root, path, leaf)
	}

	/// Keep the session writer in an explicitly owned artifact directory while
	/// tools remain jailed to the workspace. Both roots are host-authorized.
	pub fn open_with_storage(
		root: &Path,
		storage: &Path,
		path: &Path,
		leaf: Option<&str>,
	) -> Result<Self> {
		Self::open_owned(root, storage, path, leaf, None, None)
	}

	/// Open with the host-approved settings/secret directory from the outset.
	pub fn open_with_storage_in(
		root: &Path,
		storage: &Path,
		path: &Path,
		leaf: Option<&str>,
		directory: Option<&Path>,
	) -> Result<Self> {
		Self::open_owned(root, storage, path, leaf, None, directory)
	}

	pub fn open_inheriting_memory(
		root: &Path,
		path: &Path,
		leaf: Option<&str>,
		memory: crate::memory::Inheritance,
	) -> Result<Self> {
		Self::open_owned(root, root, path, leaf, Some(memory), None)
	}

	fn open_owned(
		root: &Path,
		storage: &Path,
		path: &Path,
		leaf: Option<&str>,
		inherited: Option<crate::memory::Inheritance>,
		directory: Option<&Path>,
	) -> Result<Self> {
		let log = DurableSession::open_for_workspace(storage, path, leaf, root)?;
		let mut tools = tools::ToolRuntime::default();
		let settings = match &inherited {
			Some(memory) => memory.settings.clone(),
			None => crate::settings::Settings::load(
				crate::config::agent_dir_from(directory.map(Path::to_owned), |key| {
					std::env::var_os(key)
				})
				.as_deref(),
				root,
				&[],
			)?,
		};
		tools.configure_core(settings, log.artifact_store());
		tools.configure_session_directory(log.storage_directory());
		if let Some(memory) = inherited {
			tools.inherit_agent_directory(memory.agent_directory);
			tools.configure_memory(memory.backend)?;
			tools.configure_memory_lifecycle(false);
		} else {
			tools.initialize_memory_in(root, log.id(), directory)?;
		}
		Ok(Self { tools, root: root.canonicalize()?, log, generation: 0, sequence: 0, active: None })
	}

	/// A shared native backend is owned by the host, never tool-call arguments.
	pub fn configure_memory(
		&mut self,
		backend: Option<crate::memory::SharedMemoryBackend>,
	) -> Result<()> {
		if self.active.is_some() {
			bail!("cannot change memory backend during an active run");
		}
		self.tools.configure_memory(backend)
	}

	pub fn configure_memory_model(
		&mut self,
		model: std::sync::Arc<dyn Model + Send + Sync>,
	) -> Result<()> {
		if self.active.is_some() {
			bail!("cannot change the memory model during an active run");
		}
		self.tools.configure_memory_model(&self.root, model)
	}

	/// Evaluation hosts can disable automatic learning while keeping explicit
	/// tools.
	pub fn configure_memory_lifecycle(&mut self, enabled: bool) -> Result<()> {
		if self.active.is_some() {
			bail!("cannot change automatic memory during an active run");
		}
		self.tools.configure_memory_lifecycle(enabled);
		Ok(())
	}

	/// Select before a run. Snapshots remain volatile and are cleared on mode
	/// changes.
	pub fn configure_edit(&mut self, mode: crate::edit::Mode) -> Result<()> {
		if self.active.is_some() {
			bail!("cannot change edit mode during an active run");
		}
		self.tools.configure_edit(mode);
		Ok(())
	}

	pub fn configure_plugins(
		&mut self,
		configurations: &[crate::plugins::Configuration],
		cancellation: &CancelToken,
	) -> Result<()> {
		self
			.tools
			.configure_plugins(&self.root, configurations, cancellation)
	}

	pub fn configure_mcp(
		&mut self,
		configurations: &[crate::mcp::Configuration],
		cancellation: &CancelToken,
	) -> Result<()> {
		self
			.tools
			.configure_mcp(&self.root, configurations, cancellation)
	}

	pub fn configure_language_services(
		&mut self,
		lsp: Vec<crate::language_services::LspConfiguration>,
		dap: Vec<crate::language_services::DapConfiguration>,
	) -> Result<()> {
		self.tools.configure_language_services(&self.root, lsp, dap)
	}

	pub fn configure_browser(&mut self, options: harvest_browser::Options) {
		self.tools.configure_browser(options);
	}

	/// Immutable persisted payloads, including host tool details and raw usage.
	pub fn stored_messages(&self) -> &[Box<serde_json::value::RawValue>] {
		self.log.stored_messages()
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

	/// The `yield` submission of the last run, consumed once.
	pub fn take_yield(&self) -> Option<Result<String, String>> {
		self.tools.core_state().take_yield()
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

	/// Flush the configured automatic-retention suffix, then release the writer.
	/// Backend failure becomes a UI warning rather than changing a completed
	/// answer.
	pub fn close(mut self, sink: &mut dyn FnMut(SessionEvent) -> Result<()>) -> Result<()> {
		if self.active.is_some() {
			bail!("cannot close a session with an active run");
		}
		if self
			.tools
			.retain_memory(&self.root, self.log.id(), self.log.messages(), "shutdown", true)
			.is_err()
		{
			self.emit(EventKind::MemoryWarning { operation: "retention".into() }, sink)?;
		}
		Ok(())
	}

	/// Stop successfully at a durable tool-batch boundary when the host's
	/// checkpoint returns true. Every call in the batch has its result
	/// committed; no synthetic assistant response or provider usage is
	/// recorded.
	pub fn submit_until_tool_checkpoint(
		&mut self,
		prompt: &str,
		model: &dyn Model,
		approval: &dyn tools::Approval,
		options: &RunOptions,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
		checkpoint: &mut dyn FnMut(&[Box<serde_json::value::RawValue>]) -> Result<bool>,
	) -> Result<String> {
		let handle = self.begin(options.cancellation.clone())?;
		self.execute_until(&handle, prompt, model, approval, options, sink, checkpoint)
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
		self.execute_until(handle, prompt, model, approval, options, sink, &mut |_| Ok(false))
	}

	fn execute_until(
		&mut self,
		handle: &RunHandle,
		prompt: &str,
		model: &dyn Model,
		approval: &dyn tools::Approval,
		options: &RunOptions,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
		checkpoint: &mut dyn FnMut(&[Box<serde_json::value::RawValue>]) -> Result<bool>,
	) -> Result<String> {
		let Some((active, cancellation)) = &self.active else {
			bail!("run is not active");
		};
		if active.generation != handle.generation || active.identity != handle.identity {
			bail!("stale run generation");
		}
		let cancellation = cancellation.clone();
		let result = self.drive(prompt, model, approval, options, &cancellation, sink, checkpoint);
		self.active = None;
		let outcome = if cancellation.aborted() {
			"cancelled"
		} else if result.is_ok() {
			"completed"
		} else {
			"failed"
		};
		if self
			.tools
			.retain_memory(&self.root, self.log.id(), self.log.messages(), outcome, false)
			.is_err()
		{
			self.emit(EventKind::MemoryWarning { operation: "retention".into() }, sink)?;
		}
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
		usage: Option<&crate::usage::UsageReport>,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<()> {
		self.commit_details(message, model, usage, None, None, sink)
	}

	fn commit_details(
		&mut self,
		message: &chat::ChatMessage,
		model: &dyn Model,
		usage: Option<&crate::usage::UsageReport>,
		details: Option<&serde_json::Value>,
		tool_error: Option<bool>,
		sink: &mut dyn FnMut(SessionEvent) -> Result<()>,
	) -> Result<()> {
		let entry_id = self
			.log
			.commit_with_metadata_receipt(message, model.model_id(), usage, details, tool_error)?
			.entry_id;
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
		checkpoint: &mut dyn FnMut(&[Box<serde_json::value::RawValue>]) -> Result<bool>,
	) -> Result<String> {
		if prompt.is_empty() || options.max_steps == 0 {
			bail!("prompt and positive max steps are required");
		}
		cancellation.heartbeat()?;
		self.log.ensure_ready()?;
		// Resolve reference context before modifying the session.
		let mut context = agent::context(&self.root, prompt, options)?;
		self.emit(EventKind::Started, sink)?;
		if self
			.tools
			.refresh_memory_configuration(
				&self.root,
				self.log.id(),
				self.log.messages(),
				options.user_agent_dir.as_deref(),
			)
			.is_err()
		{
			self.emit(EventKind::MemoryWarning { operation: "configuration".into() }, sink)?;
		}

		let memory =
			match self
				.tools
				.memory_context(&self.root, prompt, self.log.messages(), cancellation)
			{
				Ok(context) => context,
				Err(_) => {
					if !cancellation.aborted() {
						self.emit(EventKind::MemoryWarning { operation: "recall".into() }, sink)?;
					}
					None
				},
			};
		cancellation.heartbeat()?;
		let skill_prompt = self.tools.prepare_skill_prompt(&self.root, prompt)?;
		cancellation.heartbeat()?;
		self.commit(&chat::user(skill_prompt.as_deref().unwrap_or(prompt)), model, None, sink)?;
		let mut messages = vec![agent::system_message()];
		let mut context_index = None;
		if !context.is_empty() {
			context_index = Some(messages.len());
			messages.push(chat::system(&context));
		}
		if let Some(memory) = memory {
			messages.push(chat::system(&memory));
		}
		messages.extend_from_slice(self.log.messages());
		for step in 0..options.max_steps {
			cancellation.heartbeat()?;
			if step > 0 {
				agent::refresh_context(
					&mut messages,
					&mut context,
					&mut context_index,
					&self.root,
					prompt,
					options,
				)?;
			}
			let response =
				model.stream_response(&messages, &self.tools.specs(), cancellation, &mut |text| {
					cancellation.heartbeat()?;
					self.emit(EventKind::TextDelta { text: text.into() }, sink)
				})?;
			cancellation.heartbeat()?;
			let usage = response.usage;
			let response = response.message;
			if response.role != "assistant" {
				bail!("provider returned a non-assistant message");
			}
			let calls = response.tool_calls.clone().unwrap_or_default();
			let answer = response.content.clone().unwrap_or_default();
			self.commit(&response, model, usage.as_ref(), sink)?;
			messages.push(response);
			if calls.is_empty() {
				return Ok(answer);
			}
			for call in calls {
				cancellation.heartbeat()?;
				// Temporarily move the sole runtime owner so the event callback may
				// use the session's existing sequence/sink without aliasing tools.
				let mut runtime = std::mem::take(&mut self.tools);
				let root = self.root.clone();
				let result = if call.function.name == "task" {
					crate::task_tool::run_with_memory(
						&root,
						&call.function.arguments,
						model,
						approval,
						options,
						Some(runtime.memory_inheritance()),
					)
					.map(tools::ToolOutput::text)
				} else {
					runtime.invoke_detailed_with_events(
						&root,
						&call,
						approval,
						cancellation,
						&mut |event| {
							self
								.emit(EventKind::PluginEvent { tool_call_id: call.id.clone(), event }, sink)
						},
					)
				};
				self.tools = runtime;
				let output = match result {
					Ok(output) => output,
					Err(error) => tools::ToolOutput::failure(format!("error: {error:#}")),
				};
				let result = chat::tool_result(&call.id, output.content);
				self.commit_details(
					&result,
					model,
					None,
					output.details.as_ref(),
					Some(output.is_error),
					sink,
				)?;
				messages.push(result);
			}
			cancellation.heartbeat()?;
			let stop = checkpoint(self.stored_messages())?;
			cancellation.heartbeat()?;
			if stop {
				return Ok(String::new());
			}
		}
		bail!("stopped after {} steps", options.max_steps)
	}
}
