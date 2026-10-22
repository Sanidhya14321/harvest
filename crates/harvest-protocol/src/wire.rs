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
