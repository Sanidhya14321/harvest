use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::chat::{ChatMessage, ToolSpec};
use crate::config::Settings;

pub(crate) trait Model {
    fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage>;
}

#[derive(Debug, Clone)]
pub(crate) struct HttpModel {
    settings: Settings,
}

impl HttpModel {
    pub(crate) const fn new(settings: Settings) -> Self {
        Self { settings }
    }
}

#[derive(Debug, Serialize)]
struct CompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    tools: &'a [ToolSpec],
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
    fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
        let url = endpoint(&self.settings.base_url);
        let request = CompletionRequest {
            model: &self.settings.model,
            messages,
            tools,
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build();
        let mut call = agent.post(&url);
        if !self.settings.api_key.is_empty() {
            let header = format!("Bearer {}", self.settings.api_key);
            call = call.set("Authorization", &header);
        }
        let response = call
            .send_json(request)
            .with_context(|| format!("model request to {url} failed"))?;
        let parsed: CompletionResponse = response
            .into_json()
            .context("model response was not valid JSON")?;
        let Some(choice) = parsed.choices.into_iter().next() else {
            bail!("model response did not include a choice");
        };
        Ok(choice.message)
    }
}

fn endpoint(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::{HttpModel, Model, endpoint};
    use crate::chat::user;
    use crate::config::Settings;

    #[test]
    fn endpoint_joins_the_chat_path() {
        assert_eq!(
            endpoint("http://127.0.0.1:9/v1/"),
            "http://127.0.0.1:9/v1/chat/completions"
        );
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
                let Some(header_end) = received.windows(4).position(|mark| mark == b"\r\n\r\n")
                else {
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
            model: "test".to_owned(),
            base_url: format!("http://{address}/v1"),
            api_key: String::new(),
            max_steps: 1,
        });
        let message = model.complete(&[user("hi")], &[]).unwrap();
        assert_eq!(message.content.as_deref(), Some("done"));
        server.join().unwrap();
    }
}
