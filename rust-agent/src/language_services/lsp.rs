use std::{
	collections::{BTreeMap, VecDeque},
	path::{Path, PathBuf},
	time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use serde_json::{Value, json};

use super::{
	configuration::LspConfiguration,
	edits::{self, ApplyResult, Position},
};
use crate::{
	process_transport::Process,
	tools::{Approval, DenyMutations},
};

struct Document {
	version: i64,
	text:    String,
}
pub struct Client {
	root:          PathBuf,
	configuration: LspConfiguration,
	process:       Process<Value>,
	capabilities:  Value,
	next_id:       u64,
	closed:        bool,
	documents:     BTreeMap<String, Document>,
	diagnostics:   BTreeMap<String, Value>,
	notifications: VecDeque<Value>,
	applied_edits: Vec<Value>,
}
impl Client {
	pub fn connect(
		root: &Path,
		configuration: &LspConfiguration,
		cancellation: &CancelToken,
	) -> Result<Self> {
		configuration.validate()?;
		let root = root.canonicalize()?;
		let deadline = cancellation
			.child_with_timeout(Duration::from_millis(u64::from(configuration.process.timeout_ms)));
		let process = configuration.process.launch(&root, &deadline)?;
		let mut client = Self {
			root,
			configuration: configuration.clone(),
			process,
			capabilities: json!({}),
			next_id: 1,
			closed: false,
			documents: BTreeMap::new(),
			diagnostics: BTreeMap::new(),
			notifications: VecDeque::new(),
			applied_edits: Vec::new(),
		};
		let uri = reqwest::Url::from_directory_path(&client.root)
			.map_err(|()| anyhow::anyhow!("cannot encode LSP workspace URI"))?;
		let initialized=client.request("initialize",json!({"processId":std::process::id(),"clientInfo":{"name":"harvest-rust","version":env!("CARGO_PKG_VERSION")},"rootUri":uri.as_str(),"workspaceFolders":[{"uri":uri.as_str(),"name":"workspace"}],"capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true,"resourceOperations":[],"failureHandling":"abort"},"configuration":true,"workspaceFolders":true},"textDocument":{"synchronization":{"didSave":true},"publishDiagnostics":{"versionSupport":true},"documentSymbol":{"hierarchicalDocumentSymbolSupport":true},"rename":{"prepareSupport":true},"diagnostic":{"dynamicRegistration":false}}},"initializationOptions":configuration.initialization_options}),&DenyMutations,&deadline)?;
		ensure!(
			initialized["capabilities"].is_object(),
			"LSP initialize response has no capabilities"
		);
		let capabilities = initialized["capabilities"].clone();
		ensure!(
			capabilities
				.get("positionEncoding")
				.is_none_or(|value| value == "utf-16"),
			"unsupported LSP position encoding; UTF-16 is required"
		);
		client.capabilities = capabilities;
		client.notify("initialized", json!({}), &deadline)?;
		client.notify(
			"workspace/didChangeConfiguration",
			json!({"settings":configuration.settings}),
			&deadline,
		)?;
		Ok(client)
	}

	pub fn capabilities(&self) -> &Value {
		&self.capabilities
	}

	pub fn name(&self) -> &str {
		&self.configuration.process.name
	}

	pub fn take_notifications(&mut self) -> Vec<Value> {
		self.notifications.drain(..).collect()
	}

	pub fn take_applied_edits(&mut self) -> Vec<Value> {
		std::mem::take(&mut self.applied_edits)
	}

	pub fn stderr_tail(&self) -> Vec<u8> {
		self.process.stderr_tail()
	}

	pub fn file_uri(&self, path: &Path) -> Result<String> {
		let filesystem = JailedFs::open(&self.root)?;
		let resolved = filesystem.resource_path(path)?;
		reqwest::Url::from_file_path(resolved)
			.map(|uri| uri.to_string())
			.map_err(|()| anyhow::anyhow!("cannot encode LSP file URI"))
	}

	pub fn sync_document(&mut self, path: &Path, cancellation: &CancelToken) -> Result<String> {
		let filesystem = JailedFs::open(&self.root)?;
		let resolved = filesystem.resource_path(path)?;
		let bytes = filesystem.read(&resolved, MAX_FILE_BYTES)?;
		let text = std::str::from_utf8(&bytes)
			.map_err(|_| anyhow::anyhow!("LSP document is not UTF-8"))?
			.to_owned();
		ensure!(!text.contains('\0'), "LSP document is not text");
		let uri = self.file_uri(&resolved)?;
		let bytes = self
			.documents
			.iter()
			.filter(|(key, _)| *key != &uri)
			.map(|(_, document)| document.text.len())
			.sum::<usize>();
		ensure!(
			bytes + text.len() <= 16 * 1024 * 1024
				&& (self.documents.contains_key(&uri) || self.documents.len() < 128),
			"LSP open document cache exceeds limit"
		);
		if let Some(document) = self.documents.get(&uri) {
			if document.text == text {
				return Ok(uri);
			}
			let version = document
				.version
				.checked_add(1)
				.filter(|value| *value <= i64::from(i32::MAX))
				.context("LSP document version exhausted")?;
			let incremental = self.capabilities["textDocumentSync"]["change"] == 2
				|| self.capabilities["textDocumentSync"] == 2;
			let change = if incremental {
				json!({"range":{"start":{"line":0,"character":0},"end":edits::end_position(&document.text)},"text":text})
			} else {
				json!({"text":text})
			};
			self.notify(
				"textDocument/didChange",
				json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[change]}),
				cancellation,
			)?;
			self
				.documents
				.insert(uri.clone(), Document { version, text });
		} else {
			self.notify("textDocument/didOpen",json!({"textDocument":{"uri":uri,"version":1,"languageId":self.configuration.language_id,"text":text}}),cancellation)?;
			self
				.documents
				.insert(uri.clone(), Document { version: 1, text });
		}
		Ok(uri)
	}

	pub fn formatting_options(&self, uri: &str) -> Result<Value> {
		let filesystem = JailedFs::open(&self.root)?;
		let path = edits::uri_path(&filesystem, uri)?;
		let mut directory = path.parent();
		while let Some(parent) = directory {
			if !parent.starts_with(filesystem.root()) {
				break;
			}
			ensure!(
				!filesystem.exists(&parent.join(".editorconfig"))?,
				"native .editorconfig resolution is unfinished; provide explicit formatting options"
			);
			directory = parent.parent();
		}
		let text = &self
			.documents
			.get(uri)
			.context("document is not open")?
			.text;
		let mut spaces = None;
		let mut unit = 0_usize;
		fn gcd(mut a: usize, mut b: usize) -> usize {
			while b != 0 {
				let next = a % b;
				a = b;
				b = next;
			}
			a
		}
		for line in text.lines().filter(|line| !line.trim().is_empty()) {
			if line.starts_with('\t') {
				if spaces.is_none() {
					spaces = Some(false);
				}
			} else if line.starts_with(' ') {
				if spaces.is_none() {
					spaces = Some(true);
				}
				let count = line.bytes().take_while(|byte| *byte == b' ').count();
				unit = if unit == 0 { count } else { gcd(unit, count) };
			}
		}
		Ok(
			json!({"tabSize":if spaces==Some(true) && unit>0 { unit } else { 2 },"insertSpaces":spaces.unwrap_or(true),"trimTrailingWhitespace":true,"insertFinalNewline":true,"trimFinalNewlines":true}),
		)
	}

	pub fn position(
		&self,
		uri: &str,
		line: u32,
		character: Option<u32>,
		symbol: Option<&str>,
	) -> Result<Position> {
		ensure!(line > 0, "LSP tool line numbers start at 1");
		let document = self
			.documents
			.get(uri)
			.context("LSP document is not open")?;
		let text = document
			.text
			.split('\n')
			.nth((line - 1) as usize)
			.context("LSP line is beyond the document")?
			.trim_end_matches('\r');
		let column = if let Some(symbol) = symbol {
			ensure!(character.is_none(), "choose a symbol or explicit character, not both");
			let (symbol, occurrence) = match symbol.rsplit_once('#') {
				Some((name, number))
					if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) =>
				{
					(
						name,
						Some(
							number
								.parse::<usize>()
								.context("invalid symbol occurrence")?,
						),
					)
				},
				_ => (symbol, None),
			};
			ensure!(!symbol.is_empty(), "LSP symbol must not be empty");
			let hits: Vec<_> = text.match_indices(symbol).map(|(index, _)| index).collect();
			let offset = if let Some(occurrence) = occurrence {
				*hits
					.get(
						occurrence
							.checked_sub(1)
							.context("symbol occurrence starts at 1")?,
					)
					.context("symbol occurrence is missing")?
			} else {
				ensure!(hits.len() == 1, "LSP symbol is missing or ambiguous; use #N");
				hits[0]
			};
			text[..offset].encode_utf16().count() as u32
		} else {
			character.context("an explicit UTF-16 character or unambiguous symbol is required")?
		};
		let position = Position { line: line - 1, character: column };
		edits::position_offset(&document.text, position)?;
		Ok(position)
	}

	pub fn notify(&mut self, method: &str, params: Value, cancellation: &CancelToken) -> Result<()> {
		ensure!(!self.closed, "LSP connection is closed");
		self
			.process
			.send(&json!({"jsonrpc":"2.0","method":method,"params":params}), cancellation)
	}

	pub fn request(
		&mut self,
		method: &str,
		params: Value,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<Value> {
		ensure!(
			!self.closed
				&& !method.is_empty()
				&& method.len() <= 256
				&& (params.is_object() || params.is_null()),
			"invalid LSP request or closed connection"
		);
		let id = self.next_id;
		self.next_id = id
			.checked_add(1)
			.filter(|value| *value <= i32::MAX as u64)
			.context("LSP request IDs exhausted")?;
		let deadline = cancellation.child_with_timeout(Duration::from_millis(u64::from(
			self.configuration.process.timeout_ms,
		)));
		let result = (|| {
			self
				.process
				.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}), &deadline)?;
			let mut bytes = 0;
			for _ in 0..1024 {
				let value = self.process.receive(&deadline)?;
				bytes += serde_json::to_vec(&value)?.len();
				ensure!(bytes <= 16 * 1024 * 1024, "LSP response exchange exceeds limit");
				ensure!(value["jsonrpc"] == "2.0", "invalid LSP JSON-RPC envelope");
				if value.get("method").is_some() {
					self.handle_incoming(value, approval, &deadline)?;
					continue;
				}
				ensure!(value["id"] == id, "unexpected LSP response identity");
				return match (value.get("result"), value.get("error")) {
					(Some(value), None) => Ok(value.clone()),
					(None, Some(error)) => {
						Err(RpcError(error["code"].as_i64().context("invalid LSP error code")?).into())
					},
					_ => bail!("LSP response requires exactly one result or error"),
				};
			}
			bail!("LSP exchange has too many messages")
		})();
		if let Err(error) = &result
			&& !error.is::<RpcError>()
		{
			self
				.process
				.try_send(&json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":id}}));
			self.closed = true;
			self.process.terminate();
		}
		result
	}

	fn handle_incoming(
		&mut self,
		value: Value,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<()> {
		let method = value["method"].as_str().context("invalid LSP method")?;
		ensure!(
			value.get("result").is_none() && value.get("error").is_none(),
			"ambiguous LSP message"
		);
		if let Some(id) = value.get("id") {
			ensure!(
				id.as_str().is_some_and(|value| value.len() <= 256) || id.as_i64().is_some(),
				"invalid LSP server request identity"
			);
			let result = match method {
				"workspace/configuration" => {
					let items = value["params"]["items"]
						.as_array()
						.context("invalid configuration request")?;
					ensure!(items.len() <= 256, "too many LSP configuration sections");
					Value::Array(
						items
							.iter()
							.map(|item| {
								item["section"]
									.as_str()
									.map(|section| {
										section
											.split('.')
											.fold(&self.configuration.settings, |value, key| &value[key])
											.clone()
									})
									.unwrap_or_else(|| self.configuration.settings.clone())
							})
							.collect(),
					)
				},
				"workspace/workspaceFolders" => {
					json!([{"uri":reqwest::Url::from_directory_path(&self.root).map_err(|()|anyhow::anyhow!("invalid workspace URI"))?.as_str(),"name":"workspace"}])
				},
				"window/workDoneProgress/create" => Value::Null,
				"workspace/applyEdit" => {
					match self.apply(&value["params"]["edit"], approval, cancellation) {
						Ok(result) => {
							json!({"applied":result.applied,"failureReason":result.failure_reason})
						},
						Err(_) => {
							json!({"applied":false,"failureReason":"Workspace edit rejected by host validation"})
						},
					}
				},
				_ => {
					self.process.send(&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported language-server request"}}),cancellation)?;
					return Ok(());
				},
			};
			self
				.process
				.send(&json!({"jsonrpc":"2.0","id":id,"result":result}), cancellation)?;
		} else if method == "textDocument/publishDiagnostics" {
			let uri = value["params"]["uri"]
				.as_str()
				.context("diagnostics have no URI")?
				.to_owned();
			ensure!(value["params"]["diagnostics"].is_array(), "invalid published diagnostics");
			ensure!(
				self.diagnostics.contains_key(&uri) || self.diagnostics.len() < 256,
				"LSP diagnostics cache exceeds file limit"
			);
			let bytes = self
				.diagnostics
				.iter()
				.filter(|(key, _)| *key != &uri)
				.map(|(_, value)| serde_json::to_vec(value).map(|bytes| bytes.len()))
				.collect::<std::result::Result<Vec<_>, _>>()?
				.into_iter()
				.sum::<usize>();
			ensure!(
				bytes + serde_json::to_vec(&value["params"])?.len() <= 8 * 1024 * 1024,
				"LSP diagnostics cache exceeds byte limit"
			);
			self.diagnostics.insert(uri, value["params"].clone());
		} else {
			if self.notifications.len() == 32 {
				self.notifications.pop_front();
			}
			self.notifications.push_back(value);
		}
		Ok(())
	}

	pub fn diagnostics(&mut self, uri: &str, cancellation: &CancelToken) -> Result<Value> {
		if self.capabilities.get("diagnosticProvider").is_some() {
			return self.request(
				"textDocument/diagnostic",
				json!({"textDocument":{"uri":uri}}),
				&DenyMutations,
				cancellation,
			);
		}
		for _ in 0..1024 {
			cancellation.heartbeat()?;
			let Some(value) = self.process.try_receive()? else {
				break;
			};
			self.handle_incoming(value, &DenyMutations, cancellation)?;
		}
		let current = self.documents.get(uri).map(|document| document.version);
		Ok(match self.diagnostics.get(uri) {
			Some(value) => {
				json!({"available":true,"version":value.get("version"),"current":value["version"].as_i64().map(|version|Some(version)==current),"items":value["diagnostics"]})
			},
			None => json!({"available":false,"current":false}),
		})
	}

	pub fn apply(
		&mut self,
		edit: &Value,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<ApplyResult> {
		let versions = self
			.documents
			.iter()
			.map(|(uri, document)| (uri.clone(), document.version))
			.collect();
		ensure!(self.applied_edits.len() < 256, "too many unconsumed LSP edit reports");
		let mut result =
			edits::apply_workspace_edit(&self.root, edit, &versions, approval, cancellation)?;
		let mut synchronized = true;
		let mut watched = Vec::new();
		for path in &result.changed_files {
			let delivery = (|| -> Result<()> {
				let uri = self.file_uri(path)?;
				if self.documents.contains_key(&uri) {
					self.sync_document(path, cancellation)?;
				}
				watched.push(json!({"uri":uri,"type":2}));
				Ok(())
			})();
			if delivery.is_err() {
				synchronized = false;
				break;
			}
		}
		if synchronized && !watched.is_empty() {
			synchronized = self
				.notify("workspace/didChangeWatchedFiles", json!({"changes":watched}), cancellation)
				.is_ok();
		}
		result.synchronization_pending = !synchronized;
		self.applied_edits.push(serde_json::to_value(&result)?);
		Ok(result)
	}
}
impl Drop for Client {
	fn drop(&mut self) {
		if !self.closed {
			let token = CancelToken::new(Some(100));
			let _ = self.request("shutdown", Value::Null, &DenyMutations, &token);
			self
				.process
				.try_send(&json!({"jsonrpc":"2.0","method":"exit"}));
		}
	}
}
#[derive(Debug)]
pub struct RpcError(pub i64);
impl std::fmt::Display for RpcError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "LSP request failed with code {}", self.0)
	}
}
impl std::error::Error for RpcError {}
