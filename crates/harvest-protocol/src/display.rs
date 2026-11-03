//! Allowlisted read-only views of a selected persisted session branch.
//!
//! These views deliberately omit provider replay metadata, signatures,
//! redacted-thinking bytes, extension details, and unknown payloads. They are
//! not a replacement for the original session records. Text and tool arguments
//! remain untrusted input; hosts must escape/sanitize when rendering them.
use anyhow::{Context, Result};
use serde::{
	Deserialize, Deserializer, Serialize,
	de::{DeserializeOwned, Error},
};
use serde_json::value::RawValue;

use crate::session::{SessionDocument, SessionObject};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayProjection {
	pub session_id: String,
	pub title:      Option<String>,
	pub cwd:        Option<String>,
	pub leaf_id:    String,
	pub entries:    Vec<DisplayEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEntry {
	pub id:        String,
	pub parent_id: Option<String>,
	pub timestamp: Option<String>,
	pub content:   DisplayContent,
}

impl DisplayEntry {
	pub const fn is_visible(&self) -> bool {
		!matches!(self.content, DisplayContent::Hidden | DisplayContent::Unknown { .. })
	}
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum DisplayContent {
	Message {
		message: Box<DisplayMessage>,
	},
	CustomMessage {
		custom_type: String,
		blocks:      Vec<DisplayBlock>,
	},
	Compaction {
		summary:             String,
		short_summary:       Option<String>,
		first_kept_entry_id: Option<String>,
		tokens_before:       Option<u64>,
		tokens_after:        Option<u64>,
	},
	BranchSummary {
		summary: String,
		from_id: Option<String>,
	},
	ModelChange {
		model: String,
		role:  Option<String>,
	},
	ThinkingLevelChange {
		thinking_level: Option<String>,
	},
	/// Preserve tree identity without forwarding a display:false custom body.
	Hidden,
	/// Unknown records remain in the source document and do not enter the view.
	Unknown {
		entry_type: String,
	},
}

// Serde's internally tagged enum visitor buffers through a generic value,
// which cannot retain RawValue tool arguments. Parse the tagged boundaries as
// raw objects before decoding their selected display fields instead.
impl<'de> Deserialize<'de> for DisplayContent {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
		let object = SessionObject::deserialize(deserializer)?;
		let kind: String = required(&object, "kind")?;
		Ok(match kind.as_str() {
			"message" => Self::Message { message: required(&object, "message")? },
			"custom_message" => Self::CustomMessage {
				custom_type: required(&object, "customType")?,
				blocks:      required(&object, "blocks")?,
			},
			"compaction" => Self::Compaction {
				summary:             required(&object, "summary")?,
				short_summary:       field(&object, "shortSummary"),
				first_kept_entry_id: field(&object, "firstKeptEntryId"),
				tokens_before:       field(&object, "tokensBefore"),
				tokens_after:        field(&object, "tokensAfter"),
			},
			"branch_summary" => Self::BranchSummary {
				summary: required(&object, "summary")?,
				from_id: field(&object, "fromId"),
			},
			"model_change" => {
				Self::ModelChange { model: required(&object, "model")?, role: field(&object, "role") }
			},
			"thinking_level_change" => {
				Self::ThinkingLevelChange { thinking_level: field(&object, "thinkingLevel") }
			},
			"hidden" => Self::Hidden,
			"unknown" => Self::Unknown { entry_type: required(&object, "entryType")? },
			_ => return Err(D::Error::custom("unknown display entry kind")),
		})
	}
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMessage {
	pub role:          String,
	pub blocks:        Vec<DisplayBlock>,
	pub model:         Option<String>,
	pub tool_call_id:  Option<String>,
	pub tool_name:     Option<String>,
	pub is_error:      bool,
	pub error_message: Option<String>,
	pub stop_reason:   Option<String>,
	pub execution:     Option<DisplayExecution>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DisplayBlock {
	Text {
		text: String,
	},
	Thinking {
		thinking: String,
	},
	Image {
		data:      String,
		mime_type: String,
	},
	ToolCall {
		id:        String,
		name:      String,
		#[serde(skip_serializing_if = "Option::is_none")]
		arguments: Option<Box<RawValue>>,
		intent:    Option<String>,
	},
	/// There is intentionally no data/signature field in this display variant.
	RedactedThinking,
	Unknown {
		block_type: String,
	},
}

impl<'de> Deserialize<'de> for DisplayBlock {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
		let object = SessionObject::deserialize(deserializer)?;
		let kind: String = required(&object, "type")?;
		Ok(match kind.as_str() {
			"text" => Self::Text { text: required(&object, "text")? },
			"thinking" => Self::Thinking { thinking: required(&object, "thinking")? },
			"image" => Self::Image {
				data:      required(&object, "data")?,
				mime_type: required(&object, "mimeType")?,
			},
			"toolCall" => Self::ToolCall {
				id:        required(&object, "id")?,
				name:      required(&object, "name")?,
				arguments: object.get("arguments").cloned(),
				intent:    field(&object, "intent"),
			},
			"redactedThinking" => Self::RedactedThinking,
			"unknown" => Self::Unknown { block_type: required(&object, "blockType")? },
			_ => return Err(D::Error::custom("unknown display block type")),
		})
	}
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayExecution {
	pub language:  ExecutionLanguage,
	pub code:      String,
	pub output:    String,
	pub exit_code: Option<i32>,
	pub cancelled: bool,
	pub truncated: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionLanguage {
	Shell,
	Python,
	Javascript,
}

/// Select one branch in causal order without modifying its persisted records.
///
/// # Errors
/// Rejects a missing/cyclic parent chain or malformed required session/tree
/// IDs.
pub fn project_branch(document: &SessionDocument, leaf: &str) -> Result<ReplayProjection> {
	let entries = document
		.branch(leaf)?
		.into_iter()
		.map(project_entry)
		.collect::<Result<_>>()?;
	Ok(ReplayProjection {
		session_id: field(document.header(), "id").context("invalid display session ID")?,
		title: field(document.header(), "title"),
		cwd: field(document.header(), "cwd"),
		leaf_id: leaf.to_owned(),
		entries,
	})
}

fn field<T: DeserializeOwned>(object: &SessionObject, name: &str) -> Option<T> {
	object
		.get(name)
		.and_then(|raw| serde_json::from_str(raw.get()).ok())
}

fn required<T: DeserializeOwned, E: Error>(
	object: &SessionObject,
	name: &str,
) -> std::result::Result<T, E> {
	field(object, name).ok_or_else(|| E::custom(format!("invalid display {name}")))
}

fn project_entry(entry: &SessionObject) -> Result<DisplayEntry> {
	let entry_type = field::<String>(entry, "type").unwrap_or_default();
	let content = match entry_type.as_str() {
		"message" => match field::<SessionObject>(entry, "message") {
			Some(message) => project_message(&message),
			None => DisplayContent::Unknown { entry_type },
		},
		"custom_message" if field::<bool>(entry, "display") != Some(true) => DisplayContent::Hidden,
		"custom_message" => DisplayContent::CustomMessage {
			custom_type: field(entry, "customType").unwrap_or_default(),
			blocks:      content_blocks(entry.get("content").map(Box::as_ref)),
		},
		"compaction" => DisplayContent::Compaction {
			summary:             field(entry, "summary").unwrap_or_default(),
			short_summary:       field(entry, "shortSummary"),
			first_kept_entry_id: field(entry, "firstKeptEntryId"),
			tokens_before:       field(entry, "tokensBefore"),
			tokens_after:        field(entry, "tokensAfter"),
		},
		"branch_summary" => DisplayContent::BranchSummary {
			summary: field(entry, "summary").unwrap_or_default(),
			from_id: field(entry, "fromId"),
		},
		"model_change" => DisplayContent::ModelChange {
			model: field(entry, "model").unwrap_or_default(),
			role:  field(entry, "role"),
		},
		"thinking_level_change" => {
			DisplayContent::ThinkingLevelChange { thinking_level: field(entry, "thinkingLevel") }
		},
		_ => DisplayContent::Unknown { entry_type },
	};
	Ok(DisplayEntry {
		id: field(entry, "id").context("invalid display entry ID")?,
		parent_id: field(entry, "parentId").context("invalid display parent ID")?,
		timestamp: field(entry, "timestamp"),
		content,
	})
}

fn project_message(message: &SessionObject) -> DisplayContent {
	let role = field::<String>(message, "role").unwrap_or_default();
	if matches!(role.as_str(), "custom" | "hookMessage")
		&& field::<bool>(message, "display") != Some(true)
	{
		return DisplayContent::Hidden;
	}
	let mut blocks = content_blocks(message.get("content").map(Box::as_ref));
	if let Some(images) = message.get("images") {
		blocks.extend(content_blocks(Some(images.as_ref())));
	}
	let execution = match role.as_str() {
		"bashExecution" => Some((ExecutionLanguage::Shell, "command")),
		"pythonExecution" => Some((ExecutionLanguage::Python, "code")),
		"jsExecution" | "javascriptExecution" => Some((ExecutionLanguage::Javascript, "code")),
		_ => None,
	}
	.map(|(language, source)| DisplayExecution {
		language,
		code: field(message, source).unwrap_or_default(),
		output: field(message, "output").unwrap_or_default(),
		exit_code: field(message, "exitCode"),
		cancelled: field(message, "cancelled").unwrap_or(false),
		truncated: field(message, "truncated").unwrap_or(false),
	});
	DisplayContent::Message {
		message: Box::new(DisplayMessage {
			role,
			blocks,
			model: field(message, "model"),
			tool_call_id: field(message, "toolCallId"),
			tool_name: field(message, "toolName"),
			is_error: field(message, "isError").unwrap_or(false),
			error_message: field(message, "errorMessage"),
			stop_reason: field(message, "stopReason"),
			execution,
		}),
	}
}

fn content_blocks(raw: Option<&RawValue>) -> Vec<DisplayBlock> {
	let Some(raw) = raw else { return Vec::new() };
	if let Ok(text) = serde_json::from_str::<String>(raw.get()) {
		return vec![DisplayBlock::Text { text }];
	}
	let Ok(blocks) = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) else {
		return Vec::new();
	};
	blocks
		.into_iter()
		.filter_map(|block| serde_json::from_str::<SessionObject>(block.get()).ok())
		.map(|block| {
			let kind = field::<String>(&block, "type").unwrap_or_default();
			match kind.as_str() {
				"text" if field::<String>(&block, "text").is_some() => {
					DisplayBlock::Text { text: field(&block, "text").unwrap_or_default() }
				},
				"thinking" if field::<String>(&block, "thinking").is_some() => {
					DisplayBlock::Thinking { thinking: field(&block, "thinking").unwrap_or_default() }
				},
				"image"
					if field::<String>(&block, "data").is_some()
						&& field::<String>(&block, "mimeType").is_some() =>
				{
					DisplayBlock::Image {
						data:      field(&block, "data").unwrap_or_default(),
						mime_type: field(&block, "mimeType").unwrap_or_default(),
					}
				},
				"toolCall"
					if field::<String>(&block, "id").is_some()
						&& field::<String>(&block, "name").is_some() =>
				{
					DisplayBlock::ToolCall {
						id:        field(&block, "id").unwrap_or_default(),
						name:      field(&block, "name").unwrap_or_default(),
						arguments: block.get("arguments").cloned(),
						intent:    field(&block, "intent"),
					}
				},
				"redactedThinking" => DisplayBlock::RedactedThinking,
				_ => DisplayBlock::Unknown { block_type: kind },
			}
		})
		.collect()
}
