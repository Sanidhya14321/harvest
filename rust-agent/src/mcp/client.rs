use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	sync::atomic::{AtomicU64, Ordering},
	time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::cancel::CancelToken;
use serde_json::{Value, json};

use super::{
	PROTOCOL_VERSION,
	configuration::{Configuration, RequestIdFormat},
	transport::Transport,
};
use crate::{
	chat::{ToolCall, ToolSpec},
	tools::Approval,
};

static CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
const MAX_ID: u64 = 9_007_199_254_740_991;
const MAX_MESSAGES: usize = 1024;
const MAX_EXCHANGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ServerInfo {
	pub name:             String,
	pub version:          String,
	pub protocol_version: String,
	pub capabilities:     Value,
}
#[derive(Debug, Clone)]
pub struct ToolRegistration {
	pub spec:          ToolSpec,
	pub original_name: String,
	pub definition:    Value,
}
pub struct Client {
	name:          String,
	root:          PathBuf,
	transport:     Transport,
	info:          ServerInfo,
	tools:         Vec<ToolRegistration>,
	next_id:       u64,
	connection_id: u64,
	id_format:     RequestIdFormat,
	timeout:       Duration,
	closed:        bool,
	tools_changed: bool,
}
impl Client {
	pub fn connect(
		root: &Path,
		configuration: &Configuration,
		cancellation: &CancelToken,
	) -> Result<Self> {
		configuration.validate()?;
		ensure!(configuration.enabled, "MCP server is disabled");
		let root = root
			.canonicalize()
			.context("cannot resolve MCP workspace")?;
		let timeout = Duration::from_millis(u64::from(configuration.timeout_ms));
		let deadline = cancellation.child_with_timeout(timeout);
		let transport = Transport::connect(&root, configuration, &deadline)?;
		let mut client = Self {
			name: configuration.name.clone(),
			root,
			transport,
			info: ServerInfo {
				name:             String::new(),
				version:          String::new(),
				protocol_version: String::new(),
				capabilities:     json!({}),
			},
			tools: Vec::new(),
			next_id: 1,
			connection_id: CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
			id_format: configuration.request_id_format,
			timeout,
			closed: false,
			tools_changed: false,
		};
		let result=client.request("initialize",json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"roots":{"listChanged":false}},"clientInfo":{"name":"harvest-rust","version":env!("CARGO_PKG_VERSION")}}),&deadline)?;
		let version = result["protocolVersion"]
			.as_str()
			.context("MCP initialize response has no protocol version")?;
		ensure!(
			matches!(version, "2025-11-25" | "2025-06-18" | "2025-03-26" | "2024-11-05"),
			"unsupported MCP protocol version"
		);
		let name = result["serverInfo"]["name"]
			.as_str()
			.context("MCP initialize response has no server name")?;
		let server_version = result["serverInfo"]["version"]
			.as_str()
			.context("MCP initialize response has no server version")?;
		ensure!(
			name.len() <= 256 && server_version.len() <= 256 && result["capabilities"].is_object(),
			"invalid MCP server identity or capabilities"
		);
		client.info = ServerInfo {
			name:             name.into(),
			version:          server_version.into(),
			protocol_version: version.into(),
			capabilities:     result["capabilities"].clone(),
		};
		client.transport.set_version(version);
		client
			.transport
			.notify(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}), &deadline)?;
		if client.info.capabilities.get("tools").is_some() {
			client.refresh_tools(&deadline)?;
		}
		Ok(client)
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn info(&self) -> &ServerInfo {
		&self.info
	}

	pub fn tools(&self) -> &[ToolRegistration] {
		&self.tools
	}

	pub fn tools_changed(&self) -> bool {
		self.tools_changed
	}

	pub fn stderr_tail(&self) -> Vec<u8> {
		self.transport.stderr_tail()
	}

	fn identity(&mut self) -> Result<Value> {
		let id = self.next_id;
		ensure!(id <= MAX_ID, "MCP request identities exhausted");
		self.next_id = id + 1;
		Ok(match self.id_format {
			RequestIdFormat::Number => json!(id),
			RequestIdFormat::String => json!(format!("harvest-{}-{id}", self.connection_id)),
		})
	}

	pub fn request(
		&mut self,
		method: &str,
		params: Value,
		cancellation: &CancelToken,
	) -> Result<Value> {
		self.request_with_notifications(method, params, cancellation, &mut |_| Ok(()))
	}

	pub fn request_with_notifications(
		&mut self,
		method: &str,
		params: Value,
		cancellation: &CancelToken,
		on_notification: &mut dyn FnMut(&Value) -> Result<()>,
	) -> Result<Value> {
		ensure!(!self.closed, "MCP connection is closed; reconnect explicitly");
		ensure!(
			!method.is_empty() && method.len() <= 256 && params.is_object(),
			"invalid MCP request method or parameters"
		);
		cancellation.heartbeat()?;
		let id = self.identity()?;
		let deadline = cancellation.child_with_timeout(self.timeout);
		let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
		let result = self.exchange(&request, &id, &deadline, on_notification);
		if let Err(error) = &result {
			// A failed/ambiguous exchange cannot reuse late frames as a later
			// response. Known structured JSON-RPC errors have no ambiguity.
			if !error.is::<RpcError>() {
				self.transport.cancel(&id);
				self.closed = true;
				self.transport.close();
			}
		}
		result
	}

	fn exchange(
		&mut self,
		request: &Value,
		id: &Value,
		cancellation: &CancelToken,
		on_notification: &mut dyn FnMut(&Value) -> Result<()>,
	) -> Result<Value> {
		let mut incoming = self.transport.start(request, cancellation)?;
		let mut total = 0;
		for _ in 0..MAX_MESSAGES {
			cancellation.heartbeat()?;
			let frame = if let Some(incoming) = &mut incoming {
				incoming
					.next(cancellation)?
					.context("MCP stream ended before its response")?
			} else {
				self.transport.receive(cancellation)?
			};
			total += serde_json::to_vec(&frame)?.len();
			ensure!(total <= MAX_EXCHANGE_BYTES, "MCP exchange exceeds response limit");
			ensure!(frame.is_object() && frame["jsonrpc"] == "2.0", "invalid MCP JSON-RPC envelope");
			if let Some(method) = frame.get("method") {
				let method = method.as_str().context("invalid MCP server method")?;
				ensure!(
					frame.get("result").is_none() && frame.get("error").is_none(),
					"ambiguous MCP server message"
				);
				if let Some(server_id) = frame.get("id") {
					validate_id(server_id)?;
					let response = self.server_request(method, server_id)?;
					self.transport.notify(&response, cancellation)?;
				} else {
					if method == "notifications/tools/list_changed" {
						self.tools_changed = true;
					}
					if method == "notifications/cancelled" && frame["params"]["requestId"] == *id {
						bail!("MCP server cancelled the active request");
					}
					on_notification(&frame)?;
				}
				continue;
			}
			ensure!(frame.get("id") == Some(id), "unexpected MCP response identity");
			match (frame.get("result"), frame.get("error")) {
				(Some(result), None) => {
					cancellation.heartbeat()?;
					return Ok(result.clone());
				},
				(None, Some(error)) => {
					let code = error["code"].as_i64().context("invalid MCP error code")?;
					return Err(RpcError(code).into());
				},
				_ => bail!("MCP response requires exactly one result or error"),
			}
		}
		bail!("MCP exchange has too many messages")
	}

	fn server_request(&self, method: &str, id: &Value) -> Result<Value> {
		let result = match method {
			"ping" => json!({}),
			"roots/list" => {
				let uri = reqwest::Url::from_directory_path(&self.root)
					.map_err(|()| anyhow::anyhow!("cannot encode MCP workspace root"))?;
				json!({"roots":[{"uri":uri.as_str(),"name":self.root.file_name().and_then(|v|v.to_str()).unwrap_or("workspace")} ]})
			},
			_ => {
				return Ok(
					json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Server request is unsupported by this host"}}),
				);
			},
		};
		Ok(json!({"jsonrpc":"2.0","id":id,"result":result}))
	}

	fn list(&mut self, method: &str, key: &str, cancellation: &CancelToken) -> Result<Vec<Value>> {
		let deadline = cancellation.child_with_timeout(self.timeout);
		let mut cursor = None::<String>;
		let mut seen = BTreeSet::new();
		let mut values = Vec::new();
		let mut bytes = 0;
		for _ in 0..64 {
			let params = cursor
				.as_ref()
				.map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
			let page = self.request(method, params, &deadline)?;
			bytes += serde_json::to_vec(&page)?.len();
			ensure!(bytes <= MAX_EXCHANGE_BYTES, "MCP paginated list exceeds limit");
			values.extend(
				page[key]
					.as_array()
					.context("MCP list has no result array")?
					.iter()
					.cloned(),
			);
			ensure!(values.len() <= 4096, "MCP list has too many entries");
			match page.get("nextCursor") {
				None | Some(Value::Null) => return Ok(values),
				Some(value) => {
					let next = value.as_str().context("invalid MCP pagination cursor")?;
					ensure!(
						!next.is_empty() && next.len() <= 4096 && seen.insert(next.to_owned()),
						"MCP pagination cursor repeats or exceeds limit"
					);
					cursor = Some(next.into());
				},
			}
		}
		bail!("MCP list exceeds pagination limit")
	}

	pub fn refresh_tools(&mut self, cancellation: &CancelToken) -> Result<()> {
		let definitions = self.list("tools/list", "tools", cancellation)?;
		let mut tools = Vec::new();
		let mut names = BTreeSet::new();
		let mut aliases = BTreeSet::new();
		for definition in definitions {
			let name = definition["name"]
				.as_str()
				.context("MCP tool has no name")?;
			ensure!(
				!name.is_empty() && name.len() <= 256 && names.insert(name.to_owned()),
				"invalid or duplicate MCP tool name"
			);
			let description = match definition.get("description") {
				None => "",
				Some(value) => value.as_str().context("invalid MCP tool description")?,
			};
			let digest =
				harvest_runtime::content_hash(&serde_json::to_vec(&[self.name.as_str(), name])?);
			let display: String = name
				.chars()
				.map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
				.take(32)
				.collect();
			let server_display: String = self
				.name
				.chars()
				.map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
				.take(16)
				.collect();
			let alias = format!("mcp_{}_{}_{}", server_display, display, &digest[..16]);
			ensure!(aliases.insert(alias.clone()), "MCP host tool alias collision");
			let spec =
				ToolSpec::from_schema(alias, description.into(), definition["inputSchema"].clone())?;
			tools.push(ToolRegistration { spec, original_name: name.into(), definition });
		}
		self.tools = tools;
		self.tools_changed = false;
		Ok(())
	}

	pub fn invoke(
		&mut self,
		call: &ToolCall,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<String> {
		let original = self
			.tools
			.iter()
			.find(|tool| tool.spec.name() == call.function.name)
			.context("MCP tool is not registered")?
			.original_name
			.clone();
		ensure!(
			call.function.arguments.len() <= crate::process_transport::MAX_FRAME_BYTES,
			"MCP tool arguments exceed limit"
		);
		let arguments: Value = serde_json::from_str(&call.function.arguments)
			.map_err(|_| anyhow::anyhow!("invalid MCP tool arguments"))?;
		ensure!(arguments.is_object(), "MCP tool arguments must be an object");
		cancellation.heartbeat()?;
		// Remote annotations/readOnlyHint never bypass host authorization.
		ensure!(approval.approve(call)?, "MCP tool requires host approval");
		cancellation.heartbeat()?;
		let result = self.call_tool(&original, arguments, cancellation)?;
		let text = serde_json::to_string(&result)?;
		ensure!(
			text.len() <= harvest_runtime::MAX_TOOL_BYTES,
			"MCP tool result exceeds host output limit"
		);
		Ok(text)
	}

	pub fn call_tool(
		&mut self,
		name: &str,
		arguments: Value,
		cancellation: &CancelToken,
	) -> Result<Value> {
		ensure!(arguments.is_object(), "MCP tool arguments must be an object");
		let result =
			self.request("tools/call", json!({"name":name,"arguments":arguments}), cancellation)?;
		ensure!(
			result["content"].is_array() && result.get("isError").is_none_or(Value::is_boolean),
			"invalid MCP tool result"
		);
		Ok(result)
	}

	pub fn resources(&mut self, cancellation: &CancelToken) -> Result<Vec<Value>> {
		self.list("resources/list", "resources", cancellation)
	}

	pub fn resource_templates(&mut self, cancellation: &CancelToken) -> Result<Vec<Value>> {
		self.list("resources/templates/list", "resourceTemplates", cancellation)
	}

	pub fn read_resource(&mut self, uri: &str, cancellation: &CancelToken) -> Result<Value> {
		self.request("resources/read", json!({"uri":uri}), cancellation)
	}

	pub fn subscribe(&mut self, uri: &str, cancellation: &CancelToken) -> Result<Value> {
		self.request("resources/subscribe", json!({"uri":uri}), cancellation)
	}

	pub fn unsubscribe(&mut self, uri: &str, cancellation: &CancelToken) -> Result<Value> {
		self.request("resources/unsubscribe", json!({"uri":uri}), cancellation)
	}

	pub fn prompts(&mut self, cancellation: &CancelToken) -> Result<Vec<Value>> {
		self.list("prompts/list", "prompts", cancellation)
	}

	pub fn get_prompt(
		&mut self,
		name: &str,
		arguments: Value,
		cancellation: &CancelToken,
	) -> Result<Value> {
		ensure!(arguments.is_object(), "MCP prompt arguments must be an object");
		self.request("prompts/get", json!({"name":name,"arguments":arguments}), cancellation)
	}
}
impl Drop for Client {
	fn drop(&mut self) {
		self.transport.close();
	}
}
#[derive(Debug)]
pub struct RpcError(pub i64);
impl std::fmt::Display for RpcError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "MCP request failed with JSON-RPC code {}", self.0)
	}
}
impl std::error::Error for RpcError {}
fn validate_id(id: &Value) -> Result<()> {
	ensure!(
		id.as_str()
			.is_some_and(|value| !value.is_empty() && value.len() <= 256)
			|| id.as_u64().is_some_and(|value| value <= MAX_ID)
			|| id
				.as_i64()
				.is_some_and(|value| value.unsigned_abs() <= MAX_ID),
		"invalid MCP server request identity"
	);
	Ok(())
}
