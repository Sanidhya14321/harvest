use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::cancel::CancelToken;
use serde_json::{Value, json};

use super::{
	configuration::{DapConfiguration, LspConfiguration},
	dap, lsp,
};
use crate::{
	chat::{FunctionCall, ToolCall, ToolSpec},
	tools::Approval,
};

#[derive(Default)]
pub struct Runtime {
	root:                PathBuf,
	lsp_configs:         Vec<LspConfiguration>,
	dap_configs:         Vec<DapConfiguration>,
	language_clients:    BTreeMap<String, lsp::Client>,
	debug_clients:       BTreeMap<String, dap::Client>,
	breakpoints:         BTreeMap<(String, String), Vec<Value>>,
	edit_store:          Option<pi_edit::EditStore>,
	unavailable_servers: Vec<super::discovery::UnavailableServer>,
}
impl Runtime {
	/// Synchronize actual native writer paths, including a committed prefix on
	/// failure. This never starts a server or infers success from tool
	/// arguments. Present files are change notifications; a path-only callback
	/// cannot infer whether an existing file was newly created or renamed into
	/// place.
	pub fn synchronize_native_files(
		&mut self,
		paths: &[PathBuf],
		cancellation: &CancelToken,
	) -> Result<()> {
		ensure!(paths.len() <= 256, "too many native synchronization paths");
		if paths.is_empty() || self.language_clients.is_empty() {
			return Ok(());
		}
		let filesystem = harvest_runtime::JailedFs::open(&self.root)?;
		let mut resolved = std::collections::BTreeSet::new();
		let mut path_bytes = 0usize;
		for path in paths {
			cancellation.heartbeat()?;
			let authored = path
				.to_str()
				.context("native synchronization path must be UTF-8")?;
			ensure!(
				!authored.is_empty() && authored.len() <= 4096 && !authored.contains("://"),
				"native synchronization requires a bounded filesystem path"
			);
			path_bytes = path_bytes
				.checked_add(authored.len())
				.context("native synchronization path size overflow")?;
			ensure!(path_bytes <= 128 * 1024, "native synchronization paths exceed byte limit");
			// Resolve every path before inspecting its operation/state, including
			// nonexistent targets beneath an existing symlinked ancestor.
			resolved.insert(filesystem.resource_path(path)?);
		}
		let mut deleted = Vec::new();
		let mut present = Vec::new();
		for path in resolved {
			cancellation.heartbeat()?;
			if filesystem.entry_exists(&path)? {
				ensure!(
					filesystem.file_type(&path)?.is_file(),
					"native synchronization target must be a regular file"
				);
				present.push(super::edits::WorkspaceChange::Edit { path });
			} else {
				deleted.push(super::edits::WorkspaceChange::Delete { path });
			}
		}
		deleted.extend(present);
		self.deliver_native_changes(&deleted, cancellation)
	}

	/// Preserve committed operation identities from the native writer. Jail
	/// every path before any notification, using the existing reconciliation
	/// engine.
	pub(crate) fn synchronize_native_changes(
		&mut self,
		changes: &[super::edits::WorkspaceChange],
		cancellation: &CancelToken,
	) -> Result<()> {
		use super::edits::WorkspaceChange;
		ensure!(changes.len() <= 64, "too many native changes");
		if changes.is_empty() || self.language_clients.is_empty() {
			return Ok(());
		}
		let fs = harvest_runtime::JailedFs::open(&self.root)?;
		let mut bytes = 0usize;
		let mut resolve = |path: &Path| -> Result<PathBuf> {
			cancellation.heartbeat()?;
			let text = path.to_str().context("native change path must be UTF-8")?;
			ensure!(
				!text.is_empty() && text.len() <= 4096 && !text.contains("://"),
				"invalid native change path"
			);
			bytes = bytes
				.checked_add(text.len())
				.context("native change size overflow")?;
			ensure!(bytes <= 128 * 1024, "native change paths exceed limit");
			fs.resource_path(path)
		};
		let mut resolved = Vec::new();
		for change in changes {
			resolved.push(match change {
				WorkspaceChange::Edit { path } => WorkspaceChange::Edit { path: resolve(path)? },
				WorkspaceChange::Create { path } => WorkspaceChange::Create { path: resolve(path)? },
				WorkspaceChange::Delete { path } => WorkspaceChange::Delete { path: resolve(path)? },
				WorkspaceChange::Rename { old_path, new_path } => WorkspaceChange::Rename {
					old_path: resolve(old_path)?,
					new_path: resolve(new_path)?,
				},
			});
		}
		self.deliver_native_changes(&resolved, cancellation)
	}

	fn deliver_native_changes(
		&mut self,
		changes: &[super::edits::WorkspaceChange],
		cancellation: &CancelToken,
	) -> Result<()> {
		let mut failed = false;
		for client in self.language_clients.values_mut() {
			// A previous delivery failure has already invalidated its overlays;
			// normal service invocation owns reconnect and fresh document opens.
			if client.needs_reload() {
				failed = true;
				continue;
			}
			if client
				.reconcile_native_changes(changes, cancellation)
				.is_err()
			{
				failed = true;
			}
		}
		ensure!(
			!failed,
			"native filesystem changes committed; LSP synchronization requires server reload"
		);
		cancellation.heartbeat()?;
		Ok(())
	}

	/// Share the current session snapshot owner; mode changes may replace it.
	pub fn set_edit_store(&mut self, store: pi_edit::EditStore) {
		for client in self.language_clients.values_mut() {
			client.set_edit_store(store.clone());
		}
		self.edit_store = Some(store);
	}

	/// Drain late, version/fingerprint-checked messages from existing clients.
	/// No new server is started and no workspace checker command is spawned.
	pub fn poll_deferred_diagnostics(&mut self, cancellation: &CancelToken) -> Result<Vec<Value>> {
		let mut updates = Vec::new();
		let mut remaining = harvest_runtime::MAX_TOOL_BYTES / 2;
		for (name, client) in &mut self.language_clients {
			cancellation.heartbeat()?;
			if remaining < 1024 {
				break;
			}
			match client.poll_deferred_diagnostics(cancellation, remaining - 1024) {
				Ok(messages) => {
					for mut message in messages {
						message["server"] = json!(name);
						let size = message.to_string().len();
						remaining = remaining.saturating_sub(size);
						updates.push(message);
					}
				},
				Err(_) if cancellation.aborted() => {
					cancellation.heartbeat()?;
				},
				Err(_) => {
					let value = json!({"server":name,"available":false,"reason":"diagnostic_transport_unavailable"});
					remaining = remaining.saturating_sub(value.to_string().len());
					updates.push(value);
				},
			}
		}
		Ok(updates)
	}

	/// Opt-in discovery for explicit native LSP entrypoints. Ordinary configure
	/// remains explicit-only so the application's enable gate owns startup.
	pub fn configure_discovered(
		&mut self,
		root: &Path,
		explicit: Vec<LspConfiguration>,
		dap: Vec<DapConfiguration>,
	) -> Result<()> {
		let discovery = super::discovery::discover(root)?;
		let mut selected: BTreeMap<_, _> = discovery
			.configurations
			.into_iter()
			.map(|c| (c.process.name.clone(), c))
			.collect();
		for config in explicit {
			selected.insert(config.process.name.clone(), config);
		}
		let mut selected: Vec<_> = selected.into_values().collect();
		selected.sort_by_key(|c| (c.is_linter, c.process.name.clone()));
		self.configure(root, selected, dap)?;
		self.unavailable_servers = discovery
			.unavailable
			.into_iter()
			.filter(|entry| {
				!self
					.lsp_configs
					.iter()
					.any(|config| config.process.name == entry.name)
			})
			.collect();
		Ok(())
	}

	pub fn configure(
		&mut self,
		root: &Path,
		lsp: Vec<LspConfiguration>,
		dap: Vec<DapConfiguration>,
	) -> Result<()> {
		ensure!(lsp.len() <= 16 && dap.len() <= 16, "too many native language/debug services");
		let mut names = std::collections::BTreeSet::new();
		for server in &lsp {
			server.validate()?;
			ensure!(names.insert(server.process.name.clone()), "duplicate LSP server name");
		}
		names.clear();
		for adapter in &dap {
			adapter.validate()?;
			ensure!(names.insert(adapter.process.name.clone()), "duplicate debug adapter name");
		}
		self.language_clients.clear();
		self.debug_clients.clear();
		self.breakpoints.clear();
		self.unavailable_servers.clear();
		self.root = root.canonicalize()?;
		self.lsp_configs = lsp;
		self.dap_configs = dap;
		Ok(())
	}

	pub fn specs(&self) -> Vec<ToolSpec> {
		let mut tools = Vec::new();
		if !self.lsp_configs.is_empty() {
			tools.push(ToolSpec::from_schema("lsp".into(),include_str!("../prompts/tools/lsp-native.md").trim().into(),json!({"type":"object","properties":{"action":{"type":"string"},"server":{"type":"string"},"file":{"type":"string"},"line":{"type":"integer","minimum":1},"character":{"type":"integer","minimum":0},"symbol":{"type":"string"},"query":{"type":"string"},"new_name":{"type":"string"},"apply":{"type":"boolean"},"method":{"type":"string"},"payload":{"type":["object","string"]},"options":{"type":"object"}},"required":["action"]})).expect("static LSP schema"));
		}
		if !self.dap_configs.is_empty() {
			tools.push(ToolSpec::from_schema("debug".into(),include_str!("../prompts/tools/debug-native.md").trim().into(),json!({"type":"object","properties":{"action":{"type":"string"},"adapter":{"type":"string"},"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"file":{"type":"string"},"line":{"type":"integer"},"thread_id":{"type":"integer"},"frame_id":{"type":"integer"},"variable_ref":{"type":"integer"},"expression":{"type":"string"},"command":{"type":"string"},"arguments":{"type":"object"},"condition":{"type":"string"},"hit_condition":{"type":"string"},"memory_reference":{"type":"string"},"count":{"type":"integer"},"data":{"type":"string"},"cwd":{"type":"string"},"pid":{"type":"integer"},"context":{"type":"string"},"scope_id":{"type":"integer"},"offset":{"type":"integer"},"levels":{"type":"integer"},"instruction_count":{"type":"integer"},"instruction_offset":{"type":"integer"}},"required":["action"]})).expect("static debug schema"));
		}
		tools
	}

	pub fn invoke(
		&mut self,
		call: &ToolCall,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<String> {
		ensure!(
			call.function.arguments.len() <= crate::process_transport::MAX_FRAME_BYTES,
			"language tool arguments exceed limit"
		);
		let arguments: Value = serde_json::from_str(&call.function.arguments)
			.map_err(|_| anyhow::anyhow!("invalid language tool arguments"))?;
		ensure!(arguments.is_object(), "language tool arguments must be an object");
		let result = match call.function.name.as_str() {
			"lsp" => self.language(&arguments, call, approval, cancellation),
			"debug" => self.debug(&arguments, call, approval, cancellation),
			_ => bail!("unknown language service tool"),
		};
		let deferred = if cancellation.aborted() {
			Vec::new()
		} else {
			self
				.poll_deferred_diagnostics(cancellation)
				.unwrap_or_default()
		};
		// Even a failed semantic request can have applied a server-requested edit.
		// Broadcast its actual prefix before returning either success or failure.
		let batches: Vec<_> = self
			.language_clients
			.iter_mut()
			.filter_map(|(name, client)| {
				let changes = client.take_workspace_changes();
				(!changes.is_empty()).then(|| (name.clone(), changes))
			})
			.collect();
		let mut synchronization_pending = false;
		for (origin, changes) in &batches {
			for (name, client) in &mut self.language_clients {
				if name != origin && client.reconcile_changes(changes, cancellation).is_err() {
					synchronization_pending = true;
				}
			}
		}
		let reports: Vec<_> = self
			.language_clients
			.values_mut()
			.flat_map(lsp::Client::take_applied_edits)
			.collect();
		let mut result = match result {
			Ok(value) => value,
			Err(error) if reports.is_empty() => return Err(error),
			Err(error) => {
				let mut output =
					harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES - 4096);
				serde_json::to_writer(&mut output, &reports)?;
				return Err(error.context(format!(
					"LSP request failed after workspace publication; re-read affected files; committed \
					 reports: {}",
					String::from_utf8(output.into_inner())?
				)));
			},
		};
		if !reports.is_empty() {
			if let Some(object) = result.as_object_mut() {
				object.insert("workspaceEdits".into(), json!(reports));
			} else {
				result = json!({"result":result,"workspaceEdits":reports});
			}
		}
		if synchronization_pending {
			if !result.is_object() {
				result = json!({"result":result});
			}
			result["workspaceSynchronizationPending"] = json!(true);
		}
		if !deferred.is_empty() {
			if !result.is_object() {
				result = json!({"result":result});
			}
			if let Some(items) = result
				.get_mut("deferredDiagnostics")
				.and_then(Value::as_array_mut)
			{
				items.extend(deferred);
			} else {
				result["deferredDiagnostics"] = json!(deferred);
			}
		}
		let mut output = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES);
		if serde_json::to_writer(&mut output, &result).is_err() {
			let has_reports = result
				.get("workspaceEdits")
				.is_some_and(|value| value.as_array().is_some_and(|v| !v.is_empty()));
			let has_diagnostics = result
				.get("deferredDiagnostics")
				.is_some_and(|value| value.as_array().is_some_and(|v| !v.is_empty()));
			if has_reports || has_diagnostics {
				let mut summary = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES);
				serde_json::to_writer(
					&mut summary,
					&json!({"resultUnavailable":"host_output_limit","workspaceEdits":result.get("workspaceEdits"),"deferredDiagnostics":result.get("deferredDiagnostics"),
					"workspaceSynchronizationPending":synchronization_pending}),
				)?;
				return Ok(String::from_utf8(summary.into_inner())?);
			}
			bail!("language service result exceeds host output limit");
		}
		Ok(String::from_utf8(output.into_inner())?)
	}

	fn language(
		&mut self,
		arguments: &Value,
		call: &ToolCall,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<Value> {
		let action = arguments["action"]
			.as_str()
			.context("LSP action is required")?;
		if action == "diagnostics_deferred" {
			return Ok(json!({"deferredDiagnostics":self.poll_deferred_diagnostics(cancellation)?}));
		}
		if action == "diagnostics" && arguments["file"] == "*" {
			let mut reports = Vec::new();
			let mut bytes = 0usize;
			for (name, client) in &self.language_clients {
				if client.needs_reload() {
					continue;
				}
				for report in client.cached_workspace_diagnostics() {
					let value = json!({"server":name,"diagnostics":report});
					let size = value.to_string().len();
					if bytes + size > harvest_runtime::MAX_TOOL_BYTES / 2 {
						break;
					}
					bytes += size;
					reports.push(value);
				}
			}
			return Ok(
				json!({"scope":"published_workspace_documents","complete":false,"files":reports,
				"note":"Only cached server diagnostics are included; workspace checker execution is pending conversion"}),
			);
		}
		if action == "status" {
			return Ok(
				json!({"servers":self.lsp_configs.iter().map(|server|json!({"name":server.process.name,"connected":self.language_clients.get(&server.process.name).is_some_and(|client|!client.needs_reload()),"requiresReload":self.language_clients.get(&server.process.name).is_some_and(lsp::Client::needs_reload),"extensions":server.extensions})).collect::<Vec<_>>(),"unavailable":self.unavailable_servers}),
			);
		}
		let file = arguments["file"].as_str();
		let mut configurations: Vec<_> = self
			.lsp_configs
			.iter()
			.filter(|server| {
				if let Some(name) = arguments["server"].as_str() {
					return server.process.name == name;
				}
				if let Some(file) = file.filter(|file| *file != "*") {
					return server.supports_path(Path::new(file));
				}
				true
			})
			.collect();
		if arguments["server"].is_null() && file.is_some() && configurations.len() > 1 {
			let primary: Vec<_> = configurations
				.iter()
				.copied()
				.filter(|server| !server.is_linter)
				.collect();
			if primary.len() == 1 {
				configurations = primary;
			}
		}
		ensure!(
			configurations.len() == 1,
			"choose one configured LSP server explicitly; file selection is missing or ambiguous"
		);
		let configuration = (*configurations[0]).clone();
		let name = configuration.process.name.clone();
		let readonly = matches!(
			action,
			"hover"
				| "definition"
				| "type_definition"
				| "implementation"
				| "references"
				| "symbols"
				| "diagnostics"
				| "capabilities"
		);
		if !readonly {
			ensure!(approval.approve(call)?, "LSP action requires host approval");
		}
		cancellation.heartbeat()?;
		if action == "reload" {
			self.language_clients.remove(&name);
			return Ok(json!({"reloaded":name}));
		}
		if self
			.language_clients
			.get(&name)
			.is_some_and(lsp::Client::needs_reload)
		{
			self.language_clients.remove(&name);
		}
		if !self.language_clients.contains_key(&name) {
			if configuration.startup_approval {
				let mut environment =
					harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_EVENT_BYTES);
				serde_json::to_writer(&mut environment, &configuration.process.env)?;
				let arguments = json!({"action":"start_server","server":name,"command":configuration.process.command,
					"args":configuration.process.args,"cwd":configuration.process.cwd,"environmentNames":configuration.process.env.keys().collect::<Vec<_>>(),
					"environmentFingerprint":harvest_runtime::content_hash(&environment.into_inner()),"parentCall":call});
				let mut bounded = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_EVENT_BYTES);
				serde_json::to_writer(&mut bounded, &arguments)?;
				let startup = ToolCall {
					id:       call.id.clone(),
					kind:     call.kind.clone(),
					function: FunctionCall {
						name:      "lsp".into(),
						arguments: String::from_utf8(bounded.into_inner())?,
					},
				};
				ensure!(
					approval.approve(&startup)?,
					"discovered LSP server startup requires host approval"
				);
			}
			let mut client = lsp::Client::connect(&self.root, &configuration, cancellation)?;
			if let Some(store) = &self.edit_store {
				client.set_edit_store(store.clone());
			}
			self.language_clients.insert(name.clone(), client);
		}
		let client = self
			.language_clients
			.get_mut(&name)
			.context("LSP owner is missing")?;
		if action == "capabilities" {
			return Ok(client.capabilities().clone());
		}
		if action == "request" {
			let payload = match arguments.get("payload") {
				Some(Value::String(value)) => serde_json::from_str(value)
					.map_err(|_| anyhow::anyhow!("raw LSP payload is not valid JSON"))?,
				Some(value) => value.clone(),
				None => {
					if let Some(file) = file.filter(|file| *file != "*") {
						let uri = client.sync_document(Path::new(file), cancellation)?;
						let mut params = json!({"textDocument":{"uri":uri}});
						if let Some(line) = arguments["line"]
							.as_u64()
							.and_then(|value| u32::try_from(value).ok())
						{
							let character = arguments["character"]
								.as_u64()
								.and_then(|value| u32::try_from(value).ok());
							params["position"] = serde_json::to_value(client.position(
								&uri,
								line,
								character,
								arguments["symbol"].as_str(),
							)?)?;
						}
						params
					} else {
						json!({})
					}
				},
			};
			return client.request(
				arguments["method"]
					.as_str()
					.or_else(|| arguments["query"].as_str())
					.context("raw LSP method is required")?,
				payload,
				approval,
				cancellation,
			);
		}
		if action == "symbols" && (file.is_none() || file == Some("*")) {
			return client.request(
				"workspace/symbol",
				json!({"query":arguments["query"].as_str().unwrap_or("")}),
				approval,
				cancellation,
			);
		}
		let file = file.context("LSP file is required")?;
		let uri = client.sync_document(Path::new(file), cancellation)?;
		let mut params = json!({"textDocument":{"uri":uri}});
		if matches!(
			action,
			"hover"
				| "definition"
				| "type_definition"
				| "implementation"
				| "references"
				| "rename"
				| "code_actions"
		) {
			let line = arguments["line"]
				.as_u64()
				.and_then(|value| u32::try_from(value).ok())
				.context("LSP line is required")?;
			let character = arguments
				.get("character")
				.map(|value| {
					value
						.as_u64()
						.and_then(|value| u32::try_from(value).ok())
						.context("invalid UTF-16 character")
				})
				.transpose()?;
			let position = client.position(&uri, line, character, arguments["symbol"].as_str())?;
			params["position"] = serde_json::to_value(position)?;
		}
		let method = match action {
			"hover" => "textDocument/hover",
			"definition" => "textDocument/definition",
			"type_definition" => "textDocument/typeDefinition",
			"implementation" => "textDocument/implementation",
			"symbols" => "textDocument/documentSymbol",
			"references" => {
				params["context"] = json!({"includeDeclaration":true});
				"textDocument/references"
			},
			"rename" => {
				params["newName"] = json!(
					arguments["new_name"]
						.as_str()
						.context("rename new_name is required")?
				);
				"textDocument/rename"
			},
			"code_actions" => {
				let position = params["position"].clone();
				params.as_object_mut().unwrap().remove("position");
				params["range"] = json!({"start":position,"end":position});
				params["context"] = json!({"diagnostics":client.diagnostic_items(&uri)});
				"textDocument/codeAction"
			},
			"format" => {
				params["options"] = match arguments.get("options") {
					Some(value) => value.clone(),
					None => client.formatting_options(&uri)?,
				};
				"textDocument/formatting"
			},
			"diagnostics" => return client.diagnostics(&uri, cancellation),
			_ => bail!("LSP action is not converted"),
		};
		let result = client.request(method, params, approval, cancellation)?;
		let apply = arguments["apply"].as_bool().unwrap_or(action == "rename");
		let result = if apply && matches!(action, "rename" | "format" | "code_actions") {
			let edit = if action == "format" {
				json!({"changes":{(uri.clone()):result}})
			} else if action == "code_actions" {
				let query = arguments["query"]
					.as_str()
					.context("applying a code action requires query")?;
				let actions = result.as_array().context("invalid code action response")?;
				let selected: Vec<_> = actions
					.iter()
					.enumerate()
					.filter(|(index, action)| {
						query.parse::<usize>().map_or_else(
							|_| {
								action["title"]
									.as_str()
									.is_some_and(|title| title.contains(query))
							},
							|number| *index == number,
						)
					})
					.map(|(_, action)| action)
					.collect();
				ensure!(selected.len() == 1, "code action selection is missing or ambiguous");
				ensure!(
					selected[0].get("command").is_none(),
					"combined edit/command code-action application is not converted"
				);
				selected[0]
					.get("edit")
					.cloned()
					.context("selected code action has no workspace edit")?
			} else {
				result
			};
			let mut preview = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES / 2);
			serde_json::to_writer(&mut preview, &edit)?;
			let applied = client.apply_for_tool(&edit, call, approval, cancellation)?;
			json!({"edit":edit,"application":applied})
		} else {
			result
		};
		Ok(
			json!({"result":result,"notifications":client.take_notifications(),"workspaceEdits":client.take_applied_edits()}),
		)
	}

	fn debug(
		&mut self,
		arguments: &Value,
		call: &ToolCall,
		approval: &dyn Approval,
		cancellation: &CancelToken,
	) -> Result<Value> {
		let action = arguments["action"]
			.as_str()
			.context("debug action is required")?;
		if action == "sessions" {
			return Ok(
				json!({"sessions":self.debug_clients.iter().map(|(name,client)|json!({"adapter":name,"status":client.status})).collect::<Vec<_>>() }),
			);
		}
		let adapters: Vec<_> = self
			.dap_configs
			.iter()
			.filter(|configuration| {
				arguments["adapter"]
					.as_str()
					.is_none_or(|name| configuration.process.name == name)
			})
			.collect();
		ensure!(adapters.len() == 1, "choose one configured debug adapter explicitly");
		let configuration = (*adapters[0]).clone();
		let name = configuration.process.name.clone();
		let mut request = arguments
			.get("arguments")
			.cloned()
			.unwrap_or_else(|| json!({}));
		ensure!(request.is_object(), "debug arguments must be an object");
		macro_rules! field {
			($source:literal, $target:literal) => {
				if let Some(value) = arguments.get($source) {
					request[$target] = value.clone();
				}
			};
		}
		field!("thread_id", "threadId");
		field!("frame_id", "frameId");
		field!("variable_ref", "variablesReference");
		field!("scope_id", "variablesReference");
		field!("expression", "expression");
		field!("context", "context");
		field!("memory_reference", "memoryReference");
		field!("offset", "offset");
		field!("count", "count");
		field!("data", "data");
		field!("levels", "levels");
		field!("instruction_count", "instructionCount");
		field!("instruction_offset", "instructionOffset");
		let command = match action {
			"launch" | "attach" => {
				let defaults = if action == "attach" {
					&configuration.attach_defaults
				} else {
					&configuration.launch_defaults
				};
				let mut merged = defaults
					.as_object()
					.context("invalid debug defaults")?
					.clone();
				merged.extend(request.as_object().unwrap().clone());
				request = Value::Object(merged);
				field!("program", "program");
				field!("args", "args");
				field!("cwd", "cwd");
				field!("pid", "processId");
				action
			},
			"continue" => "continue",
			"step_over" => "next",
			"step_in" => "stepIn",
			"step_out" => "stepOut",
			"pause" => "pause",
			"threads" => "threads",
			"stack_trace" => "stackTrace",
			"scopes" => "scopes",
			"variables" => "variables",
			"evaluate" => "evaluate",
			"read_memory" => "readMemory",
			"write_memory" => "writeMemory",
			"disassemble" => "disassemble",
			"modules" => "modules",
			"loaded_sources" => "loadedSources",
			"terminate" => "terminate",
			"disconnect" => "disconnect",
			"set_breakpoint" | "remove_breakpoint" => {
				let file = arguments["file"]
					.as_str()
					.context("breakpoint file is required")?;
				let filesystem = harvest_runtime::JailedFs::open(&self.root)?;
				let path = filesystem.resource_path(Path::new(file))?;
				let line = arguments["line"]
					.as_u64()
					.filter(|line| *line > 0)
					.context("breakpoint line is required")?;
				let key = (name.clone(), path.to_string_lossy().into_owned());
				let mut breakpoints = self.breakpoints.get(&key).cloned().unwrap_or_default();
				breakpoints.retain(|breakpoint| breakpoint["line"] != line);
				if action == "set_breakpoint" {
					let mut breakpoint = json!({"line":line});
					if let Some(condition) = arguments.get("condition") {
						breakpoint["condition"] = condition.clone();
					}
					if let Some(condition) = arguments.get("hit_condition") {
						breakpoint["hitCondition"] = condition.clone();
					}
					breakpoints.push(breakpoint);
				}
				request = json!({"source":{"path":path},"breakpoints":breakpoints});
				"setBreakpoints"
			},
			"custom_request" => arguments["command"]
				.as_str()
				.context("custom debugger command is required")?,
			"output" => "output",
			_ => bail!(
				"debug action is not converted; use an explicitly approved custom_request when \
				 supported by the adapter"
			),
		};
		if matches!(command, "continue" | "next" | "stepIn" | "stepOut" | "pause" | "stackTrace") {
			ensure!(
				request["threadId"].as_u64().is_some(),
				"this debugger request requires thread_id"
			);
		}
		let final_call = ToolCall {
			id:       call.id.clone(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "debug".into(),
				arguments: serde_json::to_string(
					&json!({"adapter":name,"action":action,"command":command,"arguments":request}),
				)?,
			},
		};
		ensure!(approval.approve(&final_call)?, "debugger request requires host approval");
		cancellation.heartbeat()?;
		if !self.debug_clients.contains_key(&name) {
			self
				.debug_clients
				.insert(name.clone(), dap::Client::connect(&self.root, &configuration, cancellation)?);
		}
		let client = self
			.debug_clients
			.get_mut(&name)
			.context("debug adapter owner is missing")?;
		let result = if action == "output" {
			json!({"events":client.drain_events(cancellation)?})
		} else if matches!(action, "launch" | "attach") {
			client.start(action == "attach", request.clone(), cancellation)?
		} else {
			client.request(command, request.clone(), cancellation)?
		};
		if command == "setBreakpoints" {
			let path = request["source"]["path"]
				.as_str()
				.context("invalid breakpoint source")?;
			self.breakpoints.insert(
				(name.clone(), path.into()),
				request["breakpoints"].as_array().unwrap().clone(),
			);
		}
		let output = json!({"result":result,"status":client.status,"events":client.events()});
		if action == "disconnect" {
			self.debug_clients.remove(&name);
			self.breakpoints.retain(|(adapter, _), _| adapter != &name);
		}
		Ok(output)
	}
}
