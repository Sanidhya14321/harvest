use std::{fmt, path::Path, sync::Arc};

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
	/// Invalidate command-backed credentials after a host-classified auth
	/// failure. Defaults to no refresh for SDK models; never implies
	/// OAuth/account rotation.
	fn invalidate_command_credentials(
		&self,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<()> {
		cancellation.heartbeat()
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
	settings:       Settings,
	request_usage:  bool,
	message_tokens: Option<u32>,
	values:         Option<Arc<LiveValues>>,
}

struct LiveValues {
	sources:  crate::configuration::ProviderValues,
	resolver: crate::configuration::ValueResolver,
}
impl fmt::Debug for HttpModel {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("HttpModel")
			.field("model", &self.settings.model)
			.field("request_usage", &self.request_usage)
			.field("anthropic_messages", &self.message_tokens.is_some())
			.finish_non_exhaustive()
	}
}

/// Stable error categories; URLs, headers and server reason text are excluded.
#[derive(Debug)]
pub enum ProviderRequestError {
	InvalidRequest,
	InvalidResponse,
	Status(u16),
	Transport(ureq::ErrorKind),
}
impl fmt::Display for ProviderRequestError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidRequest => formatter.write_str("invalid model request configuration"),
			Self::InvalidResponse => formatter.write_str("invalid model response encoding"),
			Self::Status(code) => write!(formatter, "model request failed with HTTP {code}"),
			Self::Transport(kind) => write!(formatter, "model transport failed ({kind:?})"),
		}
	}
}
impl std::error::Error for ProviderRequestError {}

impl HttpModel {
	pub const fn new(settings: Settings) -> Self {
		Self { settings, request_usage: false, message_tokens: None, values: None }
	}

	/// Shared native CLI/SDK builder: retain configured value sources, explicit
	/// wire API and request accounting options instead of reducing to Settings.
	pub fn from_configuration(
		root: &Path,
		resolved: &crate::configuration::Resolved,
		request_usage: bool,
		allow_commands: bool,
	) -> Result<Self> {
		let model = Self::new(resolved.settings.clone())
			.request_usage(request_usage)
			.with_config_values(root, resolved.provider_values.clone(), allow_commands)?;
		if resolved.api == crate::configuration::Api::AnthropicMessages {
			model.anthropic_messages(resolved.max_output_tokens)
		} else {
			Ok(model)
		}
	}

	/// Interpret file credential/header sources at request time. Commands
	/// require explicit host opt-in; plain Settings API keys remain literal by
	/// default.
	pub fn with_config_values(
		mut self,
		root: &Path,
		values: crate::configuration::ProviderValues,
		allow_commands: bool,
	) -> Result<Self> {
		values.validate()?;
		self.values = if values.is_empty() {
			None
		} else {
			Some(Arc::new(LiveValues {
				sources:  values,
				resolver: crate::configuration::ValueResolver::new(root, allow_commands)?,
			}))
		};
		Ok(self)
	}

	/// Auth-retry owners can invalidate ALL command-backed secrets,
	/// including headers; invalidation itself never runs a command or request.
	pub fn invalidate_config_commands(
		&self,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<()> {
		if let Some(values) = &self.values {
			if let Some(key) = &values.sources.api_key {
				values.resolver.invalidate(key, cancellation)?;
			}
			for source in values.sources.headers.values() {
				values.resolver.invalidate(source, cancellation)?;
			}
		}
		Ok(())
	}

	fn headers(
		&self,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<Vec<(String, String)>> {
		let Some(values) = &self.values else {
			return Ok(if self.message_tokens.is_some() {
				vec![
					("x-api-key".into(), self.settings.api_key.clone()),
					("anthropic-version".into(), "2023-06-01".into()),
				]
			} else if self.settings.api_key.is_empty() {
				Vec::new()
			} else {
				vec![("Authorization".into(), format!("Bearer {}", self.settings.api_key))]
			});
		};
		let mut headers = values
			.resolver
			.headers(&[&values.sources.headers], cancellation)?;
		let key = match &values.sources.api_key {
			Some(source) => values
				.resolver
				.resolve(source, false, cancellation)?
				.ok_or_else(|| anyhow::anyhow!("configured provider credential is unavailable"))?,
			None => self.settings.api_key.clone(),
		};
		if self.message_tokens.is_some() {
			headers
				.entry("anthropic-version".into())
				.or_insert_with(|| "2023-06-01".into());
			if !key.is_empty() {
				headers.insert("x-api-key".into(), key);
			}
		} else if !key.is_empty() {
			headers.insert("authorization".into(), format!("Bearer {key}"));
		}
		crate::configuration::values::check_headers(&headers)?;
		Ok(headers.into_iter().collect())
	}

	/// Select the Anthropic Messages wire API explicitly. Endpoint/model policy
	/// remains the caller's responsibility until catalog resolution is ported.
	pub fn anthropic_messages(mut self, max_output_tokens: u32) -> Result<Self> {
		if max_output_tokens == 0 {
			bail!("max output tokens must be at least 1");
		}
		self.message_tokens = Some(max_output_tokens);
		Ok(self)
	}

	/// Explicit opt-in until compatible-host request policy is catalog-resolved.
	#[must_use]
	pub const fn request_usage(mut self, enabled: bool) -> Self {
		self.request_usage = enabled;
		self
	}

	fn request(
		&self,
		request: CompletionRequest<'_>,
		streaming: bool,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<crate::transport::Response> {
		crate::transport::Response::open(
			endpoint(&self.settings.base_url),
			self.headers(cancellation)?,
			serde_json::to_vec(&request)?,
			streaming,
			cancellation,
		)
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
	fn invalidate_command_credentials(
		&self,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<()> {
		self.invalidate_config_commands(cancellation)
	}

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
		if let Some(tokens) = self.message_tokens {
			return crate::anthropic::complete(
				&self.settings,
				tokens,
				messages,
				tools,
				self.headers(&Default::default())?,
			);
		}
		let request = CompletionRequest {
			model: &self.settings.model,
			messages,
			tools,
			stream: false,
			stream_options: None,
		};
		let response =
			self.request(request, false, &harvest_runtime::cancel::CancelToken::default())?;
		let bytes = harvest_runtime::read_bounded(response, harvest_runtime::MAX_HTTP_BYTES)?;
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
		if let Some(tokens) = self.message_tokens {
			return crate::anthropic::stream(
				&self.settings,
				tokens,
				messages,
				tools,
				cancellation,
				on_text,
				self.headers(cancellation)?,
			);
		}
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
			cancellation,
		)?;
		if response.sse {
			crate::streaming::decode_response(response, cancellation, on_text)
		} else {
			let bytes = harvest_runtime::read_bounded(response, harvest_runtime::MAX_HTTP_BYTES)?;
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
