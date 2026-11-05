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
	edits::{self, ApplyResult, Position, WorkspaceChange},
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
	root:              PathBuf,
	configuration:     LspConfiguration,
	process:           Process<Value>,
	capabilities:      Value,
	next_id:           u64,
	closed:            bool,
	documents:         BTreeMap<String, Document>,
	diagnostics:       BTreeMap<String, Value>,
	notifications:     VecDeque<Value>,
	applied_edits:     Vec<Value>,
	workspace_changes: Vec<WorkspaceChange>,
	edit_store:        Option<pi_edit::EditStore>,
	deferred:          super::diagnostics::Deferred,
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
			workspace_changes: Vec::new(),
			edit_store: None,
			deferred: Default::default(),
		};
		let uri = reqwest::Url::from_directory_path(&client.root)
			.map_err(|()| anyhow::anyhow!("cannot encode LSP workspace URI"))?;
		let initialized=client.request("initialize",json!({"processId":std::process::id(),"clientInfo":{"name":"harvest-rust","version":env!("CARGO_PKG_VERSION")},"rootUri":uri.as_str(),"workspaceFolders":[{"uri":uri.as_str(),"name":"workspace"}],"capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true,"resourceOperations":["create","rename","delete"],"failureHandling":"abort"},"configuration":true,"workspaceFolders":true},"textDocument":{"synchronization":{"didSave":true},"publishDiagnostics":{"versionSupport":true},"documentSymbol":{"hierarchicalDocumentSymbolSupport":true},"rename":{"prepareSupport":true},"diagnostic":{"dynamicRegistration":false}}},"initializationOptions":configuration.initialization_options}),&DenyMutations,&deadline)?;
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

	pub(crate) fn set_edit_store(&mut self, store: pi_edit::EditStore) {
		self.edit_store = Some(store);
	}

	pub(crate) fn take_workspace_changes(&mut self) -> Vec<WorkspaceChange> {
		std::mem::take(&mut self.workspace_changes)
	}

	pub fn needs_reload(&self) -> bool {
		self.closed
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
		ensure!(!self.closed, "LSP connection requires reload before document synchronization");
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
			self.notify("textDocument/didOpen",json!({"textDocument":{"uri":uri,"version":1,"languageId":self.configuration.document_language(&resolved),"text":text}}),cancellation)?;
			self
				.documents
				.insert(uri.clone(), Document { version: 1, text });
		}
		if let Some(document) = self.documents.get(&uri) {
			self
				.deferred
				.expect(uri.clone(), resolved, &document.text, document.version);
		}
		Ok(uri)
	}

	pub fn formatting_options(&self, uri: &str) -> Result<Value> {
		let filesystem = JailedFs::open(&self.root)?;
		let path = edits::uri_path(&filesystem, uri)?;
		let document = self.documents.get(uri).context("document is not open")?;
		super::formatting::resolve(&filesystem, &path, &document.text)
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
					let context =
						json!({"server":self.name(),"requestId":id,"label":value["params"].get("label")});
					match self.apply_with_context(
						&value["params"]["edit"],
						approval,
						cancellation,
						Some(&context),
					) {
						Ok(result) => {
							json!({"applied":result.applied,"failureReason":result.failure_reason,"failedChange":result.failed_change_index})
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
			self.cache_diagnostics(value["params"].clone())?;
		} else {
			if self.notifications.len() == 32 {
				self.notifications.pop_front();
			}
			self.notifications.push_back(value);
		}
		Ok(())
	}

	fn cache_diagnostics(&mut self, mut params: Value) -> Result<()> {
		let filesystem = JailedFs::open(&self.root)?;
		let raw_uri = params["uri"].as_str().context("diagnostics have no URI")?;
		let Ok(path) = edits::uri_path(&filesystem, raw_uri) else {
			return Ok(());
		};
		let uri = self.file_uri(&path)?;
		super::diagnostics::validate(&params["diagnostics"])?;
		if let Some(version) = params.get("version").filter(|v| !v.is_null()) {
			ensure!(version.as_i64().is_some(), "invalid diagnostic version");
		}
		ensure!(
			self.diagnostics.contains_key(&uri) || self.diagnostics.len() < 256,
			"LSP diagnostics cache exceeds file limit"
		);
		let mut budget = harvest_runtime::BoundedWriter::new(8 * 1024 * 1024);
		for (key, value) in &self.diagnostics {
			if key != &uri {
				serde_json::to_writer(&mut budget, value)?;
			}
		}
		params["uri"] = json!(uri);
		serde_json::to_writer(&mut budget, &params)?;
		self.diagnostics.insert(uri, params);
		Ok(())
	}

	fn poll_diagnostic_messages(&mut self, cancellation: &CancelToken) -> Result<()> {
		let mut budget = harvest_runtime::BoundedWriter::new(8 * 1024 * 1024);
		for _ in 0..64 {
			cancellation.heartbeat()?;
			let Some(value) = self.process.try_receive()? else {
				break;
			};
			serde_json::to_writer(&mut budget, &value)?;
			ensure!(
				value["jsonrpc"] == "2.0" && value.get("method").is_some(),
				"unexpected idle LSP response"
			);
			self.handle_incoming(value, &DenyMutations, cancellation)?;
		}
		Ok(())
	}

	pub(crate) fn diagnostic_items(&self, uri: &str) -> Vec<Value> {
		let version = self.documents.get(uri).map(|document| document.version);
		self
			.diagnostics
			.get(uri)
			.filter(|params| params["version"].as_i64() == version && version.is_some())
			.and_then(|params| params["diagnostics"].as_array())
			.map(|items| items.iter().take(50).cloned().collect())
			.unwrap_or_default()
	}

	fn cached_diagnostics(&self, uri: &str) -> Value {
		match self.diagnostics.get(uri) {
			Some(value) => {
				let version = value["version"].as_i64();
				let current = version.map(|version| {
					self
						.documents
						.get(uri)
						.is_some_and(|document| document.version == version)
				});
				if current == Some(false) {
					json!({"uri":uri,"available":false,"current":false,"staleVersion":version,"items":[]})
				} else {
					super::diagnostics::report(
						uri,
						version,
						current,
						value["diagnostics"]
							.as_array()
							.map(Vec::as_slice)
							.unwrap_or(&[]),
					)
				}
			},
			None => json!({"uri":uri,"available":false,"current":false,"items":[]}),
		}
	}

	pub(crate) fn cached_workspace_diagnostics(&self) -> Vec<Value> {
		self
			.diagnostics
			.keys()
			.take(20)
			.map(|uri| self.cached_diagnostics(uri))
			.collect()
	}

	pub(crate) fn poll_deferred_diagnostics(
		&mut self,
		cancellation: &CancelToken,
		max_bytes: usize,
	) -> Result<Vec<Value>> {
		if self.closed {
			return Ok(Vec::new());
		}
		let result = (|| {
			self.poll_diagnostic_messages(cancellation)?;
			self
				.deferred
				.collect(&JailedFs::open(&self.root)?, &self.diagnostics, max_bytes)
		})();
		if result.is_err() && !cancellation.aborted() {
			self.closed = true;
			self.documents.clear();
			self.diagnostics.clear();
			self.deferred.clear();
			self.process.terminate();
		}
		result
	}

	pub fn diagnostics(&mut self, uri: &str, cancellation: &CancelToken) -> Result<Value> {
		let filesystem = JailedFs::open(&self.root)?;
		let path = edits::uri_path(&filesystem, uri)?;
		let uri = self.sync_document(&path, cancellation)?;
		self.poll_diagnostic_messages(cancellation)?;
		if self
			.capabilities
			.get("diagnosticProvider")
			.is_some_and(|v| !v.is_null() && v.as_bool() != Some(false))
		{
			let version = self.documents.get(&uri).map(|d| d.version);
			let before = filesystem.read(&path, MAX_FILE_BYTES)?;
			let previous = self
				.diagnostics
				.get(&uri)
				.and_then(|v| v.get("resultId"))
				.filter(|v| v.is_string())
				.cloned();
			let mut params = json!({"textDocument":{"uri":uri}});
			if let Some(id) = &previous {
				params["previousResultId"] = id.clone();
			}
			let result =
				self.request("textDocument/diagnostic", params, &DenyMutations, cancellation)?;
			if filesystem.read(&path, MAX_FILE_BYTES)? != before {
				return Ok(
					json!({"uri":uri,"available":false,"current":false,"reason":"document_changed"}),
				);
			}
			match result["kind"].as_str() {
                Some("full") => self.cache_diagnostics(json!({"uri":uri,"version":version,"diagnostics":result["items"],"resultId":result.get("resultId")}))?,
                Some("unchanged") => {
                    let cached = self.diagnostics.get_mut(&uri).context("unchanged diagnostics have no prior result")?;
                    ensure!(previous.is_some() && result.get("resultId") == previous.as_ref(), "unchanged diagnostic result identity differs");
                    cached["version"] = json!(version);
                },
                _ => bail!("invalid LSP diagnostic report kind"),
            }
		}
		let report = self.cached_diagnostics(&uri);
		if report["current"] == true {
			if let Some(items) = report["items"].as_array() {
				self.deferred.acknowledge(&uri, items);
			}
		}
		Ok(report)
	}

	pub fn apply(
		&mut self,
		edit: &Value,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<ApplyResult> {
		self.apply_with_context(edit, approval, cancellation, None)
	}

	pub(crate) fn apply_for_tool(
		&mut self,
		edit: &Value,
		call: &crate::chat::ToolCall,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<ApplyResult> {
		self.apply_with_context(
			edit,
			approval,
			cancellation,
			Some(&json!({"toolCall":call,"server":self.name()})),
		)
	}

	fn apply_with_context(
		&mut self,
		edit: &Value,
		approval: &dyn Approval,
		cancellation: &CancelToken,
		context: Option<&Value>,
	) -> Result<ApplyResult> {
		ensure!(!self.closed, "LSP connection requires reload before workspace edits");
		let filesystem = JailedFs::open(&self.root)?;
		let mut versions = BTreeMap::new();
		for (uri, document) in &self.documents {
			let path = edits::uri_path(&filesystem, uri)?;
			if filesystem.read_optional(&path, MAX_FILE_BYTES)?.as_deref()
				== Some(document.text.as_bytes())
			{
				versions.insert(uri.clone(), document.version);
			}
		}
		ensure!(self.applied_edits.len() < 256, "too many unconsumed LSP edit reports");
		let mut reports = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES / 4);
		serde_json::to_writer(&mut reports, &self.applied_edits)?;
		ensure!(
			self.workspace_changes.len() <= 4096 - 512,
			"unconsumed LSP workspace changes exceed limit"
		);
		let mut result =
			super::workspace::apply(&self.root, edit, &versions, approval, cancellation, context)?;
		self
			.workspace_changes
			.extend(result.executed_changes.iter().cloned());
		result.synchronization_pending = self
			.reconcile_changes(&result.executed_changes, cancellation)
			.is_err();
		self.applied_edits.push(serde_json::to_value(&result)?);
		Ok(result)
	}

	/// Reconcile actual committed operations, including a failed write's prefix.
	/// Failed delivery invalidates the connection; the next runtime invocation
	/// reconnects and opens fresh disk content instead of using stale overlays.
	pub(crate) fn reconcile_changes(
		&mut self,
		changes: &[WorkspaceChange],
		cancellation: &CancelToken,
	) -> Result<()> {
		self.reconcile(changes, cancellation, true)
	}

	/// Native writes already invalidated provenance; native edits published
	/// fresh engine-owned snapshots. Only synchronize language-server state.
	pub(crate) fn reconcile_native_changes(
		&mut self,
		changes: &[WorkspaceChange],
		cancellation: &CancelToken,
	) -> Result<()> {
		self.reconcile(changes, cancellation, false)
	}

	fn reconcile(
		&mut self,
		changes: &[WorkspaceChange],
		cancellation: &CancelToken,
		invalidate_snapshots: bool,
	) -> Result<()> {
		if changes.is_empty() {
			return Ok(());
		}
		if invalidate_snapshots {
			for change in changes {
				for path in change.paths() {
					if let Some(store) = &self.edit_store {
						store.invalidate(&pi_edit::path_policy::canonical_key(path));
					}
				}
			}
		}
		let delivery = (|| -> Result<()> {
			ensure!(!self.closed, "LSP connection requires reload after workspace changes");
			let mut deleted = std::collections::BTreeSet::new();
			let mut final_paths = std::collections::BTreeSet::new();
			let mut watched = Vec::new();
			for change in changes {
				let mut watch = |path: &Path, kind: u32| -> Result<()> {
					let uri = self.file_uri(path)?;
					watched.push(json!({"uri":uri,"type":kind}));
					self.diagnostics.remove(&uri);
					self.deferred.invalidate(&uri);
					Ok(())
				};
				match change {
					WorkspaceChange::Edit { path } => {
						final_paths.insert(path.clone());
						watch(path, 2)?;
					},
					WorkspaceChange::Create { path } => {
						final_paths.insert(path.clone());
						watch(path, 1)?;
					},
					WorkspaceChange::Delete { path } => {
						deleted.insert(path.clone());
						final_paths.remove(path);
						watch(path, 3)?;
					},
					WorkspaceChange::Rename { old_path, new_path } => {
						deleted.insert(old_path.clone());
						final_paths.remove(old_path);
						final_paths.insert(new_path.clone());
						watch(old_path, 3)?;
						watch(new_path, 1)?;
					},
				}
			}
			for path in deleted {
				let uri = self.file_uri(&path)?;
				if self.documents.contains_key(&uri) {
					self.notify(
						"textDocument/didClose",
						json!({"textDocument":{"uri":uri}}),
						cancellation,
					)?;
					self.documents.remove(&uri);
					self.deferred.forget(&uri);
				}
			}
			for path in final_paths {
				let uri = self.file_uri(&path)?;
				if self.documents.contains_key(&uri) {
					self.sync_document(&path, cancellation)?;
					let mut params = json!({"textDocument":{"uri":uri}});
					if self.capabilities["textDocumentSync"]["save"]["includeText"] == true {
						params["text"] = json!(
							self
								.documents
								.get(&uri)
								.context("saved LSP document is missing")?
								.text
						);
					}
					self.notify("textDocument/didSave", params, cancellation)?;
				}
			}
			self.notify("workspace/didChangeWatchedFiles", json!({"changes":watched}), cancellation)
		})();
		if delivery.is_err() {
			self.closed = true;
			self.documents.clear();
			self.diagnostics.clear();
			self.deferred.clear();
			self.process.terminate();
		}
		delivery
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
