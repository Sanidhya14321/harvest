//! Version 1 executable-plugin frames. Stdout carries only these JSONL values.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::{ToolCall, ToolSpec};

pub const VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = crate::process_transport::MAX_FRAME_BYTES;
pub const MAX_TOOLS: usize = 128;
pub const MAX_EVENTS: usize = 1024;
pub const MAX_HOST_CALLS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolError {
	pub code:    String,
	pub message: String,
}

/// Data for host rendering; executable callbacks and arbitrary HTML are absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
	Progress { message: String },
	Notification { title: String, message: String },
	Choices { title: String, choices: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
	Initialize { id: u64, versions: Vec<u32> },
	Initialized { id: u64, version: u32, name: String, tools: Vec<ToolSpec> },
	Call { id: u64, name: String, arguments: Value },
	Result { id: u64, value: Value },
	Error { id: u64, error: ProtocolError },
	Event { id: u64, event: Event },
	HostCall { id: u64, parent_id: u64, call: ToolCall },
	HostResult { id: u64, value: Option<String>, error: Option<ProtocolError> },
	Cancel { id: u64 },
	Shutdown,
}

pub(crate) fn read(reader: &mut impl std::io::BufRead) -> anyhow::Result<Option<Frame>> {
	crate::process_transport::read_frame(reader)
}
pub(crate) fn encode(frame: &Frame) -> anyhow::Result<Vec<u8>> {
	crate::process_transport::encode(frame)
}

pub const MAX_EVENT_BYTES: usize = 16 * 1024;
pub const MAX_EVENT_TOTAL_BYTES: usize = 1024 * 1024;
/// Shared author/host admission rule. Limits are applied before enqueueing or
/// rendering data; this validates declarative data, never executable UI code.
pub(crate) fn event_bytes(event: &Event) -> anyhow::Result<usize> {
	if let Event::Choices { choices, .. } = event {
		anyhow::ensure!(choices.len() <= 64, "too many plugin choices");
	}
	let mut output = harvest_runtime::BoundedWriter::new(MAX_EVENT_BYTES);
	serde_json::to_writer(&mut output, event)
		.map_err(|_| anyhow::anyhow!("plugin event exceeds byte limit"))?;
	Ok(output.into_inner().len())
}
