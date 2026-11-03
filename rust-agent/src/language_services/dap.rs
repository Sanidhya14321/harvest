use std::{
	collections::{BTreeMap, VecDeque},
	path::Path,
	time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use harvest_runtime::cancel::CancelToken;
use serde_json::{Value, json};

use super::configuration::DapConfiguration;
use crate::process_transport::Process;

pub struct Client {
	configuration:  DapConfiguration,
	process:        Process<Value>,
	next_seq:       u64,
	pending:        BTreeMap<u64, String>,
	responses:      BTreeMap<u64, Value>,
	events:         VecDeque<Value>,
	capabilities:   Value,
	exchange_bytes: usize,
	initialized:    bool,
	closed:         bool,
	owns_target:    bool,
	pub status:     String,
}
impl Client {
	pub fn connect(
		root: &Path,
		configuration: &DapConfiguration,
		cancellation: &CancelToken,
	) -> Result<Self> {
		configuration.validate()?;
		let process = configuration.process.launch(root, cancellation)?;
		let mut client = Self {
			configuration: configuration.clone(),
			process,
			next_seq: 1,
			pending: BTreeMap::new(),
			responses: BTreeMap::new(),
			events: VecDeque::new(),
			capabilities: json!({}),
			exchange_bytes: 0,
			initialized: false,
			closed: false,
			owns_target: false,
			status: "initializing".into(),
		};
		let result=client.request("initialize",json!({"clientID":"harvest-rust","clientName":"Harvest","adapterID":configuration.process.name,"linesStartAt1":true,"columnsStartAt1":true,"pathFormat":"path","supportsVariableType":true,"supportsVariablePaging":true,"supportsMemoryReferences":true,"supportsProgressReporting":true,"supportsRunInTerminalRequest":false,"supportsStartDebuggingRequest":false}),cancellation)?;
		ensure!(result.is_object(), "invalid DAP initialize capabilities");
		client.capabilities = result;
		client.status = "ready".into();
		Ok(client)
	}

	pub fn capabilities(&self) -> &Value {
		&self.capabilities
	}

	pub fn events(&mut self) -> Vec<Value> {
		self.events.drain(..).collect()
	}

	pub fn stderr_tail(&self) -> Vec<u8> {
		self.process.stderr_tail()
	}

	fn sequence(&mut self) -> Result<u64> {
		let sequence = self.next_seq;
		self.next_seq = sequence
			.checked_add(1)
			.filter(|value| *value <= 9_007_199_254_740_991)
			.context("DAP sequence exhausted")?;
		Ok(sequence)
	}

	fn send_request(
		&mut self,
		command: &str,
		arguments: Value,
		cancellation: &CancelToken,
	) -> Result<u64> {
		ensure!(
			!self.closed
				&& self.pending.len() < 32
				&& !command.is_empty()
				&& command.len() <= 256
				&& arguments.is_object(),
			"invalid DAP request or closed adapter"
		);
		let sequence = self.sequence()?;
		self.process.send(
			&json!({"seq":sequence,"type":"request","command":command,"arguments":arguments}),
			cancellation,
		)?;
		self.pending.insert(sequence, command.into());
		Ok(sequence)
	}

	fn pump(&mut self, cancellation: &CancelToken) -> Result<()> {
		let frame = self.process.receive(cancellation)?;
		self.accept(frame, cancellation)
	}

	fn accept(&mut self, frame: Value, cancellation: &CancelToken) -> Result<()> {
		self.exchange_bytes += serde_json::to_vec(&frame)?.len();
		ensure!(self.exchange_bytes <= 16 * 1024 * 1024, "DAP exchange exceeds byte limit");
		ensure!(frame["seq"].as_u64().is_some(), "invalid DAP sequence");
		match frame["type"].as_str() {
			Some("response") => {
				let sequence = frame["request_seq"]
					.as_u64()
					.context("DAP response has no request sequence")?;
				let command = self
					.pending
					.get(&sequence)
					.context("unexpected DAP response sequence")?;
				ensure!(
					frame["command"].as_str() == Some(command.as_str())
						&& frame["success"].is_boolean()
						&& !self.responses.contains_key(&sequence),
					"invalid or duplicate DAP response"
				);
				self.responses.insert(sequence, frame);
			},
			Some("event") => {
				let event = frame["event"].as_str().context("DAP event has no name")?;
				match event {
					"initialized" => self.initialized = true,
					"stopped" => self.status = "stopped".into(),
					"continued" => self.status = "running".into(),
					"terminated" | "exited" => {
						self.status = "terminated".into();
						self.initialized = false;
					},
					_ => {},
				}
				if self.events.len() == 32 {
					self.events.pop_front();
				}
				self.events.push_back(frame);
			},
			Some("request") => {
				let request_seq = frame["seq"].clone();
				let command = frame["command"]
					.as_str()
					.context("invalid DAP reverse request")?;
				let sequence = self.sequence()?;
				// Reverse terminal/debugger launching must not bypass host approval.
				self.process.send(&json!({"seq":sequence,"type":"response","request_seq":request_seq,"command":command,"success":false,"message":"Reverse adapter requests are not supported by this host"}),cancellation)?;
			},
			_ => bail!("invalid DAP message type"),
		}
		Ok(())
	}

	fn wait(&mut self, sequence: u64, cancellation: &CancelToken) -> Result<Value> {
		for _ in 0..1024 {
			cancellation.heartbeat()?;
			if let Some(response) = self.responses.remove(&sequence) {
				self.pending.remove(&sequence);
				if response["success"] != true {
					return Err(RequestFailed.into());
				}
				return Ok(response.get("body").cloned().unwrap_or_else(|| json!({})));
			}
			self.pump(cancellation)?;
		}
		bail!("DAP response exchange has too many messages")
	}

	fn failed_exchange(&mut self, sequence: Option<u64>) {
		if let Some(id) = sequence
			&& self.capabilities["supportsCancelRequest"] == true
		{
			if let Ok(sequence) = self.sequence() {
				self.process.try_send(
					&json!({"seq":sequence,"type":"request","command":"cancel","arguments":{"requestId":id}}),
				);
			}
		}
		self.closed = true;
		self.process.terminate();
	}

	pub fn request(
		&mut self,
		command: &str,
		arguments: Value,
		cancellation: &CancelToken,
	) -> Result<Value> {
		let deadline = cancellation.child_with_timeout(Duration::from_millis(u64::from(
			self.configuration.process.timeout_ms,
		)));
		if self.pending.is_empty() {
			self.exchange_bytes = 0;
		}
		let sequence = self.send_request(command, arguments, &deadline)?;
		let result = self.wait(sequence, &deadline);
		if let Err(error) = &result
			&& !error.is::<RequestFailed>()
		{
			self.failed_exchange(Some(sequence));
		}
		result
	}

	pub fn start(
		&mut self,
		attach: bool,
		arguments: Value,
		cancellation: &CancelToken,
	) -> Result<Value> {
		ensure!(
			matches!(self.status.as_str(), "ready" | "terminated"),
			"debug session is already active"
		);
		ensure!(arguments.is_object(), "debug target arguments must be an object");
		let defaults = if attach {
			&self.configuration.attach_defaults
		} else {
			&self.configuration.launch_defaults
		};
		let mut merged = defaults
			.as_object()
			.context("invalid debugger defaults")?
			.clone();
		merged.extend(
			arguments
				.as_object()
				.context("invalid debug arguments")?
				.clone(),
		);
		self.start_merged(attach, merged, cancellation)
	}

	fn start_merged(
		&mut self,
		attach: bool,
		arguments: serde_json::Map<String, Value>,
		cancellation: &CancelToken,
	) -> Result<Value> {
		let deadline = cancellation.child_with_timeout(Duration::from_millis(u64::from(
			self.configuration.process.timeout_ms,
		)));
		self.exchange_bytes = 0;
		let sequence = self.send_request(
			if attach { "attach" } else { "launch" },
			Value::Object(arguments),
			&deadline,
		)?;
		self.owns_target = !attach;
		self.status = "configuring".into();
		let result: Result<Value> = (|| {
			if self.capabilities["supportsConfigurationDoneRequest"] == true {
				for _ in 0..1024 {
					if self.initialized || self.responses.contains_key(&sequence) {
						break;
					}
					self.pump(&deadline)?;
				}
				if self.initialized {
					self.request("configurationDone", json!({}), &deadline)?;
				}
			}
			let result = self.wait(sequence, &deadline)?;
			if self.status == "configuring" {
				self.status = "running".into();
			}
			Ok(result)
		})();
		if let Err(error) = &result {
			if !error.is::<RequestFailed>() {
				self.failed_exchange(Some(sequence));
			} else {
				self.status = "ready".into();
			}
		}
		result
	}

	pub fn drain_events(&mut self, cancellation: &CancelToken) -> Result<Vec<Value>> {
		self.exchange_bytes = 0;
		for _ in 0..1024 {
			cancellation.heartbeat()?;
			let Some(frame) = self.process.try_receive()? else {
				break;
			};
			self.accept(frame, cancellation)?;
		}
		Ok(self.events())
	}
}
impl Drop for Client {
	fn drop(&mut self) {
		if !self.closed {
			if let Ok(sequence) = self.sequence() {
				self.process.try_send(&json!({"seq":sequence,"type":"request","command":"disconnect","arguments":{"terminateDebuggee":self.owns_target}}));
			}
		}
	}
}
#[derive(Debug)]
pub struct RequestFailed;
impl std::fmt::Display for RequestFailed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("debug adapter reported a failed request")
	}
}
impl std::error::Error for RequestFailed {}
