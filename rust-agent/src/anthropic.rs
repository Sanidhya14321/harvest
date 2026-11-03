//! Anthropic Messages wire (`POST {base}/v1/messages`).
//!
//! Translates the OpenAI-shaped transcript: system messages hoist into the
//! top-level `system`, assistant tool calls become `tool_use` blocks, tool
//! results become `tool_result` blocks in a user turn, and consecutive
//! same-role turns merge so roles alternate as the API requires.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
	chat::{ChatMessage, FunctionCall, ToolCall, ToolSpec},
	config::Settings,
	provider::{Model, ModelResponse, ProviderRequestError},
	usage::UsageReport,
};

const API_VERSION: &str = "2023-06-01";
/// Non-streaming requests stay well under the long-request ceiling.
const MAX_OUTPUT_TOKENS: u64 = 16_384;

#[derive(Clone)]
pub struct AnthropicModel {
	settings: Settings,
}

impl std::fmt::Debug for AnthropicModel {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("AnthropicModel")
			.field("model", &self.settings.model)
			.finish_non_exhaustive()
	}
}

impl AnthropicModel {
	pub const fn new(settings: Settings) -> Self {
		Self { settings }
	}

	fn body(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<Value> {
		let (system, turns) = convert(messages);
		let mut body = json!({
			"model": self.settings.model,
			"max_tokens": self.settings.max_tokens.unwrap_or(MAX_OUTPUT_TOKENS).min(MAX_OUTPUT_TOKENS),
			"messages": turns,
		});
		if !system.is_empty() {
			body["system"] = Value::String(system);
		}
		if !tools.is_empty() {
			body["tools"] = tools.iter().map(tool).collect::<Result<Vec<_>>>()?.into();
		}
		Ok(body)
	}
}

fn tool(spec: &ToolSpec) -> Result<Value> {
	let mut value = serde_json::to_value(spec)?;
	let function = value
		.get_mut("function")
		.map(Value::take)
		.context("tool spec has no function")?;
	Ok(json!({
		"name": function["name"],
		"description": function["description"],
		"input_schema": function["parameters"],
	}))
}

fn convert(messages: &[ChatMessage]) -> (String, Vec<Value>) {
	let mut system = Vec::new();
	let mut turns: Vec<(&'static str, Vec<Value>)> = Vec::new();
	for message in messages {
		let text = message.content.as_deref().unwrap_or_default();
		let (role, blocks) = match message.role.as_str() {
			"system" | "developer" => {
				system.push(text);
				continue;
			},
			"assistant" => {
				let mut blocks = Vec::new();
				if !text.is_empty() {
					blocks.push(json!({"type": "text", "text": text}));
				}
				for call in message.tool_calls.iter().flatten() {
					let input = serde_json::from_str::<Value>(&call.function.arguments)
						.ok()
						.filter(Value::is_object)
						.unwrap_or_else(|| json!({}));
					blocks.push(json!({
						"type": "tool_use",
						"id": call.id,
						"name": call.function.name,
						"input": input,
					}));
				}
				("assistant", blocks)
			},
			"tool" => ("user", vec![json!({
				"type": "tool_result",
				"tool_use_id": message.tool_call_id.as_deref().unwrap_or_default(),
				"content": text,
			})]),
			_ => ("user", vec![json!({"type": "text", "text": text})]),
		};
		if blocks.is_empty() {
			continue;
		}
		match turns.last_mut() {
			Some((last, existing)) if *last == role => existing.extend(blocks),
			_ => turns.push((role, blocks)),
		}
	}
	let turns = turns
		.into_iter()
		.map(|(role, content)| json!({"role": role, "content": content}))
		.collect();
	(system.join("\n\n"), turns)
}

#[derive(Debug, Deserialize)]
struct Response {
	content:     Vec<Block>,
	stop_reason: Option<String>,
	usage:       Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
	Text {
		text: String,
	},
	ToolUse {
		id:    String,
		name:  String,
		input: Value,
	},
	#[serde(other)]
	Other,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_field_names, reason = "wire field names")]
struct Usage {
	input_tokens:                u64,
	output_tokens:               u64,
	#[serde(default)]
	cache_read_input_tokens:     u64,
	#[serde(default)]
	cache_creation_input_tokens: u64,
}

impl Response {
	fn into_response(self) -> Result<ModelResponse> {
		if let Some(reason) = &self.stop_reason
			&& !matches!(reason.as_str(), "end_turn" | "tool_use" | "stop_sequence")
		{
			bail!("model stopped with an unsuccessful finish reason");
		}
		let mut text = String::new();
		let mut calls = Vec::new();
		for block in self.content {
			match block {
				Block::Text { text: part } => text.push_str(&part),
				Block::ToolUse { id, name, input } => calls.push(ToolCall {
					id,
					kind: "function".to_owned(),
					function: FunctionCall { name, arguments: input.to_string() },
				}),
				Block::Other => {},
			}
		}
		// Usage is re-expressed in the OpenAI shape the accountant reads:
		// Anthropic's `input_tokens` excludes cache reads and writes.
		let usage = self.usage.map(|usage| {
			let raw = json!({
				"prompt_tokens": usage.input_tokens + usage.cache_read_input_tokens + usage.cache_creation_input_tokens,
				"completion_tokens": usage.output_tokens,
				"prompt_tokens_details": {
					"cached_tokens": usage.cache_read_input_tokens,
					"cache_write_tokens": usage.cache_creation_input_tokens,
				},
			});
			UsageReport::from_raw(
				serde_json::value::to_raw_value(&raw).expect("usage serializes"),
			)
		});
		Ok(ModelResponse {
			message: ChatMessage {
				role:         "assistant".to_owned(),
				content:      (!text.is_empty()).then_some(text),
				tool_calls:   (!calls.is_empty()).then_some(calls),
				tool_call_id: None,
			},
			usage,
		})
	}
}

impl Model for AnthropicModel {
	fn model_id(&self) -> &str {
		&self.settings.model
	}

	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
		Ok(self.complete_response(messages, tools)?.message)
	}

	fn complete_response(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
	) -> Result<ModelResponse> {
		let agent = ureq::AgentBuilder::new()
			.timeout(Duration::from_secs(300))
			.build();
		let url = format!("{}/v1/messages", self.settings.base_url.trim_end_matches('/'));
		let mut call = agent
			.post(&url)
			.set("anthropic-version", API_VERSION)
			.set("content-type", "application/json");
		if !self.settings.api_key.is_empty() {
			call = call.set("x-api-key", &self.settings.api_key);
		}
		let response =
			call
				.send_json(self.body(messages, tools)?)
				.map_err(|error| -> anyhow::Error {
					match error {
						ureq::Error::Status(code, _) => ProviderRequestError::Status(code),
						ureq::Error::Transport(transport) => {
							ProviderRequestError::Transport(transport.kind())
						},
					}
					.into()
				})?;
		let bytes =
			harvest_runtime::read_bounded(response.into_reader(), harvest_runtime::MAX_HTTP_BYTES)?;
		let parsed: Response =
			serde_json::from_slice(&bytes).context("model response was not valid JSON")?;
		parsed.into_response()
	}
}

#[cfg(test)]
mod tests;
