use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
	chat::{ChatMessage, ToolSpec},
	config::Settings,
};

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
}

#[derive(Debug, Clone)]
pub struct HttpModel {
	settings: Settings,
}

impl HttpModel {
	pub const fn new(settings: Settings) -> Self {
		Self { settings }
	}
}

#[derive(Debug, Serialize)]
struct CompletionRequest<'a> {
	model:    &'a str,
	messages: &'a [ChatMessage],
	tools:    &'a [ToolSpec],
	stream:   bool,
}

#[derive(Debug, Deserialize)]
struct CompletionResponse {
	choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
	message: ChatMessage,
}

impl Model for HttpModel {
	fn model_id(&self) -> &str {
		&self.settings.model
	}

	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
		let url = endpoint(&self.settings.base_url);

		let request =
			CompletionRequest { model: &self.settings.model, messages, tools, stream: false };
		let response = crate::transport::Response::open(
			url,
			self.settings.api_key.clone(),
			serde_json::to_vec(&request)?,
			false,
			&harvest_runtime::cancel::CancelToken::default(),
		)?;
		let bytes = harvest_runtime::read_bounded(response, harvest_runtime::MAX_HTTP_BYTES)?;
		let parsed: CompletionResponse =
			serde_json::from_slice(&bytes).context("model response was not valid JSON")?;
		let Some(choice) = parsed.choices.into_iter().next() else {
			bail!("model response did not include a choice");
		};
		Ok(choice.message)
	}

	fn stream(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
		cancellation: &harvest_runtime::cancel::CancelToken,
		on_text: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ChatMessage> {
		cancellation.heartbeat()?;
		let url = endpoint(&self.settings.base_url);
		let request =
			CompletionRequest { model: &self.settings.model, messages, tools, stream: true };
		let response = crate::transport::Response::open(
			url,
			self.settings.api_key.clone(),
			serde_json::to_vec(&request)?,
			true,
			cancellation,
		)?;
		if response.sse {
			crate::streaming::decode(response, cancellation, on_text)
		} else {
			let bytes = harvest_runtime::read_bounded(response, harvest_runtime::MAX_HTTP_BYTES)?;
			let response: CompletionResponse = serde_json::from_slice(&bytes)?;
			let message = response
				.choices
				.into_iter()
				.next()
				.context("model response did not include a choice")?
				.message;
			cancellation.heartbeat()?;
			if let Some(text) = &message.content {
				on_text(text)?;
			}
			Ok(message)
		}
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
			let body = r#"{"choices":[{"message":{"role":"assistant","content":"done"}}]}"#;
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
		let message = model.complete(&[user("hi")], &[]).unwrap();
		assert_eq!(message.content.as_deref(), Some("done"));
		server.join().unwrap();
	}
}
