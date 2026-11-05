//! Memory tools (`retain`, `recall`, `reflect`, `learn`, `memory_edit`) over
//! a pluggable [`MemoryBackend`]. The engine itself (Mnemopi/Hindsight) is
//! provided by another crate and installed into the session; this module owns
//! backend selection via the `memory.backend` setting and the tool contracts.

use std::{
	path::Path,
	sync::{Arc, LazyLock},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod mnemopi;
pub mod model;

/// One method per memory operation. Requests mirror the tool arguments plus
/// coding-agent metadata (`source`, `importance`, `cwd`, `tool`); responses
/// are backend-shaped JSON documented on each method.
pub trait MemoryBackend: Send + Sync + std::fmt::Debug {
	/// Identity is host-owned and must match the selected settings backend.
	fn id(&self) -> &str;
	/// Install only a host-selected provider, never a model-supplied endpoint.
	fn configure_llm(&self, _llm: Option<Arc<dyn harvest_mnemopi::providers::Llm>>) -> Result<()> {
		bail!("this memory backend does not support a native host LLM")
	}
	/// `{items:[{content,context?}], source, importance, cwd}` ->
	/// `{count, queued?: bool}` (queued = accepted for async retention).
	fn retain(&self, request: Value) -> Result<Value>;
	/// `{query}` -> `{count, text}` (`text` = formatted results with ids).
	fn recall(&self, request: Value) -> Result<Value>;
	/// `{query, context?}` -> `{text}` (empty text = nothing to reflect on).
	fn reflect(&self, request: Value) -> Result<Value>;
	/// `{memory, context?, source, importance, cwd}` -> `{stored: bool, queued?:
	/// bool}`.
	fn learn(&self, request: Value) -> Result<Value>;
	/// `{op, id, content?, importance?, replacementId?}` ->
	/// `{status, bank?, store?}`.
	fn edit(&self, request: Value) -> Result<Value>;
	/// Full stored content behind `memory://ID`, scoped by this backend.
	fn read(&self, _id: &str) -> Result<Option<Value>> {
		bail!("this memory backend does not support memory:// reads")
	}
	/// Native cursors refer only to acknowledged, durably retained prefixes.
	fn retention_cursors(&self, _session_id: &str) -> Result<Vec<RetentionCursor>> {
		bail!("this memory backend does not support automatic transcript retention")
	}
	fn retain_episode(
		&self,
		_episode: &RetentionEpisode,
		_cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<()> {
		bail!("this memory backend does not support automatic transcript retention")
	}
}

/// A prefix digest prevents an old branch's cursor from skipping new content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetentionCursor {
	pub messages:   usize,
	pub user_turns: usize,
	pub digest:     String,
}

pub struct RetentionEpisode {
	pub session_id:   String,
	pub cwd:          String,
	pub cursor:       RetentionCursor,
	pub transcript:   String,
	pub embed_text:   String,
	pub extract_text: Option<String>,
	pub outcome:      &'static str,
}

/// One lifecycle belongs to the existing ToolRuntime, not a global registry.
#[derive(Default)]
pub struct Lifecycle {
	recalled: bool,
	snippet:  Option<String>,
	cursor:   Option<RetentionCursor>,
}

const MAX_RETENTION_BYTES: usize = 256 * 1024;
const MAX_RETENTION_CHUNKS: usize = 8;

fn strip_memory_blocks(text: &str) -> String {
	static BLOCKS: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
		["memories", "mental_models", "hindsight_memories", "relevant_memories"]
			.iter()
			.map(|tag| {
				regex::Regex::new(&format!("(?s)<{tag}>.*?</{tag}>")).expect("memory block regex")
			})
			.collect()
	});
	let mut text = text.to_owned();
	for block in BLOCKS.iter() {
		text = block.replace_all(&text, "").into_owned();
	}
	text
}

fn without_protocol(text: &str) -> String {
	static MARKER: LazyLock<regex::Regex> = LazyLock::new(|| {
		regex::Regex::new(r"^\[(?:role:\s*[-_a-zA-Z0-9]+|[-_a-zA-Z0-9]+:end|timestamp:\s+.+)\]$")
			.expect("retention marker regex")
	});
	strip_memory_blocks(text)
		.lines()
		.filter(|line| !MARKER.is_match(line.trim()))
		.collect::<Vec<_>>()
		.join("\n")
		.trim()
		.to_owned()
}

fn conversation_text(message: &crate::chat::ChatMessage) -> String {
	let text = message.content.as_deref().unwrap_or_default();
	let original = (message.role == "user")
		.then(|| crate::managed::invocation::original_prompt(text))
		.flatten();
	without_protocol(original.as_deref().unwrap_or(text))
}

fn update_prefix(digest: &mut harvest_runtime::ContentDigest, message: &crate::chat::ChatMessage) {
	digest.update(&(message.role.len() as u64).to_le_bytes());
	digest.update(message.role.as_bytes());
	let content = if matches!(message.role.as_str(), "user" | "assistant") {
		message.content.as_deref().unwrap_or_default()
	} else {
		""
	};
	digest.update(&(content.len() as u64).to_le_bytes());
	digest.update(content.as_bytes());
}

fn prefix_matches(messages: &[crate::chat::ChatMessage], cursor: &RetentionCursor) -> bool {
	if cursor.messages > messages.len() {
		return false;
	}
	let mut digest = harvest_runtime::ContentDigest::default();
	for message in &messages[..cursor.messages] {
		update_prefix(&mut digest, message);
	}
	digest.hash() == cursor.digest
		&& messages[..cursor.messages]
			.iter()
			.filter(|message| message.role == "user")
			.count()
			== cursor.user_turns
}

fn matching_cursor(
	messages: &[crate::chat::ChatMessage],
	cursors: Vec<RetentionCursor>,
) -> Option<RetentionCursor> {
	let mut candidates = std::collections::BTreeMap::<usize, Vec<RetentionCursor>>::new();
	for cursor in cursors.into_iter().take(1024) {
		if cursor.messages > 0 && cursor.messages <= messages.len() {
			candidates.entry(cursor.messages).or_default().push(cursor);
		}
	}
	let mut digest = harvest_runtime::ContentDigest::default();
	let mut user_turns = 0;
	let mut matched = None;
	for (index, message) in messages.iter().enumerate() {
		update_prefix(&mut digest, message);
		user_turns += usize::from(message.role == "user");
		if let Some(candidates) = candidates.get(&(index + 1)) {
			let hash = digest.hash();
			if let Some(cursor) = candidates
				.iter()
				.find(|cursor| cursor.digest == hash && cursor.user_turns == user_turns)
			{
				matched = Some(cursor.clone());
			}
		}
	}
	matched
}

impl Lifecycle {
	fn enabled<'a>(
		settings: &crate::settings::Settings,
		backend: Option<&'a SharedMemoryBackend>,
	) -> Option<&'a SharedMemoryBackend> {
		(settings.string("memory.backend").as_deref() == Some("mnemopi"))
			.then_some(backend)
			.flatten()
			.filter(|backend| backend.id() == "mnemopi")
	}

	/// Recall once per opened host; this context is never appended to the log.
	pub fn before_turn(
		&mut self,
		settings: &crate::settings::Settings,
		backend: Option<&SharedMemoryBackend>,
		cwd: &Path,
		prompt: &str,
		history: &[crate::chat::ChatMessage],
		cancellation: &harvest_runtime::cancel::CancelToken,
		directory: Option<&Path>,
	) -> Result<Option<String>> {
		let Some(backend) = Self::enabled(settings, backend) else {
			return Ok(None);
		};
		if !settings.bool("mnemopi.autoRecall") {
			return Ok(None);
		}
		if self.recalled {
			return Ok(self.snippet.clone());
		}
		cancellation.heartbeat()?;
		let agent_dir =
			crate::config::agent_dir_from(directory.map(Path::to_owned), |key| std::env::var_os(key))
				.unwrap_or_else(|| cwd.to_owned());
		let redactor = crate::commands::secrets::SecretObfuscator::build(cwd, &agent_dir);
		let limit = (settings
			.number("mnemopi.recallMaxQueryChars")
			.unwrap_or(4000.0)
			.clamp(256.0, 65536.0) as usize)
			* 4;
		let latest = redactor.obfuscate(&without_protocol(prompt));
		let latest = harvest_runtime::truncate_bytes(latest.trim(), limit);
		if !latest.chars().any(char::is_alphanumeric) {
			return Ok(None);
		}
		let turns = settings
			.number("mnemopi.recallContextTurns")
			.unwrap_or(3.0)
			.clamp(1.0, 32.0) as usize;
		let mut lines = Vec::new();
		let mut bytes = latest.len();
		let mut users = 0;
		if turns > 1 {
			for message in history.iter().rev() {
				if !matches!(message.role.as_str(), "user" | "assistant") {
					continue;
				}
				if message.role == "user" {
					users += 1;
					if users >= turns {
						break;
					}
				}
				let content = redactor.obfuscate(&conversation_text(message));
				let content = content.trim();
				if content.is_empty() {
					continue;
				}
				let line = format!("{}: {content}", message.role);
				if bytes + line.len() + 32 > limit {
					break;
				}
				bytes += line.len() + 1;
				lines.push(line);
			}
		}
		lines.reverse();
		let query = if lines.is_empty() {
			latest.to_owned()
		} else {
			format!("Prior context:\n\n{}\n\n{latest}", lines.join("\n"))
		};
		let response = backend.recall(json!({"query":query}))?;
		cancellation.heartbeat()?;
		if response.get("count").and_then(Value::as_u64).unwrap_or(0) == 0 {
			self.recalled = true;
			return Ok(None);
		}
		let text = without_protocol(
			response
				.get("text")
				.and_then(Value::as_str)
				.unwrap_or_default(),
		);
		let text = redactor.obfuscate(&text);
		let budget = (settings
			.number("mnemopi.injectionTokenLimit")
			.unwrap_or(5000.0)
			.clamp(256.0, 65536.0) as usize)
			* 4;
		let context = harvest_runtime::truncate_bytes(&text, budget.saturating_sub(512));
		let rendered = handlebars::Handlebars::new().render_template(
			include_str!("prompts/memory-context.md"),
			&json!({"context":context,"current_time":now_utc()}),
		)?;
		self.snippet = Some(harvest_runtime::truncate_bytes(&rendered, budget).to_owned());
		self.recalled = true;
		Ok(self.snippet.clone())
	}

	/// Retain committed text only. Failed/cancelled provisional deltas never
	/// enter history.
	pub fn after_turn(
		&mut self,
		settings: &crate::settings::Settings,
		backend: Option<&SharedMemoryBackend>,
		cwd: &Path,
		session_id: &str,
		messages: &[crate::chat::ChatMessage],
		outcome: &'static str,
		force: bool,
		directory: Option<&Path>,
	) -> Result<()> {
		let Some(backend) = Self::enabled(settings, backend) else {
			return Ok(());
		};
		if !settings.bool("mnemopi.autoRetain") || messages.is_empty() {
			return Ok(());
		}
		if !self
			.cursor
			.as_ref()
			.is_some_and(|cursor| prefix_matches(messages, cursor))
		{
			self.cursor = matching_cursor(messages, backend.retention_cursors(session_id)?);
		}
		let user_turns = messages
			.iter()
			.filter(|message| message.role == "user")
			.count();
		let cadence = settings
			.number("mnemopi.retainEveryNTurns")
			.unwrap_or(4.0)
			.clamp(1.0, 10000.0) as usize;
		if !force
			&& user_turns.saturating_sub(self.cursor.as_ref().map_or(0, |cursor| cursor.user_turns))
				< cadence
		{
			return Ok(());
		}
		let agent_dir =
			crate::config::agent_dir_from(directory.map(Path::to_owned), |key| std::env::var_os(key))
				.unwrap_or_else(|| cwd.to_owned());
		let redactor = crate::commands::secrets::SecretObfuscator::build(cwd, &agent_dir);
		let cancellation = harvest_runtime::cancel::CancelToken::new(Some(5000));
		for _ in 0..MAX_RETENTION_CHUNKS {
			cancellation.heartbeat()?;
			let start = self.cursor.as_ref().map_or(0, |cursor| cursor.messages);
			if start >= messages.len() {
				return Ok(());
			}
			let mut digest = harvest_runtime::ContentDigest::default();
			let mut through_users = 0;
			for message in &messages[..start] {
				update_prefix(&mut digest, message);
				through_users += usize::from(message.role == "user");
			}
			let mut parts = Vec::new();
			let mut plain = Vec::new();
			let mut user = Vec::new();
			let mut bytes = 0;
			let mut end = start;
			for message in &messages[start..] {
				let retainable = matches!(message.role.as_str(), "user" | "assistant");
				let content = if retainable {
					redactor.obfuscate(&conversation_text(message))
				} else {
					String::new()
				};
				let substantive = content.chars().any(char::is_alphanumeric);
				if substantive && content.len() + 128 > MAX_RETENTION_BYTES {
					bail!("one durable message exceeds automatic memory retention budget");
				}
				if substantive && bytes + content.len() + 128 > MAX_RETENTION_BYTES {
					break;
				}
				update_prefix(&mut digest, message);
				through_users += usize::from(message.role == "user");
				end += 1;
				if substantive {
					let framed = format!("[role: {}]\n{content}\n[{}:end]", message.role, message.role);
					bytes += framed.len() + 2;
					if message.role == "user" {
						user.push(framed.clone());
					}
					plain.push(content);
					parts.push(framed);
				}
			}
			let cursor = RetentionCursor {
				messages:   end,
				user_turns: through_users,
				digest:     digest.hash(),
			};
			if parts.is_empty() {
				return Ok(());
			}
			let episode = RetentionEpisode {
				session_id: session_id.into(),
				cwd: cwd.display().to_string(),
				cursor: cursor.clone(),
				transcript: parts.join("\n\n"),
				embed_text: plain.join("\n\n"),
				extract_text: (!user.is_empty()).then(|| user.join("\n\n")),
				outcome,
			};
			backend.retain_episode(&episode, &cancellation)?;
			self.cursor = Some(cursor);
		}
		if self
			.cursor
			.as_ref()
			.is_some_and(|cursor| cursor.messages < messages.len())
		{
			bail!("automatic memory backlog exceeds per-run retention budget");
		}
		Ok(())
	}
}

/// The engine slot a host fills; `None` behaves like `memory.backend = "off"`.
pub type SharedMemoryBackend = Arc<dyn MemoryBackend>;

/// A child borrows the parent's selected store and settings; it does not create
/// another engine or retain a duplicate automatic transcript.
#[derive(Clone)]
pub struct Inheritance {
	pub agent_directory: Option<std::path::PathBuf>,
	pub backend:         Option<SharedMemoryBackend>,
	pub settings:        crate::settings::Settings,
}

pub(crate) fn configuration_key(
	settings: &crate::settings::Settings,
	agent_dir: Option<&Path>,
) -> Result<String> {
	let value = json!({"backend":settings.get("memory.backend"),"mnemopi":settings.get("mnemopi"),"autolearn":settings.get("autolearn.enabled"),"memory_model":settings.get("providers.memoryModel"),"agent_dir":agent_dir});
	let mut writer = harvest_runtime::limits::BoundedWriter::new(2 * 1024 * 1024);
	serde_json::to_writer(&mut writer, &value)?;
	Ok(harvest_runtime::content_hash(&writer.into_inner()))
}

pub(crate) fn is_backend_tool(name: &str) -> bool {
	matches!(name, "retain" | "recall" | "reflect" | "memory_edit" | "learn")
}

/// Message TS reports for the no-op backend selected by `memory.backend =
/// "off"`.
pub const MEMORY_OFF: &str = "Memory backend is off.";

pub fn read(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	id: &str,
) -> Result<crate::tools::ToolOutput> {
	if id.is_empty()
		|| id.len() > 256
		|| !id
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
	{
		bail!("invalid memory id");
	}
	let (_, backend) = resolve(settings, backend, &["mnemopi"])?;
	let row = backend
		.read(id)?
		.ok_or_else(|| anyhow::anyhow!("Memory {id} was not found."))?;
	let content = row
		.get("content")
		.and_then(Value::as_str)
		.ok_or_else(|| anyhow::anyhow!("stored memory has no content"))?
		.to_owned();
	if content.len() > harvest_runtime::MAX_TOOL_BYTES {
		bail!("stored memory exceeds the tool-result byte limit");
	}
	Ok(crate::tools::ToolOutput {
		content,
		details: Some(json!({"id":id,"bank":row["bank"],"store":row["memory_store"]})),
		is_error: false,
	})
}

/// Match the legacy `createIf` gates for model schemas and direct invocations.
pub fn tool_enabled(settings: &crate::settings::Settings, name: &str) -> bool {
	let backend = settings
		.string("memory.backend")
		.unwrap_or_else(|| "off".into());
	match name {
		"retain" | "recall" | "reflect" => matches!(backend.as_str(), "hindsight" | "mnemopi"),
		"memory_edit" => backend == "mnemopi",
		"learn" => {
			settings.bool("autolearn.enabled")
				&& matches!(backend.as_str(), "hindsight" | "mnemopi" | "local")
		},
		"manage_skill" => settings.bool("autolearn.enabled"),
		_ => true,
	}
}

fn label(id: &str) -> &str {
	match id {
		"mnemopi" => "Mnemopi",
		"hindsight" => "Hindsight",
		"sharpshooter" => "Sharpshooter",
		other => other,
	}
}

/// Resolve the backend for a tool, mirroring `resolveMemoryBackend` plus the
/// per-tool `createIf` gates.
fn resolve<'a>(
	settings: &crate::settings::Settings,
	installed: Option<&'a SharedMemoryBackend>,
	allowed: &[&str],
) -> Result<(String, &'a SharedMemoryBackend)> {
	let id = settings
		.string("memory.backend")
		.unwrap_or_else(|| "off".into());
	if !allowed.contains(&id.as_str()) {
		bail!("{MEMORY_OFF}");
	}
	match installed {
		Some(backend) if backend.id() == id => Ok((id, backend)),
		Some(_) => bail!("installed memory backend does not match the selected backend"),
		None => bail!("{} backend is not initialised for this session.", label(&id)),
	}
}

fn noun(count: u64) -> &'static str {
	if count == 1 { "memory" } else { "memories" }
}

fn now_utc() -> String {
	chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string()
}

#[derive(Deserialize)]
struct RetainArgs {
	items: Vec<Value>,
}

pub fn retain(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	cwd: &Path,
	arguments: &str,
) -> Result<String> {
	let args: RetainArgs = serde_json::from_str(arguments)?;
	if args.items.is_empty() {
		bail!("items must contain at least one memory");
	}
	let (_, backend) = resolve(settings, backend, &["hindsight", "mnemopi"])?;
	let count = args.items.len() as u64;
	let response = backend.retain(json!({
		"items": args.items,
		"source": "coding-agent-retain",
		"importance": 0.75,
		"cwd": cwd.display().to_string(),
		"tool": "retain",
	}))?;
	if response.get("count").and_then(Value::as_u64) != Some(count) {
		bail!("The memory backend did not accept every retained item.");
	}
	let verb = if response.get("queued").and_then(Value::as_bool) == Some(true) {
		"queued"
	} else {
		"stored"
	};
	Ok(format!("{count} {} {verb}.", noun(count)))
}

#[derive(Deserialize)]
struct QueryArgs {
	query:   String,
	context: Option<String>,
}

pub fn recall(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	arguments: &str,
) -> Result<String> {
	let args: QueryArgs = serde_json::from_str(arguments)?;
	let (_, backend) = resolve(settings, backend, &["hindsight", "mnemopi"])?;
	let response = backend.recall(json!({ "query": args.query }))?;
	let count = response.get("count").and_then(Value::as_u64).unwrap_or(0);
	if count == 0 {
		return Ok("No relevant memories found.".into());
	}
	let text = response
		.get("text")
		.and_then(Value::as_str)
		.unwrap_or_default();
	Ok(format!("Found {count} relevant {} (as of {} UTC):\n\n{text}", noun(count), now_utc()))
}

pub fn reflect(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	arguments: &str,
) -> Result<String> {
	let args: QueryArgs = serde_json::from_str(arguments)?;
	let (id, backend) = resolve(settings, backend, &["hindsight", "mnemopi"])?;
	let response = backend.reflect(json!({ "query": args.query, "context": args.context }))?;
	let text = response
		.get("text")
		.and_then(Value::as_str)
		.unwrap_or_default()
		.trim();
	if text.is_empty() {
		return Ok("No relevant information found to reflect on.".into());
	}
	Ok(if id == "mnemopi" {
		format!("Based on recalled memories:\n\n{text}")
	} else {
		text.to_owned()
	})
}

#[derive(Deserialize)]
struct EditArgs {
	op:             String,
	id:             String,
	content:        Option<String>,
	importance:     Option<f64>,
	replacement_id: Option<String>,
}

pub fn memory_edit(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	arguments: &str,
) -> Result<String> {
	let args: EditArgs = serde_json::from_str(arguments)?;
	if !matches!(args.op.as_str(), "update" | "forget" | "invalidate") {
		bail!("memory_edit op must be update, forget, or invalidate.");
	}
	let (_, backend) = resolve(settings, backend, &["mnemopi"])?;
	if args.op == "update" && args.content.is_none() && args.importance.is_none() {
		bail!("memory_edit update requires content or importance.");
	}
	let importance = args.importance.map(|value| value.clamp(0.0, 1.0));
	let result = backend.edit(json!({
		"op": args.op,
		"id": args.id,
		"content": args.content,
		"importance": importance,
		"replacementId": args.replacement_id,
	}))?;
	let status = result
		.get("status")
		.and_then(Value::as_str)
		.unwrap_or("updated");
	let location = match result.get("bank").and_then(Value::as_str) {
		Some(bank) => match result.get("store").and_then(Value::as_str) {
			Some(store) => format!(" in bank {bank} ({store})"),
			None => format!(" in bank {bank}"),
		},
		None => String::new(),
	};
	let id = &args.id;
	Ok(match status {
		"not_found" => format!("Memory {id} was not found{location}."),
		"not_editable" => format!(
			"Memory {id} is a read-only fact{location}; it cannot be edited. Read it with \
			 memory://{id}."
		),
		other => format!("Memory {id} {other}{location}."),
	})
}

#[derive(Deserialize)]
struct LearnSkill {
	action:          String,
	name:            String,
	description:     Option<String>,
	body:            Option<String>,
	#[serde(rename = "expectedActive")]
	expected_active: Option<String>,
}

#[derive(Deserialize)]
struct LearnArgs {
	memory:  String,
	context: Option<String>,
	skill:   Option<LearnSkill>,
}

/// `learn` writes (approval) when it carries a skill or the backend is local.
pub fn learn_requires_approval(settings: &crate::settings::Settings, arguments: &str) -> bool {
	serde_json::from_str::<Value>(arguments)
		.ok()
		.is_none_or(|value| value.get("skill").is_some_and(|skill| !skill.is_null()))
		|| settings.string("memory.backend").as_deref() == Some("local")
}

pub fn learn(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	cwd: &Path,
	arguments: &str,
) -> Result<String> {
	learn_in(settings, backend, cwd, None, arguments)
}

pub fn learn_in(
	settings: &crate::settings::Settings,
	backend: Option<&SharedMemoryBackend>,
	cwd: &Path,
	agent_directory: Option<&Path>,
	arguments: &str,
) -> Result<String> {
	let args: LearnArgs = serde_json::from_str(arguments)?;
	if args
		.skill
		.as_ref()
		.is_some_and(|skill| !matches!(skill.action.as_str(), "create" | "update"))
	{
		bail!("learn skill action must be create or update");
	}
	let (_, backend) = resolve(settings, backend, &["hindsight", "mnemopi", "local"])?;
	let response = backend.learn(json!({
		"memory": args.memory,
		"context": args.context,
		"source": "coding-agent-learn",
		"importance": 0.8,
		"cwd": cwd.display().to_string(),
		"tool": "learn",
	}))?;
	let queued = response.get("queued").and_then(Value::as_bool) == Some(true);
	if !queued && response.get("stored").and_then(Value::as_bool) != Some(true) {
		bail!("The memory backend did not store the lesson.");
	}
	let memory_message = if queued {
		"Lesson queued for retention"
	} else {
		"Lesson stored"
	};
	let Some(skill) = args.skill else {
		return Ok(format!("{memory_message}."));
	};
	let store = match agent_directory {
		Some(directory) => crate::managed::Store::open_in(directory)?,
		None => crate::managed::Store::open()?,
	};
	let safe = crate::managed::sanitize_skill_name(&skill.name).ok();
	if skill.action == "create"
		&& safe.as_deref().is_some_and(|name| {
			crate::managed::is_name_claimed_by_authored_skill(cwd, store.root(), name)
		}) {
		bail!(
			"{memory_message}. Did not create managed skill \"{}\": an authored skill of that name \
			 already exists, and managed skills cannot override authored ones. Choose a different \
			 name.",
			skill.name
		);
	}
	if skill.action == "create" && (skill.description.is_none() || skill.body.is_none()) {
		bail!(
			"{memory_message}, but the managed skill could not be written: \"create\" requires both \
			 \"description\" and \"body\"."
		);
	}
	if skill.action == "update" && skill.description.is_none() && skill.body.is_none() {
		bail!(
			"{memory_message}, but the managed skill could not be written: \"update\" requires \
			 \"description\" and/or \"body\" (omitted fields merge from current)."
		);
	}
	let verb = if skill.action == "create" {
		"Created"
	} else {
		"Updated"
	};
	crate::managed::write_managed_skill(&store, &crate::managed::SkillWrite {
		action:          skill.action.clone(),
		name:            skill.name.clone(),
		description:     skill.description,
		body:            skill.body,
		expected_active: skill.expected_active,
	})
	.map_err(|error| {
		anyhow::anyhow!("{memory_message}, but the managed skill could not be written: {error}")
	})?;
	Ok(format!("{memory_message}. {verb} managed skill \"{}\".", skill.name))
}

#[cfg(test)]
mod lifecycle_tests {
	use std::{
		collections::VecDeque,
		path::{Path, PathBuf},
		sync::{Arc, Mutex},
	};

	use anyhow::Result;
	use harvest_runtime::cancel::CancelToken;

	use crate::{
		agent::RunOptions,
		agent_session::{AgentSession, EventKind},
		chat::{ChatMessage, ToolSpec},
		provider::Model,
		session_engine::{SessionEngine, SessionEngineOptions},
		tools::DenyMutations,
	};

	struct Script {
		answers:   Mutex<VecDeque<&'static str>>,
		histories: Mutex<Vec<Vec<ChatMessage>>>,
	}
	impl Script {
		fn new(answers: impl IntoIterator<Item = &'static str>) -> Self {
			Self {
				answers:   Mutex::new(answers.into_iter().collect()),
				histories: Mutex::new(Vec::new()),
			}
		}
	}
	impl Model for Script {
		fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ChatMessage> {
			anyhow::bail!("fixture requires streaming")
		}

		fn stream(
			&self,
			messages: &[ChatMessage],
			_: &[ToolSpec],
			_: &CancelToken,
			sink: &mut dyn FnMut(&str) -> Result<()>,
		) -> Result<ChatMessage> {
			self.histories.lock().unwrap().push(messages.to_vec());
			let answer = self.answers.lock().unwrap().pop_front().unwrap();
			sink(answer)?;
			Ok(ChatMessage {
				role:         "assistant".into(),
				content:      Some(answer.into()),
				tool_calls:   None,
				tool_call_id: None,
			})
		}
	}

	fn options() -> RunOptions {
		RunOptions {
			max_steps:      4,
			user_agent_dir: None,
			tags:           Default::default(),
			cancellation:   Default::default(),
		}
	}

	fn configure(root: &Path, cadence: usize) -> PathBuf {
		let db = root.join("memory.db");
		std::fs::create_dir(root.join(".harvest")).unwrap();
		std::fs::write(root.join(".harvest/settings.json"),serde_json::json!({"memory":{"backend":"mnemopi"},"mnemopi":{"dbPath":db,"scoping":"global","autoRecall":true,"autoRetain":true,"retainEveryNTurns":cadence}}).to_string()).unwrap();
		db
	}

	fn episodes(db: &Path) -> Vec<String> {
		let connection = rusqlite::Connection::open(db).unwrap();
		let mut statement = connection
			.prepare(
				"SELECT content FROM working_memory WHERE source='coding-agent-transcript' ORDER BY \
				 rowid",
			)
			.unwrap();
		statement
			.query_map([], |row| row.get(0))
			.unwrap()
			.collect::<rusqlite::Result<_>>()
			.unwrap()
	}

	#[test]
	fn sync_reopen_recalls_acknowledged_turns_and_does_not_retain_cancelled_deltas() {
		let root = tempfile::tempdir().unwrap();
		let db = configure(root.path(), 1);
		let model = Script::new([
			"Recorded the project choice.",
			"PROVISIONAL ASSISTANT TEXT MUST NOT BE RETAINED",
		]);
		let mut session = AgentSession::open(root.path(), Path::new("session.jsonl"), None).unwrap();
		session
			.submit(
				"Project Raven uses indigo build files.",
				&model,
				&DenyMutations,
				&options(),
				&mut |_| Ok(()),
			)
			.unwrap();
		drop(session);
		assert_eq!(episodes(&db).len(), 1);
		let mut session = AgentSession::open(root.path(), Path::new("session.jsonl"), None).unwrap();
		let handle = session.begin(CancelToken::default()).unwrap();
		let result = session.execute(
			&handle,
			"Which build files does Project Raven use?",
			&model,
			&DenyMutations,
			&options(),
			&mut |event| {
				if matches!(event.event, EventKind::TextDelta { .. }) {
					handle.abort();
				}
				Ok(())
			},
		);
		assert!(result.is_err());
		let histories = model.histories.lock().unwrap();
		assert!(histories[1].iter().any(|message| message.role == "system"
			&& message.content.as_deref().is_some_and(
				|text| text.contains("<memories>") && text.contains("indigo build files")
			)));
		drop(histories);
		session.close(&mut |_| Ok(())).unwrap();
		let retained = episodes(&db);
		assert_eq!(retained.len(), 2, "reopen and close do not duplicate an acknowledged prefix");
		assert!(retained[1].contains("Which build files"));
		assert!(
			!retained
				.iter()
				.any(|text| text.contains("PROVISIONAL ASSISTANT"))
		);
		let log = std::fs::read_to_string(root.path().join("session.jsonl")).unwrap();
		assert!(!log.contains("<memories>"), "recalled context is transient provider input");
	}

	#[tokio::test]
	async fn async_shutdown_flushes_below_cadence_and_disabled_lifecycle_stays_empty() {
		let root = tempfile::tempdir().unwrap();
		let db = configure(root.path(), 4);
		let model = Arc::new(Script::new(["A committed answer."]));
		let session = SessionEngine::open(
			root.path(),
			Path::new("async.jsonl"),
			None,
			model,
			Arc::new(DenyMutations),
			SessionEngineOptions::default(),
		)
		.await
		.unwrap();
		session
			.submit("Keep this durable project decision.")
			.await
			.unwrap()
			.wait()
			.await
			.unwrap();
		assert!(episodes(&db).is_empty(), "cadence defers automatic retention");
		session.shutdown().await.unwrap();
		assert_eq!(episodes(&db).len(), 1, "shutdown flushes the committed suffix");
		let mut isolated =
			AgentSession::open(root.path(), Path::new("evaluation.jsonl"), None).unwrap();
		isolated.configure_memory_lifecycle(false).unwrap();
		isolated
			.submit(
				"Evaluation-only secret fixture.",
				&Script::new(["Evaluation answer."]),
				&DenyMutations,
				&options(),
				&mut |_| Ok(()),
			)
			.unwrap();
		isolated.close(&mut |_| Ok(())).unwrap();
		assert_eq!(
			episodes(&db).len(),
			1,
			"host opt-out prevents evaluation retention, including close"
		);
	}

	#[test]
	fn retention_failure_warns_without_losing_answer_and_retries_the_unacknowledged_prefix() {
		let root = tempfile::tempdir().unwrap();
		let db = configure(root.path(), 1);
		let mut session = AgentSession::open(root.path(), Path::new("retry.jsonl"), None).unwrap();
		rusqlite::Connection::open(&db)
			.unwrap()
			.execute("DROP TABLE annotations", [])
			.unwrap();
		let model = Script::new(["First completed answer.", "Second completed answer."]);
		let mut warned = false;
		let answer = session
			.submit("First durable request.", &model, &DenyMutations, &options(), &mut |event| {
				if let EventKind::MemoryWarning { operation } = event.event {
					warned |= operation == "retention";
				}
				Ok(())
			})
			.unwrap();
		assert_eq!(answer, "First completed answer.");
		assert!(warned);
		assert!(episodes(&db).is_empty());
		let repair = harvest_mnemopi::beam::BeamMemory::open_path("default", &db).unwrap();
		drop(repair);
		session
			.submit("Second durable request.", &model, &DenyMutations, &options(), &mut |_| Ok(()))
			.unwrap();
		session.close(&mut |_| Ok(())).unwrap();
		let retained = episodes(&db).join("\n");
		assert!(
			retained.contains("First durable request.")
				&& retained.contains("Second durable request.")
		);
	}
}
