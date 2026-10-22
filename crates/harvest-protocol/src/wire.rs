//! Shared collab wire constants.
//!
//! These values are the Rust-owned source of truth for the collab protocol.
//! Transitional TypeScript consumers continue to define their own copies until
//! their switch-over is complete.

pub const COLLAB_PROMPT_MESSAGE_TYPE: &str = "collab-prompt";
pub const COLLAB_PROTO: u8 = 3;
pub const ENVELOPE_HEADER_LENGTH: usize = 4;
pub const ROOM_ID_BYTES: usize = 16;
pub const ROOM_KEY_BYTES: usize = 32;
pub const WRITE_TOKEN_BYTES: usize = 16;
pub const DEFAULT_RELAY_URL: &str = "wss://my.omp.sh";
pub const DEFAULT_SHARE_URL: &str = "https://my.omp.sh/s";
pub const INTENT_FIELD: &str = "i";

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn collab_constants_match_legacy_wire_values() {
		assert_eq!(COLLAB_PROTO, 3);
		assert_eq!(COLLAB_PROMPT_MESSAGE_TYPE, "collab-prompt");
		assert_eq!(ENVELOPE_HEADER_LENGTH, 4);
		assert_eq!(ROOM_ID_BYTES, 16);
		assert_eq!(DEFAULT_RELAY_URL, "wss://my.omp.sh");
	}
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireUsage {
	pub input:        u64,
	pub output:       u64,
	pub cache_read:   u64,
	pub cache_write:  u64,
	pub total_tokens: u64,
	pub cost:         WireCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireCost {
	pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireModel {
	pub id:             String,
	pub name:           String,
	pub provider:       String,
	pub context_window: Option<u64>,
}

#[cfg(test)]
mod wire_types_tests {
	use super::*;

	#[test]
	fn wire_usage_round_trips() {
		let usage = WireUsage {
			input:        1,
			output:       2,
			cache_read:   3,
			cache_write:  4,
			total_tokens: 10,
			cost:         WireCost { total: 0.5 },
		};
		let json = serde_json::to_string(&usage).unwrap();
		let decoded: WireUsage = serde_json::from_str(&json).unwrap();
		assert_eq!(decoded, usage);
	}

	#[test]
	fn wire_model_round_trips() {
		let model = WireModel {
			id:             "m".into(),
			name:           "M".into(),
			provider:       "p".into(),
			context_window: Some(1000),
		};
		let json = serde_json::to_string(&model).unwrap();
		let decoded: WireModel = serde_json::from_str(&json).unwrap();
		assert_eq!(decoded, model);
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantContent {
	Text(TextContent),
	#[serde(rename = "thinking")]
	Thinking(ThinkingContent),
	#[serde(rename = "redactedThinking")]
	RedactedThinking(RedactedThinkingContent),
	#[serde(rename = "toolCall")]
	ToolCall(ToolCallContent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
	pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageContent {
	pub data:      String,
	#[serde(rename = "mimeType")]
	pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingContent {
	pub thinking: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedThinkingContent {
	pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallContent {
	pub id:        String,
	pub name:      String,
	pub arguments: serde_json::Value,
	pub intent:    Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
	Stop,
	Length,
	ToolUse,
	Error,
	Aborted,
}

#[cfg(test)]
mod wire_content_tests {
	use super::*;

	#[test]
	fn assistant_content_round_trips() {
		let content = AssistantContent::ToolCall(ToolCallContent {
			id:        "1".into(),
			name:      "bash".into(),
			arguments: serde_json::json!({"cmd":"ls"}),
			intent:    Some("list".into()),
		});
		let json = serde_json::to_string(&content).unwrap();
		assert!(json.contains("\"type\":\"toolCall\""));
		let decoded: AssistantContent = serde_json::from_str(&json).unwrap();
		assert_eq!(decoded, content);
	}

	#[test]
	fn stop_reason_uses_camel_case() {
		let json = serde_json::to_string(&StopReason::ToolUse).unwrap();
		assert_eq!(json, "\"toolUse\"");
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WireContent {
	Text(String),
	Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
	Text(TextContent),
	Image(ImageContent),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum WireMessage {
	User(UserMessage),
	#[serde(rename = "developer")]
	Developer(DeveloperMessage),
	#[serde(rename = "assistant")]
	Assistant(AssistantMessage),
	#[serde(rename = "toolResult")]
	ToolResult(ToolResultMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
	pub content:   WireContent,
	pub synthetic: Option<bool>,
	pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperMessage {
	pub content:   WireContent,
	pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
	pub content:       Vec<AssistantContent>,
	pub model:         String,
	pub usage:         WireUsage,
	#[serde(rename = "stopReason")]
	pub stop_reason:   StopReason,
	#[serde(rename = "errorMessage")]
	pub error_message: Option<String>,
	pub timestamp:     i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
	#[serde(rename = "toolCallId")]
	pub tool_call_id: String,
	#[serde(rename = "toolName")]
	pub tool_name:    String,
	pub content:      Vec<ContentBlock>,
	pub details:      Option<serde_json::Value>,
	#[serde(rename = "isError")]
	pub is_error:     bool,
	pub timestamp:    i64,
}

#[cfg(test)]
mod wire_message_tests {
	use super::*;

	#[test]
	fn user_message_string_content_round_trips() {
		let msg = WireMessage::User(UserMessage {
			content:   WireContent::Text("hi".into()),
			synthetic: Some(true),
			timestamp: 1,
		});
		let json = serde_json::to_string(&msg).unwrap();
		assert!(json.contains("\"role\":\"user\""));
		let decoded: WireMessage = serde_json::from_str(&json).unwrap();
		assert_eq!(decoded, msg);
	}

	#[test]
	fn assistant_message_round_trips() {
		let msg = WireMessage::Assistant(AssistantMessage {
			content:       vec![AssistantContent::Text(TextContent { text: "x".into() })],
			model:         "m".into(),
			usage:         WireUsage {
				input:        1,
				output:       2,
				cache_read:   0,
				cache_write:  0,
				total_tokens: 3,
				cost:         WireCost { total: 0.0 },
			},
			stop_reason:   StopReason::Stop,
			error_message: None,
			timestamp:     2,
		});
		let json = serde_json::to_string(&msg).unwrap();
		let decoded: WireMessage = serde_json::from_str(&json).unwrap();
		assert_eq!(decoded, msg);
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHeader {
	#[serde(rename = "type")]
	pub kind:      String,
	pub id:        String,
	pub title:     Option<String>,
	pub timestamp: String,
	pub cwd:       String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryBase {
	pub id:        String,
	pub parent_id: Option<String>,
	pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEntry {
	#[serde(flatten)]
	pub base:    EntryBase,
	pub message: WireMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessageEntry {
	#[serde(flatten)]
	pub base:        EntryBase,
	#[serde(rename = "customType")]
	pub custom_type: String,
	pub content:     WireContent,
	pub details:     Option<serde_json::Value>,
	pub display:     bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionEntry {
	#[serde(flatten)]
	pub base:                EntryBase,
	pub summary:             String,
	#[serde(rename = "shortSummary")]
	pub short_summary:       Option<String>,
	#[serde(rename = "firstKeptEntryId")]
	pub first_kept_entry_id: String,
	#[serde(rename = "tokensBefore")]
	pub tokens_before:       u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryEntry {
	#[serde(flatten)]
	pub base:    EntryBase,
	#[serde(rename = "fromId")]
	pub from_id: String,
	pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChangeEntry {
	#[serde(flatten)]
	pub base:  EntryBase,
	pub model: String,
	pub role:  Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingLevelChangeEntry {
	#[serde(flatten)]
	pub base:           EntryBase,
	#[serde(rename = "thinkingLevel")]
	pub thinking_level: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
	#[serde(rename = "message")]
	Message(MessageEntry),
	#[serde(rename = "custom_message")]
	CustomMessage(CustomMessageEntry),
	#[serde(rename = "compaction")]
	Compaction(CompactionEntry),
	#[serde(rename = "branch_summary")]
	BranchSummary(BranchSummaryEntry),
	#[serde(rename = "model_change")]
	ModelChange(ModelChangeEntry),
	#[serde(rename = "thinking_level_change")]
	ThinkingLevelChange(ThinkingLevelChangeEntry),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
	AgentStart,
	AgentEnd,
	TurnStart,
	TurnEnd,
	MessageStart {
		message: WireMessage,
	},
	MessageUpdate {
		message: WireMessage,
	},
	MessageEnd {
		message: WireMessage,
	},
	ToolExecutionStart {
		#[serde(rename = "toolCallId")]
		tool_call_id: String,
		#[serde(rename = "toolName")]
		tool_name:    String,
		args:         serde_json::Value,
		intent:       Option<String>,
	},
	ToolExecutionUpdate {
		#[serde(rename = "toolCallId")]
		tool_call_id:   String,
		#[serde(rename = "toolName")]
		tool_name:      String,
		args:           serde_json::Value,
		partial_result: serde_json::Value,
	},
	ToolExecutionEnd {
		#[serde(rename = "toolCallId")]
		tool_call_id: String,
		#[serde(rename = "toolName")]
		tool_name:    String,
		result:       serde_json::Value,
		#[serde(rename = "isError")]
		is_error:     Option<bool>,
	},
	Notice {
		level:   String,
		message: String,
		source:  Option<String>,
	},
	AutoCompactionStart {
		reason: String,
		action: String,
	},
	AutoCompactionEnd {
		aborted:       bool,
		#[serde(rename = "willRetry")]
		will_retry:    bool,
		#[serde(rename = "errorMessage")]
		error_message: Option<String>,
		skipped:       Option<bool>,
	},
	AutoRetryStart {
		attempt:       u64,
		#[serde(rename = "maxAttempts")]
		max_attempts:  u64,
		#[serde(rename = "delayMs")]
		delay_ms:      u64,
		#[serde(rename = "errorMessage")]
		error_message: String,
	},
	AutoRetryEnd {
		success:     bool,
		attempt:     u64,
		#[serde(rename = "finalError")]
		final_error: Option<String>,
	},
	ThinkingLevelChanged {
		#[serde(rename = "thinkingLevel")]
		thinking_level: Option<Option<String>>,
	},
}

#[cfg(test)]
mod agent_event_tests {
	use super::*;

	#[test]
	fn tool_execution_start_round_trips() {
		let event = AgentEvent::ToolExecutionStart {
			tool_call_id: "c".into(),
			tool_name:    "bash".into(),
			args:         serde_json::json!({"a":1}),
			intent:       None,
		};
		let json = serde_json::to_string(&event).unwrap();
		assert!(json.contains("tool_execution_start"));
		let decoded: AgentEvent = serde_json::from_str(&json).unwrap();
		assert_eq!(decoded, event);
	}
}
