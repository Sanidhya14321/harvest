use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::{MAX_HTTP_BYTES, cancel::CancelToken};
use serde_json::Value;

use super::configuration::{Configuration, Transport as ConfigurationTransport};
use crate::{process_transport::Process, transport::Response};

pub(crate) enum Transport {
	Stdio(Process<Value>),
	Http(Http),
}
pub(crate) struct Http {
	url:        String,
	headers:    Vec<(String, String)>,
	session_id: Option<String>,
	version:    Option<String>,
}
pub(crate) enum Incoming {
	Json(Option<Value>),
	Sse(crate::sse::Events<Response>),
}
impl Incoming {
	pub fn next(&mut self, cancellation: &CancelToken) -> Result<Option<Value>> {
		cancellation.heartbeat()?;
		match self {
			Self::Json(value) => Ok(value.take()),
			Self::Sse(events) => events
				.next(cancellation)?
				.map(|value| {
					serde_json::from_str(&value).map_err(|_| anyhow::anyhow!("invalid MCP SSE message"))
				})
				.transpose(),
		}
	}
}
impl Transport {
	pub fn connect(
		root: &Path,
		configuration: &Configuration,
		cancellation: &CancelToken,
	) -> Result<Self> {
		match &configuration.transport {
			ConfigurationTransport::Stdio { command, args, env, cwd } => {
				let cwd = cwd
					.as_ref()
					.map(|path| {
						if path.is_absolute() {
							path.clone()
						} else {
							root.join(path)
						}
					})
					.unwrap_or_else(|| root.to_path_buf())
					.canonicalize()
					.context("cannot resolve MCP process directory")?;
				// Bare program names use PATH; relative paths are rooted explicitly,
				// never interpolated into shell command text.
				let command = if command.components().count() > 1 && command.is_relative() {
					root.join(command)
				} else {
					command.clone()
				};
				Ok(Self::Stdio(Process::launch(&cwd, &command, args, env, cancellation)?))
			},
			ConfigurationTransport::Http { url, headers } => {
				let parsed =
					reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("invalid MCP endpoint URL"))?;
				ensure!(
					matches!(parsed.scheme(), "http" | "https") && parsed.fragment().is_none(),
					"unsupported MCP endpoint URL"
				);
				validate_headers(headers)?;
				Ok(Self::Http(Http {
					url:        url.clone(),
					headers:    headers
						.iter()
						.map(|(name, value)| (name.clone(), value.clone()))
						.collect(),
					session_id: None,
					version:    None,
				}))
			},
			ConfigurationTransport::Sse { .. } => bail!(
				"legacy MCP SSE GET/endpoint transport is not converted; use a streamable HTTP \
				 endpoint or native stdio server"
			),
		}
	}

	pub fn start(
		&mut self,
		message: &Value,
		cancellation: &CancelToken,
	) -> Result<Option<Incoming>> {
		match self {
			Self::Stdio(process) => {
				process.send(message, cancellation)?;
				Ok(None)
			},
			Self::Http(http) => {
				let response = http.post(message, cancellation)?;
				if response.sse {
					Ok(Some(Incoming::Sse(crate::sse::Events::new(response))))
				} else {
					let bytes = harvest_runtime::read_bounded(response, MAX_HTTP_BYTES)?;
					let value = serde_json::from_slice(&bytes)
						.map_err(|_| anyhow::anyhow!("invalid MCP HTTP JSON response"))?;
					Ok(Some(Incoming::Json(Some(value))))
				}
			},
		}
	}

	pub fn notify(&mut self, message: &Value, cancellation: &CancelToken) -> Result<()> {
		match self {
			Self::Stdio(process) => process.send(message, cancellation),
			Self::Http(http) => {
				let response = http.post(message, cancellation)?;
				let _ = harvest_runtime::read_bounded(response, MAX_HTTP_BYTES)?;
				Ok(())
			},
		}
	}

	pub fn receive(&mut self, cancellation: &CancelToken) -> Result<Value> {
		match self {
			Self::Stdio(process) => process.receive(cancellation),
			Self::Http(_) => bail!("MCP HTTP response stream is missing"),
		}
	}

	pub fn set_version(&mut self, version: &str) {
		if let Self::Http(http) = self {
			http.version = Some(version.into());
		}
	}

	pub fn cancel(&mut self, id: &Value) {
		let message = serde_json::json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"Host request cancelled"}});
		match self {
			Self::Stdio(process) => process.try_send(&message),
			Self::Http(http) if http.session_id.is_some() => {
				let token = CancelToken::new(Some(100));
				if let Ok(response) = http.post(&message, &token) {
					let _ = harvest_runtime::read_bounded(response, MAX_HTTP_BYTES);
				}
			},
			Self::Http(_) => {},
		}
	}

	pub fn close(&mut self) {
		match self {
			Self::Stdio(process) => process.terminate(),
			Self::Http(http) => http.close(),
		}
	}

	pub fn stderr_tail(&self) -> Vec<u8> {
		match self {
			Self::Stdio(process) => process.stderr_tail(),
			Self::Http(_) => Vec::new(),
		}
	}
}
impl Http {
	fn close(&mut self) {
		if self.session_id.is_none() {
			return;
		}
		let token = CancelToken::new(Some(100));
		if let Ok(response) = self.send(reqwest::Method::DELETE, Vec::new(), &token) {
			let _ = harvest_runtime::read_bounded(response, MAX_HTTP_BYTES);
		}
		self.session_id = None;
	}

	fn post(&mut self, message: &Value, cancellation: &CancelToken) -> Result<Response> {
		self.send(reqwest::Method::POST, serde_json::to_vec(message)?, cancellation)
	}

	fn send(
		&mut self,
		method: reqwest::Method,
		body: Vec<u8>,
		cancellation: &CancelToken,
	) -> Result<Response> {
		let mut headers = self.headers.clone();
		headers.push(("Accept".into(), "application/json, text/event-stream".into()));
		if let Some(id) = &self.session_id {
			headers.push(("Mcp-Session-Id".into(), id.clone()));
		}
		if let Some(version) = &self.version {
			headers.push(("Mcp-Protocol-Version".into(), version.clone()));
		}
		let response =
			Response::open_with_method(method, self.url.clone(), headers, body, false, cancellation)
				.map_err(|error| {
				if let Some(crate::provider::ProviderRequestError::Status(status)) =
					error.downcast_ref::<crate::provider::ProviderRequestError>()
				{
					anyhow::anyhow!("MCP HTTP request failed with HTTP {status}")
				} else {
					anyhow::anyhow!("MCP HTTP transport failed or was interrupted")
				}
			})?;
		if let Some(id) = &response.session_id {
			if let Some(current) = &self.session_id {
				ensure!(current == id, "MCP HTTP session identity changed unexpectedly");
			} else {
				self.session_id = Some(id.clone());
			}
		}
		Ok(response)
	}
}
fn validate_headers(headers: &BTreeMap<String, String>) -> Result<()> {
	ensure!(headers.len() <= 64, "too many MCP HTTP headers");
	for (name, value) in headers {
		let lower = name.to_ascii_lowercase();
		ensure!(
			!matches!(
				lower.as_str(),
				"host"
					| "content-length"
					| "content-type"
					| "transfer-encoding"
					| "connection"
					| "accept" | "mcp-session-id"
					| "mcp-protocol-version"
			) && !lower.starts_with("proxy-"),
			"MCP configuration overrides an owned or hop-by-hop HTTP header"
		);
		ensure!(
			name.len() <= 128 && value.len() <= 8192 && !value.contains(['\r', '\n']),
			"invalid MCP HTTP header"
		);
	}
	Ok(())
}
