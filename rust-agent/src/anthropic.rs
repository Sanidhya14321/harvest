//! Explicit Anthropic Messages text/tool wire boundary for the native agent.
//! Multimodal, thinking, OAuth and catalog policy remain separate migration
//! work.
use std::{
	collections::{BTreeMap, BTreeSet},
	io::Read,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{MAX_HTTP_BYTES, cancel::CancelToken};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

use crate::{
	chat::{ChatMessage, FunctionCall, ToolCall, ToolSpec},
	config::Settings,
	provider::ModelResponse,
	usage::UsageReport,
};

fn request_body(
	settings: &Settings,
	tokens: u32,
	messages: &[ChatMessage],
	tools: &[ToolSpec],
	streaming: bool,
) -> Result<Vec<u8>> {
	let mut system = Vec::new();
	let mut wire: Vec<Value> = Vec::new();
	let mut pending = BTreeSet::new();
	let mut identities = BTreeSet::new();
	for message in messages {
		if message.role == "system" {
			if !wire.is_empty() || message.tool_calls.is_some() || message.tool_call_id.is_some() {
				bail!("Messages API requires initial text system messages");
			}
			system.push(
				json!({"type":"text", "text":message.content.as_deref().context("empty system message")?}),
			);
			continue;
		}
		let mut blocks = Vec::new();
		let role = match message.role.as_str() {
			"tool" => {
				let id = message
					.tool_call_id
					.as_deref()
					.context("tool result has no identity")?;
				if !pending.remove(id) || message.tool_calls.is_some() {
					bail!("orphan or duplicate tool result");
				}
				blocks.push(json!({"type":"tool_result", "tool_use_id":id, "content":message.content.as_deref().unwrap_or("")}));
				"user"
			},
			"user" | "assistant" => {
				if !pending.is_empty() || message.tool_call_id.is_some() {
					bail!("tool results must precede the next conversation turn");
				}
				if let Some(text) = &message.content {
					blocks.push(json!({"type":"text", "text":text}));
				}
				if let Some(calls) = &message.tool_calls {
					if message.role != "assistant" || calls.len() > 32 {
						bail!("invalid tool call owner or count");
					}
					for call in calls {
						if call.kind != "function"
							|| call.id.is_empty()
							|| call.function.name.is_empty()
							|| !identities.insert(call.id.as_str())
						{
							bail!("invalid or duplicate tool identity");
						}
						let input: Value = serde_json::from_str(&call.function.arguments)
							.context("invalid tool arguments")?;
						if !input.is_object() {
							bail!("tool arguments must be an object");
						}
						pending.insert(call.id.as_str());
						blocks.push(
							json!({"type":"tool_use", "id":call.id, "name":call.function.name, "input":input}),
						);
					}
				}
				message.role.as_str()
			},
			_ => bail!("unsupported Messages API conversation role"),
		};
		if blocks.is_empty() {
			bail!("empty Messages API conversation turn");
		}
		if let Some(last) = wire.last_mut().filter(|last| last["role"] == role) {
			last["content"]
				.as_array_mut()
				.context("invalid content")?
				.extend(blocks);
		} else {
			wire.push(json!({"role":role, "content":blocks}));
		}
	}
	if !pending.is_empty() {
		bail!("conversation has unresolved tool calls");
	}
	let tools = tools.iter().map(|tool| {
		let value = serde_json::to_value(tool)?;
		Ok(json!({"name":value["function"]["name"], "description":value["function"]["description"], "input_schema":value["function"]["parameters"]}))
	}).collect::<Result<Vec<_>>>()?;
	Ok(serde_json::to_vec(
		&json!({"model":settings.model, "max_tokens":tokens, "system":system, "messages":wire, "tools":tools, "stream":streaming}),
	)?)
}

fn request(
	settings: &Settings,
	tokens: u32,
	messages: &[ChatMessage],
	tools: &[ToolSpec],
	streaming: bool,
	cancellation: &CancelToken,
	headers: Vec<(String, String)>,
) -> Result<crate::transport::Response> {
	let base = settings.base_url.trim_end_matches('/');
	let endpoint = if base.ends_with("/messages") {
		base.to_owned()
	} else {
		format!("{base}/messages")
	};
	crate::transport::Response::open(
		endpoint,
		headers,
		request_body(settings, tokens, messages, tools, streaming)?,
		streaming,
		cancellation,
	)
}

#[derive(Deserialize)]
struct Message {
	#[serde(rename = "type")]
	kind:        String,
	role:        String,
	content:     Vec<Block>,
	stop_reason: Option<String>,
	usage:       Option<Box<RawValue>>,
}
#[derive(Deserialize)]
struct Block {
	#[serde(rename = "type")]
	kind:  String,
	text:  Option<String>,
	id:    Option<String>,
	name:  Option<String>,
	input: Option<Value>,
}

fn validate_stop(reason: Option<&str>, tools: bool) -> Result<()> {
	match reason {
		Some("tool_use") if tools => Ok(()),
		Some("end_turn" | "stop_sequence") if !tools => Ok(()),
		_ => bail!("Messages API stopped without a successful matching stop reason"),
	}
}
fn assemble(
	blocks: Vec<Block>,
	reason: Option<&str>,
	usage: Option<Box<RawValue>>,
) -> Result<ModelResponse> {
	let mut content = String::new();
	let mut calls = Vec::new();
	let mut ids = BTreeSet::new();
	for block in blocks {
		match block.kind.as_str() {
			"text" => content.push_str(block.text.as_deref().context("text block has no text")?),
			"tool_use" => {
				let id = block.id.context("tool block has no identity")?;
				let name = block.name.context("tool block has no name")?;
				let input = block.input.context("tool block has no input")?;
				if id.is_empty()
					|| name.is_empty()
					|| !ids.insert(id.clone())
					|| !input.is_object()
					|| calls.len() >= 32
				{
					bail!("invalid Messages API tool block");
				}
				calls.push(ToolCall {
					id,
					kind: "function".into(),
					function: FunctionCall { name, arguments: serde_json::to_string(&input)? },
				});
			},
			_ => bail!(
				"unsupported Messages API content block; native lossless block replay is required"
			),
		}
	}
	validate_stop(reason, !calls.is_empty())?;
	Ok(ModelResponse {
		message: ChatMessage {
			role:         "assistant".into(),
			content:      (!content.is_empty()).then_some(content),
			tool_calls:   (!calls.is_empty()).then_some(calls),
			tool_call_id: None,
		},
		usage:   usage.map(UsageReport::from_anthropic_raw),
	})
}
fn decode_json(reader: impl Read) -> Result<ModelResponse> {
	let bytes = harvest_runtime::read_bounded(reader, MAX_HTTP_BYTES)?;
	let message: Message =
		serde_json::from_slice(&bytes).context("invalid Messages API response")?;
	if message.kind != "message" || message.role != "assistant" {
		bail!("invalid Messages API message identity");
	}
	assemble(message.content, message.stop_reason.as_deref(), message.usage)
}
pub(crate) fn complete(
	settings: &Settings,
	tokens: u32,
	messages: &[ChatMessage],
	tools: &[ToolSpec],
	headers: Vec<(String, String)>,
) -> Result<ModelResponse> {
	decode_json(request(settings, tokens, messages, tools, false, &CancelToken::default(), headers)?)
}
pub(crate) fn stream(
	settings: &Settings,
	tokens: u32,
	messages: &[ChatMessage],
	tools: &[ToolSpec],
	cancellation: &CancelToken,
	on_text: &mut dyn FnMut(&str) -> Result<()>,
	headers: Vec<(String, String)>,
) -> Result<ModelResponse> {
	cancellation.heartbeat()?;
	let response = request(settings, tokens, messages, tools, true, cancellation, headers)?;
	if response.sse {
		decode_stream(response, cancellation, on_text)
	} else {
		let response = decode_json(response)?;
		cancellation.heartbeat()?;
		if let Some(text) = &response.message.content {
			on_text(text)?;
		}
		cancellation.heartbeat()?;
		Ok(response)
	}
}

#[derive(Deserialize)]
struct Event {
	#[serde(rename = "type")]
	kind:          String,
	index:         Option<usize>,
	message:       Option<Message>,
	content_block: Option<Block>,
	delta:         Option<Delta>,
	usage:         Option<Box<RawValue>>,
}
#[derive(Deserialize)]
struct Delta {
	#[serde(rename = "type")]
	kind:         Option<String>,
	text:         Option<String>,
	partial_json: Option<String>,
	stop_reason:  Option<String>,
}
fn merge_usage(
	usage: &mut BTreeMap<String, Box<RawValue>>,
	raw: Option<Box<RawValue>>,
) -> Result<()> {
	if let Some(raw) = raw {
		usage.extend(
			serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(raw.get())
				.context("invalid Messages API usage")?,
		);
	}
	Ok(())
}

pub(crate) fn decode_stream(
	reader: impl Read,
	cancellation: &CancelToken,
	on_text: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<ModelResponse> {
	let mut events = crate::sse::Events::new(reader);
	let mut started = false;
	let mut finished = false;
	let mut active: Option<(Block, String)> = None;
	let mut blocks = Vec::new();
	let mut usage = BTreeMap::new();
	let mut reason = None;
	while let Some(raw) = events.next(cancellation)? {
		let event: Event = serde_json::from_str(&raw).context("invalid Messages API stream event")?;
		match event.kind.as_str() {
			"ping" => {},
			"message_start" if !started => {
				let message = event.message.context("missing start message")?;
				if message.kind != "message"
					|| message.role != "assistant"
					|| !message.content.is_empty()
					|| message.stop_reason.is_some()
				{
					bail!("invalid Messages API stream start");
				}
				merge_usage(&mut usage, message.usage)?;
				started = true;
			},
			"content_block_start" if started && !finished && active.is_none() => {
				if event.index != Some(blocks.len()) || blocks.len() >= 1024 {
					bail!("invalid Messages API block order or count");
				}
				let block = event.content_block.context("missing content block")?;
				match block.kind.as_str() {
					"text" => {
						let text = block.text.as_deref().context("missing text")?;
						if !text.is_empty() {
							on_text(text)?;
						}
					},
					"tool_use"
						if block.input.as_ref().is_some_and(|input| {
							input.as_object().is_some_and(serde_json::Map::is_empty)
						}) => {},
					_ => bail!("unsupported Messages API stream block"),
				}
				active = Some((block, String::new()));
			},
			"content_block_delta" if started && !finished => {
				if event.index != Some(blocks.len()) {
					bail!("invalid Messages API delta index");
				}
				let (block, input) = active.as_mut().context("delta outside active block")?;
				let delta = event.delta.context("missing block delta")?;
				match (block.kind.as_str(), delta.kind.as_deref()) {
					("text", Some("text_delta")) => {
						let text = delta.text.context("missing text delta")?;
						block.text.as_mut().context("missing text")?.push_str(&text);
						on_text(&text)?;
					},
					("tool_use", Some("input_json_delta")) => {
						input.push_str(&delta.partial_json.context("missing tool input delta")?);
					},
					_ => bail!("incompatible Messages API delta"),
				}
			},
			"content_block_stop" if started && !finished => {
				if event.index != Some(blocks.len()) {
					bail!("invalid Messages API stop index");
				}
				let (mut block, input) = active.take().context("block stopped without start")?;
				if !input.is_empty() {
					block.input =
						Some(serde_json::from_str(&input).context("incomplete Messages API tool input")?);
				}
				blocks.push(block);
			},
			"message_delta" if started && !finished && active.is_none() => {
				reason = event.delta.context("missing message delta")?.stop_reason;
				merge_usage(&mut usage, event.usage)?;
				finished = true;
			},
			"message_stop" if started && finished && active.is_none() => {
				cancellation.heartbeat()?;
				let raw = (!usage.is_empty())
					.then(|| serde_json::to_string(&usage).and_then(RawValue::from_string))
					.transpose()?;
				return assemble(blocks, reason.as_deref(), raw);
			},
			_ => bail!("Messages API stream error or invalid event order"),
		}
	}
	bail!("Messages API stream ended without message_stop")
}

#[cfg(test)]
mod tests {
	use super::*;
	fn event(value: Value) -> String {
		format!("event: ignored\r\ndata: {value}\r\n\r\n")
	}
	fn source() -> Vec<Value> {
		vec![
			json!({"type":"message_start","message":{"type":"message","role":"assistant","content":[],"usage":{"input_tokens":10,"output_tokens":0,"cache_read_input_tokens":7}}}),
			json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
			json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"héllo"}}),
			json!({"type":"content_block_stop","index":0}),
			json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"c1","name":"read","input":{}}}),
			json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}),
			json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"a\"}"}}),
			json!({"type":"content_block_stop","index":1}),
			json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),
			json!({"type":"message_stop"}),
		]
	}
	fn encoded(values: &[Value]) -> String {
		values.iter().map(|v| event(v.clone())).collect()
	}
	#[test]
	fn stream_text_tools_and_split_usage_replay_as_a_complete_exchange() -> Result<()> {
		let mut deltas = Vec::new();
		let result =
			decode_stream(encoded(&source()).as_bytes(), &CancelToken::default(), &mut |text| {
				deltas.push(text.to_owned());
				Ok(())
			})?;
		assert_eq!(deltas, ["héllo"]);
		let tokens = result.usage.unwrap().tokens.unwrap();
		assert_eq!(
			(tokens.input, tokens.output, tokens.cache_read, tokens.total_tokens),
			(10, 3, 7, 20)
		);
		let messages = vec![
			crate::chat::system("policy"),
			crate::chat::user("question"),
			result.message,
			crate::chat::tool_result("c1", "result".into()),
		];
		let settings = Settings {
			model:     "fixture".into(),
			base_url:  "unused".into(),
			api_key:   String::new(),
			max_steps: 2,
		};
		let request: Value = serde_json::from_slice(&request_body(
			&settings,
			42,
			&messages,
			&crate::chat::specs(),
			true,
		)?)?;
		assert_eq!(request["max_tokens"], 42);
		assert_eq!(request["system"][0]["text"], "policy");
		assert_eq!(
			request["messages"][1]["content"][1],
			json!({"type":"tool_use","id":"c1","name":"read","input":{"path":"a"}})
		);
		assert_eq!(
			request["messages"][2]["content"][0],
			json!({"type":"tool_result","tool_use_id":"c1","content":"result"})
		);
		assert!(request["tools"][0]["input_schema"].is_object());
		assert!(request["messages"][1].get("usage").is_none());
		Ok(())
	}
	#[test]
	fn unsuccessful_truncated_duplicate_and_malformed_streams_never_commit() {
		let baseline = source();
		let mut variants = vec![baseline[..9].to_vec(), baseline[1..].to_vec()];
		for (index, value) in [
			(
				2,
				json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"bad"}}),
			),
			(3, json!({"type":"message_stop"})),
			(8, json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}})),
			(
				6,
				json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"bad"}}),
			),
			(
				4,
				json!({"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":"private"}}),
			),
			(8, json!({"type":"error","error":{"message":"SECRET"}})),
		] {
			let mut changed = baseline.clone();
			changed[index] = value;
			variants.push(changed);
		}
		let mut duplicate = baseline.clone();
		duplicate.insert(1, baseline[0].clone());
		variants.push(duplicate);
		for values in variants {
			let error =
				decode_stream(encoded(&values).as_bytes(), &CancelToken::default(), &mut |_| Ok(()))
					.unwrap_err();
			assert!(!format!("{error:#}").contains("SECRET"));
		}
	}
	#[test]
	fn cancellation_and_callback_failure_stop_before_commit() {
		let mut token = CancelToken::default();
		let abort = token.emplace_abort_token();
		assert!(
			decode_stream(encoded(&source()).as_bytes(), &token, &mut |_| {
				abort.abort(harvest_runtime::cancel::AbortReason::User);
				Ok(())
			})
			.is_err()
		);
		assert!(
			decode_stream(encoded(&source()).as_bytes(), &CancelToken::default(), &mut |_| bail!(
				"consumer closed"
			))
			.is_err()
		);
	}
	#[test]
	fn usage_retains_unknown_numbers_without_claiming_partial_or_overflowed_counts() -> Result<()> {
		for raw in [
			r#"{"input_tokens":1,"future":1e400}"#,
			r#"{"input_tokens":18446744073709551615,"output_tokens":1}"#,
		] {
			let report = UsageReport::from_anthropic_raw(RawValue::from_string(raw.into())?);
			assert!(report.tokens.is_none());
			assert_eq!(report.raw.get(), raw);
		}
		let raw =
			r#"{"input_tokens":1,"output_tokens":2,"cache_creation_input_tokens":3,"future":1e400}"#;
		let report = UsageReport::from_anthropic_raw(RawValue::from_string(raw.into())?);
		assert_eq!(report.tokens.unwrap().total_tokens, 6);
		Ok(())
	}
	#[test]
	fn orphan_results_and_unresolved_calls_fail_before_network() {
		let settings = Settings {
			model:     "fixture".into(),
			base_url:  "unused".into(),
			api_key:   String::new(),
			max_steps: 2,
		};
		assert!(
			request_body(&settings, 1, &[crate::chat::tool_result("missing", "x".into())], &[], true)
				.is_err()
		);
		let message = ChatMessage {
			role:         "assistant".into(),
			content:      None,
			tool_calls:   Some(vec![ToolCall {
				id:       "c".into(),
				kind:     "function".into(),
				function: FunctionCall { name: "read".into(), arguments: "{}".into() },
			}]),
			tool_call_id: None,
		};
		assert!(request_body(&settings, 1, &[message], &[], true).is_err());
	}
	#[test]
	fn oversized_stream_and_unsupported_json_are_rejected() {
		let oversized = format!("data: {}\n\n", "x".repeat(harvest_runtime::MAX_EVENT_BYTES));
		assert!(
			decode_stream(oversized.as_bytes(), &CancelToken::default(), &mut |_| Ok(())).is_err()
		);
		let body = r#"{"type":"message","role":"assistant","content":[{"type":"thinking"}],"stop_reason":"end_turn"}"#;
		assert!(decode_json(body.as_bytes()).is_err());
	}
}
