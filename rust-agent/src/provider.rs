use std::{fmt, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
	chat::{ChatMessage, ToolSpec},
	config::Settings,
	usage::UsageReport,
};

#[derive(Debug)]
pub struct ModelResponse {
	pub message: ChatMessage,
	pub usage:   Option<UsageReport>,
}

pub trait Model {
	// Implementations can return an ID borrowed from their configuration.
	#[allow(
		clippy::unnecessary_literal_bound,
		reason = "provider IDs can borrow runtime configuration"
	)]
	fn model_id(&self) -> &str {
		"unknown"
	}
	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage>;

	fn complete_response(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
	) -> Result<ModelResponse> {
		Ok(ModelResponse { message: self.complete(messages, tools)?, usage: None })
	}

	/// Deltas are provisional; only the returned message can be committed.
	fn stream(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
		cancellation: &harvest_runtime::cancel::CancelToken,
		on_text: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ChatMessage> {
		cancellation.heartbeat()?;
		let message = self.complete(messages, tools)?;
		cancellation.heartbeat()?;
		if let Some(text) = &message.content {
			on_text(text)?;
		}
		Ok(message)
	}
	/// Compatibility seam for SDK models that only produce messages or deltas.
	fn stream_response(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
		cancellation: &harvest_runtime::cancel::CancelToken,
		on_text: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ModelResponse> {
		Ok(ModelResponse {
			message: self.stream(messages, tools, cancellation, on_text)?,
			usage:   None,
		})
	}
}

#[derive(Clone)]
pub struct HttpModel {
	settings:      Settings,
	request_usage: bool,
}

impl fmt::Debug for HttpModel {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("HttpModel")
			.field("model", &self.settings.model)
			.field("request_usage", &self.request_usage)
			.finish_non_exhaustive()
	}
}

/// Stable error categories; URLs, headers and server reason text are excluded.
#[derive(Debug)]
pub enum ProviderRequestError {
	Status(u16),
	Transport(ureq::ErrorKind),
}
impl fmt::Display for ProviderRequestError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Status(code) => write!(formatter, "model request failed with HTTP {code}"),
			Self::Transport(kind) => write!(formatter, "model transport failed ({kind:?})"),
		}
	}
}
impl std::error::Error for ProviderRequestError {}

impl HttpModel {
	pub const fn new(settings: Settings) -> Self {
		Self { settings, request_usage: false }
	}

	/// Explicit opt-in until compatible-host request policy is catalog-resolved.
	#[must_use]
	pub const fn request_usage(mut self, enabled: bool) -> Self {
		self.request_usage = enabled;
		self
	}

	fn request(&self, request: CompletionRequest<'_>, streaming: bool) -> Result<ureq::Response> {
		let agent = ureq::AgentBuilder::new()
			.timeout(Duration::from_secs(60))
			.build();
		let mut call = agent.post(&endpoint(&self.settings.base_url));
		if streaming {
			call = call.set("Accept", "text/event-stream");
		}
		if !self.settings.api_key.is_empty() {
			call = call.set("Authorization", &format!("Bearer {}", self.settings.api_key));
		}
		call.send_json(request).map_err(|error| {
			match error {
				ureq::Error::Status(code, _) => ProviderRequestError::Status(code),
				ureq::Error::Transport(transport) => ProviderRequestError::Transport(transport.kind()),
			}
			.into()
		})
	}
}

#[derive(Debug, Serialize)]
struct CompletionRequest<'a> {
	model:          &'a str,
	messages:       &'a [ChatMessage],
	tools:          &'a [ToolSpec],
	stream:         bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
	include_usage: bool,
}

#[derive(Debug, Deserialize)]
struct CompletionResponse {
	choices: Vec<Choice>,
	usage:   Option<Box<serde_json::value::RawValue>>,
}

#[derive(Debug, Deserialize)]
struct Choice {
	message:       ChatMessage,
	finish_reason: Option<String>,
	index:         Option<u64>,
}

impl Model for HttpModel {
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
		let request = CompletionRequest {
			model: &self.settings.model,
			messages,
			tools,
			stream: false,
			stream_options: None,
		};
		let response = self.request(request, false)?;
		let bytes =
			harvest_runtime::read_bounded(response.into_reader(), harvest_runtime::MAX_HTTP_BYTES)?;
		let parsed: CompletionResponse =
			serde_json::from_slice(&bytes).context("model response was not valid JSON")?;
		parsed.into_response()
	}

	fn stream(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
		cancellation: &harvest_runtime::cancel::CancelToken,
		on_text: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ChatMessage> {
		Ok(self
			.stream_response(messages, tools, cancellation, on_text)?
			.message)
	}

	fn stream_response(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
		cancellation: &harvest_runtime::cancel::CancelToken,
		on_text: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ModelResponse> {
		cancellation.heartbeat()?;
		let response = self.request(
			CompletionRequest {
				model: &self.settings.model,
				messages,
				tools,
				stream: true,
				stream_options: self
					.request_usage
					.then_some(StreamOptions { include_usage: true }),
			},
			true,
		)?;
		if response.header("Content-Type").is_some_and(|value| {
			value
				.split(';')
				.next()
				.is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
		}) {
			crate::streaming::decode_response(response.into_reader(), cancellation, on_text)
		} else {
			let bytes =
				harvest_runtime::read_bounded(response.into_reader(), harvest_runtime::MAX_HTTP_BYTES)?;
			let response: CompletionResponse = serde_json::from_slice(&bytes)?;
			let response = response.into_response()?;
			cancellation.heartbeat()?;
			if let Some(text) = &response.message.content {
				on_text(text)?;
			}
			Ok(response)
		}
	}
}

impl CompletionResponse {
	fn into_response(self) -> Result<ModelResponse> {
		let choice = self
			.choices
			.into_iter()
			.next()
			.context("model response did not include a choice")?;
		if choice.index.is_some_and(|index| index != 0) || choice.message.role != "assistant" {
			bail!("invalid model response choice or role");
		}
		if let Some(reason) = &choice.finish_reason
			&& !matches!(reason.as_str(), "stop" | "tool_calls")
		{
			bail!("model stopped with an unsuccessful finish reason");
		}
		Ok(ModelResponse { message: choice.message, usage: self.usage.map(UsageReport::from_raw) })
	}
}

fn endpoint(base_url: &str) -> String {
	format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		net::TcpListener,
		thread,
	};

	use super::{HttpModel, Model, endpoint};
	use crate::{chat::user, config::Settings};

	#[test]
	fn endpoint_joins_the_chat_path() {
		assert_eq!(endpoint("http://127.0.0.1:9/v1/"), "http://127.0.0.1:9/v1/chat/completions");
	}

	#[cfg_attr(miri, ignore)]
	#[test]
	fn http_model_reads_a_local_completion() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let server = thread::spawn(move || {
			let (mut socket, _) = listener.accept().unwrap();
			let mut received = Vec::new();
			let mut buffer = [0_u8; 1024];
			loop {
				let count = socket.read(&mut buffer).unwrap_or(0);
				if count == 0 {
					break;
				}
				let Some(chunk) = buffer.get(..count) else {
					break;
				};
				received.extend_from_slice(chunk);
				let Some(header_end) = received.windows(4).position(|mark| mark == b"\r\n\r\n") else {
					continue;
				};
				let Some(header_bytes) = received.get(..header_end) else {
					break;
				};
				let header = String::from_utf8_lossy(header_bytes);
				let mut length = 0_usize;
				for line in header.lines() {
					let Some(value) = line
						.to_ascii_lowercase()
						.strip_prefix("content-length:")
						.map(str::to_owned)
					else {
						continue;
					};
					length = value.trim().parse().unwrap_or(0);
				}
				if received.len() >= header_end.saturating_add(4).saturating_add(length) {
					break;
				}
			}
			let body = r#"{"choices":[{"message":{"role":"assistant","content":"done"}}],"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":3}}}"#;
			let response = format!(
				"HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
				body.len()
			);
			socket.write_all(response.as_bytes()).unwrap();
		});
		let model = HttpModel::new(Settings {
			model:     "test".to_owned(),
			base_url:  format!("http://{address}/v1"),
			api_key:   String::new(),
			max_steps: 1,
		});
		let response = model.complete_response(&[user("hi")], &[]).unwrap();
		assert_eq!(response.message.content.as_deref(), Some("done"));
		let usage = response.usage.unwrap().tokens.unwrap();
		assert_eq!((usage.input, usage.cache_read, usage.total_tokens), (7, 3, 12));
		server.join().unwrap();
	}
}
