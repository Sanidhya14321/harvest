//! Minimal native interactive mode (port of the core of
//! packages/coding-agent/src/modes/interactive-mode.ts).
//!
//! The UI owns the terminal; nothing on this path may write to stdout/stderr
//! except through the renderer. Tool approval is asked in-TUI: the blocking
//! session driver sends a request and waits on a reply channel.

use std::{
	collections::VecDeque,
	io::{self, Write as _},
	path::PathBuf,
	sync::{Arc, mpsc as std_mpsc},
	time::Duration,
};

use anyhow::{Context, Result};
use harvest_runtime::{
	BoundedWriter, JailedFs, MAX_EVENT_BYTES, MAX_HTTP_BYTES, MAX_TOOL_BYTES, truncate_bytes,
};
use harvest_tui::{
	component::Component,
	editor::{Editor, EditorAction},
	keyparse::Key,
	layout::viewport,
	render::Renderer,
	sanitize::{display_lines, replace_tabs, shorten_path, truncate_to_width},
	terminal::TerminalGuard,
};
use serde_json::Value;
use tokio::sync::{broadcast::error::RecvError, mpsc};

use crate::{
	chat::{ChatMessage, ToolCall},
	provider::Model,
	session_engine::{
		RunHandle, SessionEngine, SessionEngineEvent, SessionEngineEventKind, SessionEngineOptions,
	},
	tools::Approval,
};

mod completion;
mod history;
mod input;

/// Lines of a tool result shown in the transcript.
const TOOL_PREVIEW_LINES: usize = 8;
/// Bytes of tool arguments shown in a call header.
const TOOL_ARGS_PREVIEW: usize = 2000;
const MAX_TRANSCRIPT_ENTRIES: usize = 2048;
const INPUT_CAPACITY: usize = 64;
const MAX_DISPLAY_ROWS: usize = 4096;

const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

pub struct ChatOptions {
	pub model:     Arc<dyn Model + Send + Sync>,
	pub model_id:  String,
	pub workspace: PathBuf,
	pub session:   PathBuf,
	pub leaf:      Option<String>,
	pub engine:    SessionEngineOptions,
}

struct PendingChoice {
	request:      crate::plugins::protocol::ChoiceRequest,
	reply:        std_mpsc::Sender<Option<usize>>,
	cancellation: harvest_runtime::cancel::CancelToken,
	selected:     usize,
}

struct ApprovalRequest {
	name:     String,
	display:  String,
	offset:   usize,
	reviewed: bool,
	reply:    std_mpsc::Sender<bool>,
	choice:   Option<PendingChoice>,
}

impl ApprovalRequest {
	fn deny(self) {
		let _ = self.reply.send(false);
		if let Some(choice) = self.choice {
			let _ = choice.reply.send(None);
		}
	}
}

struct TuiApproval {
	requests: mpsc::Sender<ApprovalRequest>,
}

impl Approval for TuiApproval {
	fn approve(&self, call: &ToolCall) -> Result<bool> {
		// Display the entire final call (including identity), with escaped
		// controls and an allocation bound. Approval never uses a tool preview.
		let mut bytes = BoundedWriter::new(MAX_EVENT_BYTES);
		write!(&mut bytes, "{call:?}")?;
		let display = String::from_utf8(bytes.into_inner())?;
		let (reply, answer) = std_mpsc::channel();
		self
			.requests
			.blocking_send(ApprovalRequest {
				name: call.function.name.clone(),
				display,
				offset: 0,
				reviewed: false,
				reply,
				choice: None,
			})
			.map_err(|_| anyhow::anyhow!("interactive UI closed"))?;
		// Fail closed: a dropped UI denies.
		Ok(answer.recv().unwrap_or(false))
	}

	fn choose(
		&self,
		request: &crate::plugins::protocol::ChoiceRequest,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<Option<usize>> {
		crate::plugins::protocol::event_bytes(&crate::plugins::protocol::Event::Choices {
			title:   request.title.clone(),
			choices: request.choices.clone(),
		})?;
		anyhow::ensure!(
			!request.choices.is_empty()
				&& request.plugin.len() <= 128
				&& request.tool_call_id.len() <= 256,
			"invalid plugin UI request"
		);
		let (reply, answer) = std_mpsc::channel();
		let (unused, _) = std_mpsc::channel();
		let mut pending = ApprovalRequest {
			name:     request.plugin.clone(),
			display:  String::new(),
			offset:   0,
			reviewed: false,
			reply:    unused,
			choice:   Some(PendingChoice {
				request: request.clone(),
				reply,
				cancellation: cancellation.clone(),
				selected: 0,
			}),
		};
		loop {
			cancellation.heartbeat()?;
			match self.requests.try_send(pending) {
				Ok(()) => break,
				Err(mpsc::error::TrySendError::Closed(_)) => anyhow::bail!("native UI closed"),
				Err(mpsc::error::TrySendError::Full(request)) => {
					pending = request;
					std::thread::sleep(Duration::from_millis(10));
				},
			}
		}
		loop {
			cancellation.heartbeat()?;
			match answer.recv_timeout(Duration::from_millis(10)) {
				Ok(value) => return Ok(value),
				Err(std_mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
				Err(std_mpsc::RecvTimeoutError::Timeout) => {},
			}
		}
	}
}

#[derive(Clone, Copy)]
enum Role {
	User,
	Assistant,
	ToolCall,
	ToolResult,
	Notice,
	Error,
}

struct Entry {
	role: Role,
	text: String,
}

impl Entry {
	fn render(&self, width: usize) -> Vec<String> {
		let (marker, style) = match self.role {
			Role::User => ("> ", BOLD),
			Role::Assistant => ("", ""),
			Role::ToolCall => ("• ", CYAN),
			Role::ToolResult => ("  ", DIM),
			Role::Notice => ("", DIM),
			Role::Error => ("! ", RED),
		};
		let inner = width
			.saturating_sub(harvest_tui::sanitize::visible_width(marker))
			.max(1);
		let rendered = if matches!(self.role, Role::Assistant) {
			harvest_tui::markdown::render(&self.text, inner)
		} else {
			display_lines(&self.text, inner)
		};
		let mut lines: Vec<String> = rendered
			.into_iter()
			.enumerate()
			.map(|(i, l)| {
				let lead = if i == 0 { marker } else { "  " };
				let lead = if marker.is_empty() { "" } else { lead };
				format!("{style}{lead}{l}{RESET}")
			})
			.collect();
		lines.push(String::new());
		lines
	}
}

struct App {
	entries:          VecDeque<Entry>,
	entry_bytes:      usize,
	trimmed:          bool,
	streaming:        String,
	stream_truncated: bool,
	editor:           Editor,
	workspace:        Arc<JailedFs>,
	agent_directory:  Option<PathBuf>,
	session_id:       String,
	history:          Option<Arc<history::History>>,
	history_prompts:  Vec<String>,
	completion:       Option<completion::Menu>,
	completion_epoch: u64,
	completion_task:  Option<tokio::task::JoinHandle<(u64, Result<completion::Suggestions>)>>,
	pending:          Option<ApprovalRequest>,
	active:           Option<RunHandle>,
	runs:             Vec<RunHandle>,
	scroll:           usize,
	status:           String,
	quit:             bool,
}

impl App {
	fn restore_editor_history(&mut self, messages: &[ChatMessage]) {
		let mut restored = messages
			.iter()
			.rev()
			.filter(|message| message.role == "user")
			.filter_map(|message| message.content.as_deref())
			.take(harvest_tui::editor::MAX_HISTORY_ENTRIES)
			.map(|text| {
				crate::managed::invocation::original_prompt(text)
					.map(std::borrow::Cow::Owned)
					.unwrap_or(std::borrow::Cow::Borrowed(text))
			})
			.scan(0usize, |bytes, text| {
				*bytes = bytes.saturating_add(text.len());
				(*bytes <= harvest_tui::editor::MAX_HISTORY_BYTES).then_some(text)
			})
			.collect::<Vec<_>>();
		restored.reverse();
		self.editor.load_history(
			self
				.history_prompts
				.iter()
				.map(String::as_str)
				.chain(restored.iter().map(|text| text.as_ref())),
		);
	}

	fn invalidate_completion(&mut self) {
		self.completion = None;
		self.completion_epoch = self.completion_epoch.saturating_add(1);
	}

	fn request_completion(&mut self) {
		// Keep one filesystem discovery job alive at a time. Draft changes
		// invalidate its result without accumulating overlapping blocking work.
		if self.completion_task.is_some() {
			return;
		}
		self.invalidate_completion();
		let epoch = self.completion_epoch;
		let context = self.editor.completion_context();
		let workspace = Arc::clone(&self.workspace);
		let directory = self.agent_directory.clone();
		self.completion_task = Some(tokio::task::spawn_blocking(move || {
			(epoch, completion::paths(workspace, context, directory))
		}));
	}

	fn request_history(&mut self, query: String) {
		if self.completion_task.is_some() {
			return;
		}
		let Some(history) = self.history.as_ref().map(Arc::clone) else {
			self.push(
				Role::Notice,
				"Persistent history is unavailable; Up/Down still recalls this session's prompts.",
			);
			return;
		};
		self.invalidate_completion();
		let epoch = self.completion_epoch;
		let context = self.editor.completion_context();
		self.completion_task = Some(tokio::task::spawn_blocking(move || {
			(epoch, completion::history(history, context, query))
		}));
	}

	fn finish_completion(
		&mut self,
		outcome: std::result::Result<(u64, Result<completion::Suggestions>), tokio::task::JoinError>,
	) {
		match outcome {
			Ok((epoch, result)) if epoch == self.completion_epoch && self.pending.is_none() => {
				match result {
					Ok(suggestions) if self.editor.matches_completion(&suggestions.context) => {
						if suggestions.items.is_empty() {
							self.push(Role::Notice, "No matching completions.");
							return;
						}
						let apply_single = suggestions.items.len() == 1 && !suggestions.history;
						self.completion = Some(completion::Menu { suggestions, selected: 0 });
						if apply_single {
							self.accept_completion(false);
						}
					},
					Ok(_) => {},
					Err(error) => self.push(Role::Notice, format!("Completion unavailable: {error:#}")),
				}
			},
			Err(error) => self.push(Role::Notice, format!("Completion worker stopped: {error}")),
			_ => {},
		}
	}

	fn accept_completion(&mut self, chain_directory: bool) {
		let Some(menu) = self.completion.take() else {
			return;
		};
		let Some(item) = menu.suggestions.items.get(menu.selected) else {
			return;
		};
		let applied = if menu.suggestions.history {
			self.editor.matches_completion(&menu.suggestions.context)
				&& self.editor.set_text(&item.value)
		} else {
			self.editor.apply_completion(
				&menu.suggestions.context,
				menu.suggestions.replace_characters,
				&item.value,
			)
		};
		self.invalidate_completion();
		if !applied {
			self.push(
				Role::Notice,
				"Completion no longer matches the draft or exceeds the editor limit.",
			);
		} else if chain_directory && item.directory {
			self.request_completion();
		}
	}

	fn handle_completion_key(&mut self, key: &Key) -> bool {
		if *key == Key::Ctrl('r') {
			self.request_history(self.editor.text());
			return true;
		}
		if let Some(menu) = &mut self.completion {
			match key {
				Key::Up | Key::BackTab => menu.select(-1),
				Key::Down => menu.select(1),
				Key::PageUp => menu.select(-8),
				Key::PageDown => menu.select(8),
				Key::Enter | Key::Tab => {
					self.accept_completion(true);
				},
				Key::Right
					if self.editor.completion_context().column
						== self.editor.completion_context().line.chars().count() =>
				{
					self.accept_completion(true)
				},
				Key::Escape => self.invalidate_completion(),
				_ => return false,
			}
			return true;
		}
		if matches!(key, Key::Tab | Key::BackTab) {
			self.request_completion();
			return true;
		}
		false
	}

	async fn persist_history(&mut self, prompt: String) {
		let Some(history) = self.history.as_ref().map(Arc::clone) else {
			return;
		};
		let workspace = self.workspace.root().to_owned();
		let session_id = self.session_id.clone();
		let result = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
			history.record(&prompt, &workspace, &session_id)?;
			Ok(history
				.recent(100)?
				.into_iter()
				.rev()
				.map(|entry| entry.prompt)
				.collect())
		})
		.await
		.context("prompt history worker stopped")
		.and_then(|result| result);
		match result {
			Ok(prompts) => self.history_prompts = prompts,
			Err(error) => {
				self.push(Role::Notice, format!("Prompt history could not be saved: {error:#}"))
			},
		}
	}

	fn push(&mut self, role: Role, text: impl Into<String>) {
		let text = text.into();
		let mut visible = truncate_bytes(&text, MAX_TOOL_BYTES).to_owned();
		if visible.len() < text.len() {
			visible.push_str("\n[display truncated; complete committed text remains in the session]");
		}
		self.entry_bytes += visible.len();
		self.entries.push_back(Entry { role, text: visible });
		while self.entries.len() > MAX_TRANSCRIPT_ENTRIES || self.entry_bytes > MAX_HTTP_BYTES {
			if let Some(entry) = self.entries.pop_front() {
				self.entry_bytes = self.entry_bytes.saturating_sub(entry.text.len());
				self.trimmed = true;
			}
		}
		self.scroll = 0;
	}

	fn push_message(&mut self, role: &str, message: &Value) {
		let mut text = String::new();
		let mut calls = Vec::new();
		match message.get("content") {
			Some(Value::String(s)) => text.push_str(s),
			Some(Value::Array(parts)) => {
				for part in parts {
					match part.get("type").and_then(Value::as_str) {
						Some("text") => {
							text.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
						},
						Some("toolCall" | "tool_use") => calls.push((
							part
								.get("name")
								.and_then(Value::as_str)
								.unwrap_or("?")
								.to_owned(),
							part
								.get("arguments")
								.or_else(|| part.get("input"))
								.map(Value::to_string)
								.unwrap_or_default(),
						)),
						_ => {},
					}
				}
			},
			_ => {},
		}
		if let Some(Value::Array(tool_calls)) = message
			.get("tool_calls")
			.or_else(|| message.get("toolCalls"))
		{
			for call in tool_calls {
				let function = call.get("function").unwrap_or(call);
				calls.push((
					function
						.get("name")
						.and_then(Value::as_str)
						.unwrap_or("?")
						.to_owned(),
					match function.get("arguments") {
						Some(Value::String(s)) => s.clone(),
						Some(other) => other.to_string(),
						None => String::new(),
					},
				));
			}
		}
		match role {
			"user" => {
				let original = crate::managed::invocation::original_prompt(&text);
				self.push(Role::User, original.unwrap_or(text));
			},
			"assistant" => {
				self.streaming.clear();
				self.stream_truncated = false;
				if !text.trim().is_empty() {
					self.push(Role::Assistant, text);
				}
				for (name, args) in calls {
					self.push(Role::ToolCall, format!("{name} {}", preview_args(&args)));
				}
			},
			"tool" | "toolResult" => {
				let mut lines: Vec<&str> = text.lines().collect();
				let hidden = lines.len().saturating_sub(TOOL_PREVIEW_LINES);
				lines.truncate(TOOL_PREVIEW_LINES);
				let mut body = lines.join("\n");
				if hidden > 0 {
					body.push_str(&format!("\n… {hidden} more lines"));
				}
				self.push(Role::ToolResult, body);
			},
			_ => {},
		}
	}

	fn push_history(&mut self, message: &ChatMessage) {
		if let Ok(value) = serde_json::to_value(message) {
			self.push_message(&message.role, &value);
		}
	}

	fn on_event(&mut self, event: &SessionEngineEvent) {
		match &event.event {
			SessionEngineEventKind::Started => {
				self.active = self
					.runs
					.iter()
					.find(|handle| Some(handle.id()) == event.run)
					.cloned();
				self.status = "working… (ctrl+c to interrupt)".into();
			},
			SessionEngineEventKind::TextDelta { text } => {
				let preview = truncate_bytes(text, MAX_TOOL_BYTES.saturating_sub(self.streaming.len()));
				self.streaming.push_str(preview);
				self.stream_truncated |= preview.len() < text.len();
				self.scroll = 0;
			},
			SessionEngineEventKind::Committed { role, message, .. } => {
				if let Ok(value) = serde_json::from_str::<Value>(message.get()) {
					self.push_message(role, &value);
				}
			},
			SessionEngineEventKind::Completed => self.finish(event, ""),
			SessionEngineEventKind::Cancelled => {
				self.finish(event, "interrupted; /resume starts parked follow-ups")
			},
			SessionEngineEventKind::Failed { message } => {
				self.push(Role::Error, message.clone());
				self.finish(event, "");
			},
			SessionEngineEventKind::Closed => self.quit = true,
			SessionEngineEventKind::MemoryWarning { operation } => {
				self.push(Role::Notice, format!("memory {operation} is temporarily unavailable"));
			},
			SessionEngineEventKind::PluginEvent { event, .. } => match event {
				crate::plugins::protocol::Event::Progress { message } => {
					self.status = replace_tabs(message).replace('\n', " ")
				},
				crate::plugins::protocol::Event::Notification { title, message } => {
					self.push(Role::Notice, format!("{title}\n{message}"))
				},
				crate::plugins::protocol::Event::Choices { title, choices } => self.push(
					Role::Notice,
					format!(
						"{title}\n{}",
						choices
							.iter()
							.enumerate()
							.map(|(index, label)| format!("{}. {label}", index + 1))
							.collect::<Vec<_>>()
							.join("\n")
					),
				),
			},
			SessionEngineEventKind::Queued => {},
		}
	}

	fn finish(&mut self, event: &SessionEngineEvent, notice: &str) {
		self.runs.retain(|handle| Some(handle.id()) != event.run);
		if self.active.as_ref().map(RunHandle::id) != event.run {
			return;
		}
		if !self.streaming.is_empty() {
			let text = std::mem::take(&mut self.streaming);
			self.push(Role::Assistant, text);
		}
		if !notice.is_empty() {
			self.push(Role::Notice, notice);
		}
		self.active = None;
		self.stream_truncated = false;
		self.status.clear();
	}

	fn frame(&mut self, width: usize, height: usize) -> (Vec<String>, Option<(usize, usize)>) {
		if let Some(request) = &mut self.pending {
			if let Some(choice) = &request.choice {
				let mut details = harvest_tui::sanitize::wrap(
					&format!(
						"Plugin {:?} / tool call {:?}",
						choice.request.plugin, choice.request.tool_call_id
					),
					width,
				);
				details
					.extend(harvest_tui::sanitize::wrap(&format!("{:?}", choice.request.title), width));
				for (index, value) in choice.request.choices.iter().enumerate() {
					details.extend(harvest_tui::sanitize::wrap(
						&format!(
							"{} {}. {:?}",
							if index == choice.selected { "›" } else { " " },
							index + 1,
							value
						),
						width,
					));
				}
				let page = height.saturating_sub(2).max(1);
				request.offset = request.offset.min(details.len().saturating_sub(page));
				request.reviewed |= width > 0 && height >= 3 && request.offset + page >= details.len();
				let mut lines = vec![truncate_to_width(
					&format!("Plugin choice {}/{}", choice.selected + 1, choice.request.choices.len()),
					width,
				)];
				lines.extend(
					details
						.iter()
						.skip(request.offset)
						.take(page)
						.map(|line| truncate_to_width(line, width)),
				);
				lines.push(truncate_to_width(
					if request.reviewed {
						"↑/↓ choose · Enter submit · Esc dismiss · PgUp/PgDn review"
					} else {
						"Read to end · PgUp/PgDn · ↑/↓ choose · Esc dismiss"
					},
					width,
				));
				lines.truncate(height);
				return (lines, None);
			}
			// Debug formatting already escapes terminal controls and invisible
			// formatting characters. Do not shorten paths or otherwise rewrite
			// the final arguments shown for approval.
			let details = harvest_tui::sanitize::wrap(&request.display, width);
			let page = height.saturating_sub(2).max(1);
			request.offset = request.offset.min(details.len().saturating_sub(page));
			request.reviewed |= width > 0 && height >= 3 && request.offset + page >= details.len();
			let mut lines = vec![format!(
				"{YELLOW}{BOLD}{}{RESET}",
				truncate_to_width(&format!("Allow {}?", replace_tabs(&request.name)), width)
			)];
			lines.extend(details.iter().skip(request.offset).take(page).cloned());
			let choices = if request.reviewed {
				"y allow · n deny"
			} else {
				"read to end before allowing · n deny"
			};
			lines.push(truncate_to_width(
				&format!(
					"{choices} · PgUp/PgDn · rows {}–{}/{}",
					request.offset + 1,
					(request.offset + page).min(details.len()),
					details.len()
				),
				width,
			));
			lines.truncate(height);
			return (lines, None);
		}
		let mut recent = VecDeque::new();
		for entry in &self.entries {
			for line in entry.render(width) {
				if recent.len() == MAX_DISPLAY_ROWS {
					recent.pop_front();
				}
				recent.push_back(line);
			}
		}
		if !self.streaming.is_empty() {
			for line in harvest_tui::markdown::render(&self.streaming, width) {
				if recent.len() == MAX_DISPLAY_ROWS {
					recent.pop_front();
				}
				recent.push_back(line);
			}
			if self.stream_truncated {
				recent.push_back(truncate_to_width("[stream preview truncated]", width));
			}
		}
		let mut body: Vec<_> = recent.into_iter().collect();
		if self.trimmed || body.len() >= MAX_DISPLAY_ROWS {
			body.insert(
				0,
				truncate_to_width("[older display entries remain in the session log]", width),
			);
		}
		let mut footer = Vec::new();
		let status = if self.status.is_empty() {
			"Enter send · Tab complete · Ctrl-_ undo · Ctrl-Y redo · Ctrl-R history"
		} else {
			&self.status
		};
		footer.push(format!("{DIM}{}{RESET}", truncate_to_width(status, width)));
		let mut editor = self.editor.render(width);
		let mut cursor_in_editor = self.editor.cursor(width);
		// Keep the cursor's editor window visible even for a pasted multiline
		// draft or a terminal shorter than the draft.
		let suggestions = self
			.completion
			.as_ref()
			.map_or_else(Vec::new, |menu| menu.render(width, height.saturating_sub(3)));
		footer.truncate(height.saturating_sub(suggestions.len()).saturating_sub(1));
		let editor_height = height.saturating_sub(footer.len() + suggestions.len());
		if editor.len() > editor_height {
			let start = cursor_in_editor
				.map_or(0, |(row, _)| row.saturating_add(1).saturating_sub(editor_height));
			editor = editor.into_iter().skip(start).take(editor_height).collect();
			cursor_in_editor = cursor_in_editor.map(|(row, col)| (row.saturating_sub(start), col));
		}
		let reserved = footer.len() + editor.len();
		let body_height = height.saturating_sub(reserved);
		self.scroll = self.scroll.min(body.len().saturating_sub(body_height));
		let mut lines = viewport(&body, body_height, self.scroll);
		lines.resize(body_height, String::new());
		lines.extend(footer);
		let top = lines.len();
		lines.extend(editor);
		lines.extend(suggestions);
		let cursor = if self.pending.is_some() {
			None
		} else {
			cursor_in_editor.map(|(r, c)| (top + r, c))
		};
		(lines, cursor)
	}
}

fn preview_args(args: &str) -> String {
	let flat = replace_tabs(truncate_bytes(args, TOOL_ARGS_PREVIEW)).replace('\n', " ");
	let mut cut = flat.len().min(TOOL_ARGS_PREVIEW);
	while !flat.is_char_boundary(cut) {
		cut -= 1;
	}
	shorten_path(&flat[..cut])
}

#[derive(Debug, PartialEq, Eq)]
enum Input {
	Key(Key),
	Paste { text: String, enter: bool },
	Resize,
	Fatal(String),
}

/// Run the interactive UI until the user quits.
pub fn run(options: ChatOptions) -> Result<u8> {
	TerminalGuard::check()?;
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(2)
		.enable_all()
		.build()?;
	let result = runtime.block_on(run_async(options));
	runtime.shutdown_timeout(Duration::from_secs(1));
	result
}

async fn run_async(options: ChatOptions) -> Result<u8> {
	let workspace = Arc::new(JailedFs::open(&options.workspace)?);
	let history_workspace = options.workspace.clone();
	let agent_directory = options.engine.user_agent_dir.clone();
	let history_directory = agent_directory.clone();
	let history_loaded = tokio::task::spawn_blocking(move || -> Result<_> {
		let history =
			Arc::new(history::History::open(&history_workspace, history_directory.as_deref())?);
		let prompts = history
			.recent(100)?
			.into_iter()
			.rev()
			.map(|entry| entry.prompt)
			.collect::<Vec<_>>();
		Ok((history, prompts))
	})
	.await
	.context("native history worker failed")
	.and_then(|result| result);
	let (approval_tx, mut approvals) = mpsc::channel(1);
	let session = SessionEngine::open(
		&options.workspace,
		&options.session,
		options.leaf.as_deref(),
		options.model,
		Arc::new(TuiApproval { requests: approval_tx }),
		options.engine,
	)
	.await
	.context("failed to open interactive session")?;
	let mut subscription = session.subscribe().await?;

	let mut app = App {
		entries: VecDeque::new(),
		entry_bytes: 0,
		trimmed: false,
		streaming: String::new(),
		stream_truncated: false,
		editor: Editor::new("› "),
		workspace,
		agent_directory,
		session_id: subscription.snapshot.session_id.clone(),
		history: None,
		history_prompts: Vec::new(),
		completion: None,
		completion_epoch: 0,
		completion_task: None,
		pending: None,
		active: None,
		runs: Vec::new(),
		scroll: 0,
		status: String::new(),
		quit: false,
	};
	app.push(
		Role::Notice,
		format!(
			"harvest · {} · {}",
			options.model_id,
			shorten_path(&options.workspace.display().to_string())
		),
	);
	for message in &subscription.snapshot.messages {
		app.push_history(message);
	}
	match history_loaded {
		Ok((history, prompts)) => {
			app.history = Some(history);
			app.history_prompts = prompts;
		},
		Err(error) => {
			app.push(Role::Notice, format!("persistent prompt history unavailable: {error:#}"))
		},
	}
	app.restore_editor_history(&subscription.snapshot.messages);

	let guard = TerminalGuard::enter()?;
	let (input_tx, mut inputs) = mpsc::channel(INPUT_CAPACITY);
	let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
	let reader = input::spawn_reader(input_tx, Arc::clone(&stop))
		.context("failed to start terminal input reader")?;

	let mut renderer = Renderer::new();
	let mut out = io::stdout();
	let outcome: Result<()> = async { while !app.quit {
		if app.pending.as_ref().and_then(|request| request.choice.as_ref()).is_some_and(|choice| choice.cancellation.heartbeat().is_err()) {
			if let Some(request) = app.pending.take() { request.deny(); }
		}
		anyhow::ensure!(guard.is_active(), "interactive terminal was restored after a panic");
		let (cols, rows) = TerminalGuard::size();
		let (lines, cursor) = app.frame(cols as usize, rows as usize);
		renderer.draw(&mut out, &lines, cols as usize, rows as usize, cursor)?;
		tokio::select! {
			input = inputs.recv() => match input {
				None => break,
				Some(Input::Resize) => renderer.invalidate(),
				Some(Input::Fatal(message)) => anyhow::bail!(message),
				Some(Input::Paste { text, enter }) => handle_paste(&mut app, &session, text, enter).await,
				Some(Input::Key(key)) => handle_key(&mut app, &session, key).await,
			},
			event = subscription.events.recv() => match event {
				Ok(event) => app.on_event(&event),
				Err(RecvError::Lagged(_)) => {
					subscription = session.subscribe().await?;
					app.entries.clear(); app.entry_bytes = 0; app.trimmed = false;
					app.streaming.clear(); app.stream_truncated = false;
					app.active = app.runs.iter().find(|handle| Some(handle.id()) == subscription.snapshot.active).cloned();
					app.runs.retain(|handle| Some(handle.id()) == subscription.snapshot.active || subscription.snapshot.queued.contains(&handle.id()));
					for message in &subscription.snapshot.messages { app.push_history(message); }
					app.restore_editor_history(&subscription.snapshot.messages);
					app.push(Role::Notice, "display resynchronized from committed history after event lag; provisional text was discarded");
				},
				Err(RecvError::Closed) => break,
			},
			completion = async { app.completion_task.as_mut().expect("enabled completion worker").await }, if app.completion_task.is_some() => {
				app.completion_task = None;
				app.finish_completion(completion);
			},
			request = approvals.recv(), if app.pending.is_none() => {
				if let Some(request) = request {
					app.invalidate_completion();
					app.pending = Some(request);
				}
			},
		}
	} Ok(()) }.await;
	if let Some(request) = app.pending.take() {
		request.deny();
	}
	if let Some(task) = app.completion_task.take() {
		task.abort();
	}
	for handle in app.runs.drain(..) {
		handle.abort();
	}
	stop.store(true, std::sync::atomic::Ordering::Relaxed);
	drop(inputs);
	approvals.close();
	while let Ok(request) = approvals.try_recv() {
		request.deny();
	}
	let shutdown = session.shutdown().await;
	let reader_result = reader.join();
	drop(guard);
	outcome?;
	shutdown.context("interactive session shutdown failed")?;
	anyhow::ensure!(reader_result.is_ok(), "terminal input reader panicked");
	Ok(0)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::chat::FunctionCall;

	#[test]
	fn approval_exposes_the_entire_final_call_and_denial_unblocks_the_driver() {
		let (requests, mut incoming) = mpsc::channel(1);
		let policy = TuiApproval { requests };
		let call = ToolCall {
			id:       "original-call".into(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "bash".into(),
				arguments: format!("{}\u{1b}[2J; executable-suffix", "x".repeat(4000)),
			},
		};
		let driver = std::thread::spawn(move || policy.approve(&call));
		let request = incoming.blocking_recv().unwrap();
		assert!(request.display.contains("original-call"));
		assert!(request.display.contains("executable-suffix"));
		assert!(!request.display.contains('\u{1b}'));
		assert!(request.display.contains("\\u{1b}"));
		request.reply.send(false).unwrap();
		assert!(!driver.join().unwrap().unwrap());
	}
}

async fn handle_key(app: &mut App, session: &crate::session_engine::SessionHandle, key: Key) {
	if let Some(mut request) = app.pending.take() {
		if let Some(choice) = request.choice.as_mut() {
			match key {
				Key::Up => {
					choice.selected = choice.selected.saturating_sub(1);
					app.pending = Some(request);
				},
				Key::Down => {
					choice.selected = (choice.selected + 1).min(choice.request.choices.len() - 1);
					app.pending = Some(request);
				},
				Key::PageDown => {
					request.offset = request.offset.saturating_add(10);
					app.pending = Some(request);
				},
				Key::PageUp => {
					request.offset = request.offset.saturating_sub(10);
					app.pending = Some(request);
				},
				Key::Enter if request.reviewed && choice.cancellation.heartbeat().is_ok() => {
					if let Some(choice) = request.choice.take() {
						let _ = choice.reply.send(Some(choice.selected));
					}
					app.push(Role::Notice, "Plugin choice submitted.");
				},
				Key::Escape | Key::Ctrl('c') | Key::Ctrl('d') => {
					request.deny();
					if key == Key::Ctrl('d') {
						app.quit = true;
					}
					if key == Key::Ctrl('c') {
						if let Some(handle) = &app.active {
							handle.abort();
						}
					}
				},
				_ => app.pending = Some(request),
			}
			return;
		}
		match key {
			Key::Char('y' | 'Y') if request.reviewed => {
				let _ = request.reply.send(true);
				app.push(Role::Notice, format!("approved {}", request.name));
			},
			Key::Char('n' | 'N') | Key::Escape | Key::Ctrl('c') => {
				let name = request.name.clone();
				request.deny();
				app.push(Role::Notice, format!("denied {name}"));
				if key == Key::Ctrl('c') {
					if let Some(handle) = &app.active {
						handle.abort();
					}
				}
			},
			Key::Ctrl('d') => {
				request.deny();
				app.quit = true;
			},
			Key::PageDown | Key::Down => {
				request.offset = request.offset.saturating_add(10);
				app.pending = Some(request);
			},
			Key::PageUp | Key::Up => {
				request.offset = request.offset.saturating_sub(10);
				app.pending = Some(request);
			},
			_ => app.pending = Some(request),
		}
		return;
	}
	if app.handle_completion_key(&key) {
		return;
	}
	app.invalidate_completion();
	match key {
		Key::Ctrl('c') => {
			if let Some(handle) = &app.active {
				handle.abort();
				app.status = "interrupting…".into();
			} else if app.editor.text().is_empty() {
				app.quit = true;
			} else {
				app.editor.set_text("");
			}
		},
		Key::Ctrl('d') if app.editor.text().is_empty() => app.quit = true,
		Key::PageUp => app.scroll = app.scroll.saturating_add(10),
		Key::PageDown => app.scroll = app.scroll.saturating_sub(10),
		key => match app.editor.apply(&key) {
			EditorAction::Rejected => {
				app.push(Role::Error, "input exceeds the editor byte limit; draft unchanged")
			},
			EditorAction::None => {},
			EditorAction::Submit(text) => {
				if matches!(text.trim(), "/exit" | "/quit") {
					app.quit = true;
					return;
				}
				if text.trim() == "/help" {
					let commands = completion::COMMANDS
						.iter()
						.map(|(name, description)| format!("{name} — {description}"))
						.collect::<Vec<_>>()
						.join("\n");
					app.push(
						Role::Notice,
						format!(
							"{commands}\nTab completes paths/commands; arrows choose, Enter inserts. \
							 Ctrl-_ or Ctrl-Z undo; Ctrl-Y redo. Ctrl-R searches native history. \
							 Shift-Enter inserts a newline."
						),
					);
					return;
				}
				if text.trim() == "/skills" {
					app.editor.set_text("/skill:");
					app.request_completion();
					return;
				}
				if text.trim() == "/clear" {
					app.entries.clear();
					app.entry_bytes = 0;
					app.trimmed = false;
					app.scroll = 0;
					return;
				}
				if text.trim() == "/history" || text.trim().starts_with("/history ") {
					app.request_history(
						text
							.trim()
							.strip_prefix("/history")
							.unwrap_or_default()
							.trim()
							.to_owned(),
					);
					return;
				}
				if text.trim() == "/resume" {
					if let Err(error) = session.resume_follow_ups().await {
						app.push(Role::Error, error.to_string());
					}
					return;
				}
				let draft = text.clone();
				let result = if app.active.is_some() {
					session.follow_up(text).await
				} else {
					session.submit(text).await
				};
				match result {
					Ok(ticket) => {
						app.runs.push(ticket.handle.clone());
						if app.active.is_none() {
							app.active = Some(ticket.handle.clone());
						}
						app.status = "working… (ctrl+c to interrupt)".into();
						app.persist_history(draft).await;
					},
					Err(error) => {
						app.editor.set_text(&draft);
						app.push(Role::Error, error.to_string());
					},
				}
			},
		},
	}
}

async fn handle_paste(
	app: &mut App,
	session: &crate::session_engine::SessionHandle,
	text: String,
	enter: bool,
) {
	// Paste and its same-read Enter are one input transaction. No approval or
	// session event can move focus between insertion and submission.
	if app.pending.is_some() {
		return;
	}
	app.invalidate_completion();
	match app.editor.apply(&Key::Paste(text)) {
		EditorAction::Rejected => app.push(
			Role::Error,
			"paste exceeds the editor byte limit; draft unchanged and Enter ignored",
		),
		EditorAction::None if enter => handle_key(app, session, Key::Enter).await,
		_ => {},
	}
}
