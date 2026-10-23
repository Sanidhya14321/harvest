//! One process owner, bounded reader/writer queues, deadline and cancellation.
use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
	time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::cancel::CancelToken;
use serde::Deserialize;
use serde_json::Value;

use super::protocol::{self, Event, Frame, ProtocolError};
use crate::{
	chat::{ToolCall, ToolSpec},
	tools::Approval,
};

fn default_timeout() -> u32 {
	30_000
}

/// Trusted host configuration; never constructed from model tool arguments.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
	pub executable:  PathBuf,
	#[serde(default)]
	pub arguments:   Vec<String>,
	#[serde(default)]
	pub environment: BTreeMap<String, String>,
	#[serde(default = "default_timeout")]
	pub timeout_ms:  u32,
}
impl std::fmt::Debug for Configuration {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PluginConfiguration")
			.field("executable", &self.executable)
			.field("timeout_ms", &self.timeout_ms)
			.finish_non_exhaustive()
	}
}

pub struct Plugin {
	process: crate::process_transport::Process<Frame>,
	tools:   Vec<ToolSpec>,
	next_id: u64,
	timeout: Duration,
}

impl Plugin {
	pub fn launch(
		root: &Path,
		configuration: &Configuration,
		cancellation: &CancelToken,
	) -> Result<Self> {
		cancellation.heartbeat()?;
		ensure!(
			(1..=600_000).contains(&configuration.timeout_ms),
			"plugin timeout must be between 1 and 600000 milliseconds"
		);
		ensure!(
			configuration.arguments.len() <= 128 && configuration.environment.len() <= 128,
			"plugin launch configuration exceeds limit"
		);
		let executable = if configuration.executable.is_absolute() {
			configuration.executable.clone()
		} else {
			root.join(&configuration.executable)
		};
		let executable = executable
			.canonicalize()
			.context("cannot resolve native plugin executable")?;
		let extension = executable
			.extension()
			.and_then(|v| v.to_str())
			.unwrap_or("")
			.to_ascii_lowercase();
		let stem = executable
			.file_stem()
			.and_then(|v| v.to_str())
			.unwrap_or("")
			.to_ascii_lowercase();
		ensure!(
			!matches!(
				extension.as_str(),
				"ts" | "tsx" | "js" | "mjs" | "cjs" | "py" | "jl" | "sh" | "ps1" | "cmd" | "bat"
			) && !matches!(
				stem.as_str(),
				"bun"
					| "node" | "nodejs"
					| "python" | "python3"
					| "julia" | "bash"
					| "sh" | "pwsh"
					| "powershell"
			),
			"language plugins are retired; build a native executable using Harvest's Rust plugin SDK \
			 (docs/rust-plugin-protocol.md)"
		);
		let process = crate::process_transport::Process::launch(
			root,
			&executable,
			&configuration.arguments,
			&configuration.environment,
			cancellation,
		)?;
		let mut plugin = Self {
			process,
			tools: Vec::new(),
			next_id: 1,
			timeout: Duration::from_millis(u64::from(configuration.timeout_ms)),
		};
		let deadline = cancellation.child_with_timeout(plugin.timeout);
		let initialize = plugin
			.send(&Frame::Initialize { id: 0, versions: vec![protocol::VERSION] }, &deadline)
			.and_then(|()| plugin.receive(&deadline));
		match initialize {
			Ok(Frame::Initialized { id: 0, version: protocol::VERSION, name, tools }) => {
				ensure!(
					!name.is_empty() && name.len() <= 128 && tools.len() <= protocol::MAX_TOOLS,
					"invalid plugin handshake identity or tool count"
				);
				let mut names = BTreeSet::new();
				for tool in &tools {
					tool.validate()?;
					ensure!(names.insert(tool.name()), "duplicate plugin tool name");
				}
				plugin.tools = tools;
				Ok(plugin)
			},
			_ => {
				plugin.terminate();
				bail!("plugin handshake failed; native protocol version 1 is required");
			},
		}
	}

	pub fn tools(&self) -> &[ToolSpec] {
		&self.tools
	}

	/// Host diagnostics only; never insert stderr into model output or stdout.
	pub fn stderr_tail(&self) -> Vec<u8> {
		self.process.stderr_tail()
	}

	fn send(&mut self, frame: &Frame, cancellation: &CancelToken) -> Result<()> {
		self.process.send(frame, cancellation)
	}

	fn receive(&mut self, cancellation: &CancelToken) -> Result<Frame> {
		self.process.receive(cancellation)
	}

	pub fn invoke(
		&mut self,
		root: &Path,
		call: &ToolCall,
		approval: &dyn Approval,
		cancellation: &CancelToken,
		on_event: &mut dyn FnMut(Event) -> Result<()>,
	) -> Result<String> {
		ensure!(
			!self.process.closed()
				&& self
					.tools
					.iter()
					.any(|tool| tool.name() == call.function.name),
			"plugin tool is unavailable"
		);
		ensure!(
			call.function.arguments.len() < protocol::MAX_FRAME_BYTES,
			"plugin arguments exceed limit"
		);
		let arguments: Value = serde_json::from_str(&call.function.arguments)
			.map_err(|_| anyhow::anyhow!("invalid plugin tool arguments"))?;
		ensure!(arguments.is_object(), "plugin arguments must be an object");
		ensure!(
			!call.id.is_empty() && call.id.len() <= 256,
			"plugin tool call identity exceeds limit"
		);
		cancellation.heartbeat()?;
		// All external execution needs final-argument host approval. Plugin metadata
		// cannot grant readonly privileges or authorize nested host mutations.
		ensure!(approval.approve(call)?, "plugin tool requires host approval");
		cancellation.heartbeat()?;
		let id = self.next_id;
		self.next_id = id
			.checked_add(1)
			.context("plugin request identity exhausted")?;
		let deadline = cancellation.child_with_timeout(self.timeout);
		let result = self.exchange(root, id, call, arguments, approval, &deadline, on_event);
		if result.is_err() {
			self.process.try_send(&Frame::Cancel { id });
			self.terminate();
		}
		result
	}

	fn exchange(
		&mut self,
		root: &Path,
		id: u64,
		call: &ToolCall,
		arguments: Value,
		approval: &dyn Approval,
		deadline: &CancelToken,
		on_event: &mut dyn FnMut(Event) -> Result<()>,
	) -> Result<String> {
		self.send(&Frame::Call { id, name: call.function.name.clone(), arguments }, deadline)?;
		let mut events = 0;
		let mut event_bytes = 0;
		let mut host_ids = BTreeSet::new();
		let mut bytes = 0_usize;
		loop {
			let (frame, wire_bytes) = self.process.receive_with_size(deadline)?;
			bytes = bytes
				.checked_add(wire_bytes)
				.context("plugin exchange byte count overflow")?;
			ensure!(bytes <= 16 * 1024 * 1024, "plugin response exchange exceeds limit");
			match frame {
				Frame::Result { id: response_id, value } if response_id == id => {
					deadline.heartbeat()?;
					return Ok(serde_json::to_string(&value)?);
				},
				Frame::Error { id: response_id, .. } if response_id == id => {
					bail!("plugin reported a structured execution error")
				},
				Frame::Event { id: response_id, event } if response_id == id => {
					events += 1;
					ensure!(events <= protocol::MAX_EVENTS, "too many plugin events");
					event_bytes += protocol::event_bytes(&event)?;
					ensure!(
						event_bytes <= protocol::MAX_EVENT_TOTAL_BYTES,
						"plugin event stream exceeds byte limit"
					);
					on_event(event)?;
				},
				Frame::HostCall { id: host_id, parent_id, call } if parent_id == id => {
					ensure!(
						host_ids.len() < protocol::MAX_HOST_CALLS && host_ids.insert(host_id),
						"duplicate or excessive plugin host calls"
					);
					let response =
						match crate::tools::invoke_with_cancellation(root, &call, approval, deadline) {
							Ok(value) => {
								Frame::HostResult { id: host_id, value: Some(value), error: None }
							},
							Err(_) => Frame::HostResult {
								id:    host_id,
								value: None,
								error: Some(ProtocolError {
									code:    "host_tool_failed".into(),
									message: "Host tool denied or failed".into(),
								}),
							},
						};
					self.send(&response, deadline)?;
				},
				_ => bail!("plugin returned an unexpected frame or request identity"),
			}
		}
	}

	fn terminate(&mut self) {
		self.process.terminate();
	}
}
impl Drop for Plugin {
	fn drop(&mut self) {
		self.process.try_send(&Frame::Shutdown);
	}
}
