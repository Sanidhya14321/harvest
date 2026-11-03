//! Memory tools (`retain`, `recall`, `reflect`, `learn`, `memory_edit`) over
//! a pluggable [`MemoryBackend`]. The engine itself (Mnemopi/Hindsight) is
//! provided by another crate and installed into the session; this module owns
//! backend selection via the `memory.backend` setting and the tool contracts.

use std::{path::Path, sync::Arc};

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

pub mod mnemopi;

/// One method per memory operation. Requests mirror the tool arguments plus
/// coding-agent metadata (`source`, `importance`, `cwd`, `tool`); responses
/// are backend-shaped JSON documented on each method.
pub trait MemoryBackend: Send + Sync + std::fmt::Debug {
	/// Identity is host-owned and must match the selected settings backend.
	fn id(&self) -> &str;
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
}

/// The engine slot a host fills; `None` behaves like `memory.backend = "off"`.
pub type SharedMemoryBackend = Arc<dyn MemoryBackend>;

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
	let store = crate::managed::Store::open()?;
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
