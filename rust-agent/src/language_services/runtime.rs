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
	root:             PathBuf,
	lsp_configs:      Vec<LspConfiguration>,
	dap_configs:      Vec<DapConfiguration>,
	language_clients: BTreeMap<String, lsp::Client>,
	debug_clients:    BTreeMap<String, dap::Client>,
	breakpoints:      BTreeMap<(String, String), Vec<Value>>,
}
impl Runtime {
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
			"lsp" => self.language(&arguments, call, approval, cancellation)?,
			"debug" => self.debug(&arguments, call, approval, cancellation)?,
			_ => bail!("unknown language service tool"),
		};
		let result = serde_json::to_string(&result)?;
		ensure!(
			result.len() <= harvest_runtime::MAX_TOOL_BYTES,
			"language service result exceeds host output limit"
		);
		Ok(result)
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
		if action == "status" {
			return Ok(
				json!({"servers":self.lsp_configs.iter().map(|server|json!({"name":server.process.name,"connected":self.language_clients.contains_key(&server.process.name),"extensions":server.extensions})).collect::<Vec<_>>() }),
			);
		}
		let file = arguments["file"].as_str();
		let configurations: Vec<_> = self
			.lsp_configs
			.iter()
			.filter(|server| {
				if let Some(name) = arguments["server"].as_str() {
					return server.process.name == name;
				}
				if let Some(file) = file.filter(|file| *file != "*") {
					let extension = Path::new(file)
						.extension()
						.and_then(|value| value.to_str())
						.unwrap_or("");
					return server.extensions.is_empty()
						|| server.extensions.iter().any(|value| {
							value
								.trim_start_matches('.')
								.eq_ignore_ascii_case(extension)
						});
				}
				true
			})
			.collect();
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
		if !self.language_clients.contains_key(&name) {
			self
				.language_clients
				.insert(name.clone(), lsp::Client::connect(&self.root, &configuration, cancellation)?);
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
				params["context"] = json!({"diagnostics":[]});
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
			let applied = client.apply(&edit, approval, cancellation)?;
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
