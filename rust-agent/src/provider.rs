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

    #[test]
    fn http_model_reads_a_local_completion() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let _read = socket.read(&mut buffer).unwrap();
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
