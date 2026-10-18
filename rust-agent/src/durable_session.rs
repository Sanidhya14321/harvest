//! Single-writer session-v3 persistence and conservative text/function replay.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use harvest_runtime::{ContentDigest, JailedFs, RootLock, content_hash};
use serde_json::{Value, json};

use crate::{
	chat::{ChatMessage, FunctionCall, ToolCall},
	session_format::{SESSION_LIMIT, SessionDocument},
};

/// Mutation and snapshot commands share the same cooperating-writer lease.
pub(crate) fn lock_session(fs: &JailedFs, path: &Path) -> Result<RootLock> {
	let mut lock = fs.relative(path)?.into_os_string();
	lock.push(".rust-owner.lock");
	fs.lock(Path::new(&lock))
}

pub struct DurableSession {
	fs:         JailedFs,
	path:       PathBuf,
	_ownership: RootLock,
	length:     usize,
	digest:     ContentDigest,
	id:         String,
	leaf:       Option<String>,
	messages:   Vec<ChatMessage>,
}

impl DurableSession {
	/// Opens a current v3 log or creates one. Imports must use a separate file.
	pub fn open(root: &Path, path: &Path, leaf: Option<&str>) -> Result<Self> {
		let fs = JailedFs::open(root)?;
		let path = fs.relative(path)?;
		let ownership = lock_session(&fs, &path)?;
		let mut bytes = fs.read_optional(&path, SESSION_LIMIT)?.unwrap_or_default();
		if bytes.is_empty() {
			if leaf.is_some() {
				bail!("cannot select a branch of an empty session");
			}
			let timestamp = chrono::Utc::now().to_rfc3339();
			let id = content_hash(
				format!("{}:{timestamp}:{}", root.display(), std::process::id()).as_bytes(),
			);
			let header =
				json!({"type":"session", "version":3, "id":id, "timestamp":timestamp, "cwd":fs.root()});
			let mut line = serde_json::to_vec(&header)?;
			line.push(b'\n');
			fs.append_checked(&path, &line, SESSION_LIMIT, Some(&content_hash(&bytes)))?;
			bytes = line;
		}
		let document = SessionDocument::decode(&bytes)?;
		if document.migrated() {
			bail!("legacy sessions require a separate v3 import before writing");
		}
		if document.malformed_records() != 0 || !bytes.ends_with(b"\n") {
			bail!("damaged or unterminated session requires explicit recovery before writing");
		}
		document.validate_tree()?;
		let id: String = serde_json::from_str(document.header()["id"].get())?;
		let leaf = leaf.map(str::to_owned).or_else(|| {
			document
				.entries()
				.last()
				.and_then(|entry| entry.get("id"))
				.and_then(|raw| serde_json::from_str(raw.get()).ok())
		});
		let messages = match &leaf {
			Some(leaf) => crate::session_context::rebuild(&document.branch(leaf)?)?,
			None => Vec::new(),
		};
		validate_pairs(&messages, true)?;
		let mut digest = ContentDigest::default();
		digest.update(&bytes);
		Ok(Self { fs, path, _ownership: ownership, length: bytes.len(), digest, id, leaf, messages })
	}

	pub fn id(&self) -> &str {
		&self.id
	}

	pub fn messages(&self) -> &[ChatMessage] {
		&self.messages
	}

	pub fn leaf(&self) -> Option<&str> {
		self.leaf.as_deref()
	}

	pub fn ensure_ready(&self) -> Result<()> {
		validate_pairs(&self.messages, true)
	}

	/// In-memory state advances only after the append has been synced to disk.
	pub fn commit(&mut self, message: &ChatMessage, model: &str) -> Result<String> {
		self.commit_with_usage(message, model, None)
	}

	pub fn commit_with_usage(
		&mut self,
		message: &ChatMessage,
		model: &str,
		usage: Option<&crate::usage::UsageReport>,
	) -> Result<String> {
		validate_pairs(self.messages.iter().chain(std::iter::once(message)), false)?;
		let timestamp = chrono::Utc::now();
		let mut stored = stored_message(message, model, timestamp.timestamp_millis())?;
		if let Some(usage) = usage {
			if message.role != "assistant" {
				bail!("only assistant messages can contain provider usage");
			}
			stored["usageUnavailable"] = json!(usage.tokens.is_none());
			stored["costUnavailable"] = json!(true);
			if let Some(tokens) = &usage.tokens {
				let mut normalized = serde_json::to_value(tokens)?;
				normalized["cost"] =
					json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0});
				stored["usage"] = normalized;
			}
		}
		if message.role == "tool" {
			let name = self
				.messages
				.iter()
				.rev()
				.flat_map(|row| row.tool_calls.iter().flatten())
				.find(|call| Some(&call.id) == message.tool_call_id.as_ref())
				.context("orphaned tool result")?
				.function
				.name
				.clone();
			stored["toolName"] = json!(name);
		}
		let id = content_hash(format!("{}:{}:{}", self.id, self.length, stored).as_bytes());
		let row = json!({"type":"message", "id":id, "parentId":self.leaf, "timestamp":timestamp.to_rfc3339(), "message":stored});
		// Keep unknown usage fields lossless, including integers beyond f64/u64.
		let mut raw_row: crate::session_format::SessionObject = serde_json::from_value(row)?;
		if let Some(usage) = usage {
			let mut raw_message: crate::session_format::SessionObject =
				serde_json::from_str(raw_row["message"].get())?;
			raw_message.insert("providerUsage".into(), usage.raw.clone());
			raw_row.insert("message".into(), serde_json::value::to_raw_value(&raw_message)?);
		}
		let mut bytes = serde_json::to_vec(&raw_row)?;
		if bytes.len() > harvest_runtime::MAX_EVENT_BYTES {
			bail!("session event exceeds frame limit");
		}
		bytes.push(b'\n');
		self
			.fs
			.append_checked(&self.path, &bytes, SESSION_LIMIT, Some(&self.digest.hash()))?;
		self.length += bytes.len();
		self.digest.update(&bytes);
		self.leaf = Some(id.clone());
		self.messages.push(message.clone());
		Ok(id)
	}
}

pub(crate) fn replay(raw: &serde_json::value::RawValue) -> Result<ChatMessage> {
	let raw: crate::session_format::SessionObject = serde_json::from_str(raw.get())?;
	if raw.contains_key("providerPayload") {
		bail!("opaque provider replay is not implemented");
	}
	// Usage and unknown metadata never enter the model request or a floating-
	// point Value conversion. Only replay-relevant fields need decoding.
	let mut fields = serde_json::Map::new();
	for key in ["role", "content", "toolCallId"] {
		if let Some(value) = raw.get(key) {
			fields.insert(key.into(), serde_json::from_str(value.get())?);
		}
	}
	let message = Value::Object(fields);
	let role = message["role"].as_str().context("invalid message role")?;
	if !matches!(role, "user" | "developer" | "assistant" | "toolResult") {
		bail!("unsupported replay role {role}");
	}
	let mut text = String::new();
	let mut calls = Vec::new();
	match &message["content"] {
		Value::String(value) if role != "assistant" => text.push_str(value),
		Value::Array(blocks) => {
			for block in blocks {
				match block["type"].as_str() {
					Some("text") => {
						if block.get("textSignature").is_some() {
							bail!("signed text replay is not implemented");
						}
						text.push_str(block["text"].as_str().context("invalid text block")?);
					},
					Some("toolCall") if role == "assistant" => {
						if block.get("providerMetadata").is_some()
							|| block.get("customWireName").is_some()
						{
							bail!("native tool replay is not implemented");
						}
						if !block["arguments"].is_object() {
							bail!("tool arguments must be an object");
						}
						calls.push(ToolCall {
							id:       block["id"].as_str().context("missing tool ID")?.into(),
							kind:     "function".into(),
							function: FunctionCall {
								name:      block["name"].as_str().context("missing tool name")?.into(),
								arguments: serde_json::to_string(&block["arguments"])?,
							},
						});
					},
					_ => bail!("unsupported content block requires a provider replay adapter"),
				}
			}
		},
		_ => bail!("invalid session message content"),
	}
	Ok(ChatMessage {
		role:         if role == "toolResult" { "tool" } else { role }.into(),
		content:      Some(text),
		tool_calls:   (!calls.is_empty()).then_some(calls),
		tool_call_id: if role == "toolResult" {
			Some(
				message["toolCallId"]
					.as_str()
					.context("missing result ID")?
					.into(),
			)
		} else {
			None
		},
	})
}

fn stored_message(message: &ChatMessage, model: &str, timestamp: i64) -> Result<Value> {
	let mut blocks = Vec::new();
	if let Some(text) = &message.content {
		blocks.push(json!({"type":"text", "text":text}));
	}
	for call in message.tool_calls.as_deref().unwrap_or_default() {
		let arguments: Value = serde_json::from_str(&call.function.arguments)?;
		if call.kind != "function" || !arguments.is_object() {
			bail!("unsupported tool call");
		}
		blocks.push(
			json!({"type":"toolCall", "id":call.id, "name":call.function.name, "arguments":arguments}),
		);
	}
	let mut stored = json!({"role":message.role, "content":blocks, "timestamp":timestamp});
	match message.role.as_str() {
		"assistant" => {
			stored["api"] = json!("openai-completions");
			stored["provider"] = json!("openai-compatible");
			stored["model"] = json!(model);
			stored["stopReason"] = json!(if message
				.tool_calls
				.as_ref()
				.is_some_and(|calls| !calls.is_empty())
			{
				"toolUse"
			} else {
				"stop"
			});
			// Reports are attached by the transport-aware commit path.
			// Absence must remain distinguishable from a valid zero-token report.
			stored["usageUnavailable"] = json!(true);
			stored["costUnavailable"] = json!(true);
			stored["usage"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}});
		},
		"tool" => {
			stored["role"] = json!("toolResult");
			stored["toolCallId"] = json!(
				message
					.tool_call_id
					.as_ref()
					.context("missing tool result ID")?
			);
			stored["toolName"] = json!("unknown");
			stored["isError"] = json!(
				message
					.content
					.as_ref()
					.is_some_and(|text| text.starts_with("error:"))
			);
		},
		"user" | "developer" => {},
		_ => bail!("unsupported durable message role"),
	}
	Ok(stored)
}

fn validate_pairs<'a>(
	messages: impl IntoIterator<Item = &'a ChatMessage>,
	require_closed: bool,
) -> Result<()> {
	let mut pending = std::collections::BTreeSet::new();
	for message in messages {
		if message.role == "tool" {
			let id = message
				.tool_call_id
				.as_deref()
				.context("missing tool result ID")?;
			if !pending.remove(id) {
				bail!("orphaned or duplicate tool result");
			}
		} else {
			if !pending.is_empty() {
				bail!("unresolved tool calls require recovery before resume");
			}
			for call in message.tool_calls.as_deref().unwrap_or_default() {
				if call.id.is_empty() || !pending.insert(call.id.as_str()) {
					bail!("duplicate tool call ID");
				}
			}
		}
	}
	if require_closed && !pending.is_empty() {
		bail!("unresolved tool calls require recovery before resume");
	}
	Ok(())
}
