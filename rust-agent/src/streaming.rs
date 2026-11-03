//! Bounded OpenAI-compatible SSE assembly. Partial output is never durable.
use std::{collections::BTreeMap, io::Read};

use anyhow::{Context, Result, bail};
use harvest_runtime::cancel::CancelToken;
use serde::Deserialize;
use serde_json::{Value, value::RawValue};

use crate::{
	chat::{ChatMessage, FunctionCall, ToolCall},
	provider::ModelResponse,
	usage::UsageReport,
};

#[derive(Deserialize)]
struct StreamChunk {
	choices: Option<Vec<Value>>,
	usage:   Option<Box<RawValue>>,
	error:   Option<Box<RawValue>>,
}

#[cfg(test)]
pub fn decode(
	reader: impl Read,
	cancellation: &CancelToken,
	on_text: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<ChatMessage> {
	Ok(decode_response(reader, cancellation, on_text)?.message)
}

pub fn decode_response(
	reader: impl Read,
	cancellation: &CancelToken,
	on_text: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<ModelResponse> {
	let mut events = crate::sse::Events::new(reader);
	let mut content = String::new();
	let mut calls: BTreeMap<usize, ToolCall> = BTreeMap::new();
	let mut finished = false;
	let mut usage = None;
	loop {
		let event = events
			.next(cancellation)?
			.context("stream ended without DONE")?;
		if event == "[DONE]" {
			if !finished {
				bail!("stream has no successful finish reason");
			}
			let mut ids = std::collections::BTreeSet::new();
			for call in calls.values() {
				if call.id.is_empty() || call.function.name.is_empty() || !ids.insert(&call.id) {
					bail!("invalid or duplicate streamed tool identity");
				}
				let args: Value = serde_json::from_str(&call.function.arguments)
					.context("incomplete streamed tool arguments")?;
				if !args.is_object() {
					bail!("tool arguments must be an object");
				}
			}
			return Ok(ModelResponse {
				message: ChatMessage {
					role:         "assistant".into(),
					content:      (!content.is_empty()).then_some(content),
					tool_calls:   (!calls.is_empty()).then(|| calls.into_values().collect()),
					tool_call_id: None,
				},
				usage,
			});
		}
		let chunk: StreamChunk = serde_json::from_str(&event).context("invalid stream event JSON")?;
		if chunk.error.is_some() {
			bail!("provider returned a stream error");
		}
		if let Some(raw) = chunk.usage {
			usage = Some(UsageReport::from_raw(raw));
		}
		let choices = chunk.choices.context("stream missing choices")?;
		for choice in choices {
			if choice["index"].as_u64() != Some(0) {
				bail!("unsupported stream choice index");
			}
			if finished {
				bail!("stream choice continued after finish");
			}
			let delta = &choice["delta"];
			if let Some(role) = delta.get("role").and_then(Value::as_str)
				&& role != "assistant"
			{
				bail!("invalid streamed role");
			}
			if let Some(text) = delta.get("content").and_then(Value::as_str) {
				content.push_str(text);
				on_text(text)?;
			}
			merge_tools(delta, &mut calls)?;
			if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
				if !matches!(reason, "stop" | "tool_calls") {
					bail!("stream stopped with an unsuccessful finish reason");
				}
				finished = true;
			}
		}
	}
}

fn merge_tools(delta: &Value, calls: &mut BTreeMap<usize, ToolCall>) -> Result<()> {
	if let Some(fragments) = delta.get("tool_calls") {
		for fragment in fragments.as_array().context("invalid tool delta")? {
			let index = fragment["index"]
				.as_u64()
				.context("missing tool delta index")?;
			if index >= 32 {
				bail!("too many streamed tools");
			}
			let call = calls
				.entry(usize::try_from(index)?)
				.or_insert_with(|| ToolCall {
					id:       String::new(),
					kind:     "function".into(),
					function: FunctionCall { name: String::new(), arguments: String::new() },
				});
			if fragment
				.get("type")
				.and_then(Value::as_str)
				.is_some_and(|kind| kind != "function")
			{
				bail!("unsupported tool type");
			}
			for (value, output) in [
				(&fragment["id"], &mut call.id),
				(&fragment["function"]["name"], &mut call.function.name),
				(&fragment["function"]["arguments"], &mut call.function.arguments),
			] {
				if let Some(text) = value.as_str() {
					output.push_str(text);
				}
			}
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use harvest_runtime::MAX_EVENT_BYTES;
	use serde_json::json;

	use super::*;
	fn frame(delta: Value, reason: Value) -> String {
		let value = json!({"choices":[{"index":0,"delta":delta,"finish_reason":reason}]});
		format!("data: {value}\n\n")
	}
	#[test]
	fn fragmented_sse_reassembles_text_and_tool_arguments() -> Result<()> {
		let mut source = String::from(": heartbeat\r\n\r\n");
		source.push_str(&frame(json!({"role":"assistant","content":"hé","tool_calls":[{"index":0,"id":"c","function":{"name":"write","arguments":"{\"path\":"}}]}), Value::Null));
		source.push_str(&frame(json!({"content":"llo","tool_calls":[{"index":0,"function":{"arguments":"\"a\",\"contents\":\"ok\"}"}}]}), json!("tool_calls")));
		source.push_str("data: {\"choices\":[],\"usage\":{\"total_tokens\":7}}\n\ndata: [DONE]\n\n");
		let mut deltas = Vec::new();
		let message = decode(source.as_bytes(), &Default::default(), &mut |text| {
			deltas.push(text.to_owned());
			Ok(())
		})?;
		assert_eq!(deltas, ["hé", "llo"]);
		assert_eq!(message.content.as_deref(), Some("héllo"));
		let call = &message.tool_calls.unwrap()[0];
		assert_eq!(call.id, "c");
		assert_eq!(
			serde_json::from_str::<Value>(&call.function.arguments)?,
			json!({"path":"a", "contents":"ok"})
		);
		Ok(())
	}
	#[test]
	fn truncated_and_unsuccessful_streams_never_return_a_committable_message() {
		for source in [
			frame(json!({"content":"partial"}), Value::Null),
			String::from("data: [DONE]\n\n"),
			frame(json!({}), json!("length")),
			String::from("data: {\"error\":{\"message\":\"bad\"}}\n\n"),
		] {
			assert!(decode(source.as_bytes(), &Default::default(), &mut |_| Ok(())).is_err());
		}
	}
	#[test]
	fn oversized_stream_frame_and_invalid_tools_are_rejected() {
		let oversized = format!("data: {}\n\n", "x".repeat(MAX_EVENT_BYTES));
		assert!(
			decode(oversized.as_bytes(), &Default::default(), &mut |_| Ok(()))
				.unwrap_err()
				.to_string()
				.contains("frame limit")
		);
		let mut source = frame(
			json!({"tool_calls":[{"index":0,"id":"c","function":{"name":"write","arguments":"{"}}]}),
			json!("tool_calls"),
		);
		source.push_str("data: [DONE]\n\n");
		assert!(
			decode(source.as_bytes(), &Default::default(), &mut |_| Ok(()))
				.unwrap_err()
				.to_string()
				.contains("incomplete streamed tool")
		);
	}
	#[test]
	fn cancellation_after_a_delta_stops_otherwise_complete_usage_assembly() -> Result<()> {
		let mut token = CancelToken::default();
		let abort = token.emplace_abort_token();
		let mut source = frame(json!({"content":"partial"}), json!("stop"));
		source.push_str("data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n");
		let complete = decode_response(source.as_bytes(), &Default::default(), &mut |_| Ok(()))?;
		assert_eq!(complete.usage.unwrap().tokens.unwrap().total_tokens, 12);
		let error = decode_response(source.as_bytes(), &token, &mut |_| {
			abort.abort(harvest_runtime::cancel::AbortReason::User);
			Ok(())
		})
		.unwrap_err();
		assert!(error.to_string().contains("Aborted: User"));
		Ok(())
	}
}
