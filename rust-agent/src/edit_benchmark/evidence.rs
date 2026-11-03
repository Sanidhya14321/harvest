//! Projection of actual persisted calls/results; never infer tool success from
//! assistant prose. Original messages and unknown provider fields stay
//! lossless.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Default, Serialize)]
pub struct ToolEvidence {
	/// Conservative: every call except read/grep, including unknown tool names.
	pub mutation_calls:   usize,
	/// All tool families, including grep and unknown names.
	pub unresolved_calls: usize,
	pub write_calls:      usize,
	pub read_calls:       usize,
	pub edit_calls:       usize,
	pub successful_reads: usize,
	pub successful_edits: usize,
	pub unchanged_edits:  usize,
	pub failed_reads:     usize,
	pub failed_edits:     usize,
	pub unresolved_reads: usize,
	pub unresolved_edits: usize,
}
#[derive(Deserialize)]
struct Message {
	role:     String,
	content:  Option<Box<RawValue>>,
	#[serde(rename = "toolCallId")]
	call_id:  Option<String>,
	#[serde(rename = "isError")]
	is_error: Option<bool>,
	details:  Option<Details>,
}
#[derive(Deserialize)]
struct Details {
	noop: Option<bool>,
}
#[derive(Deserialize)]
struct Block {
	#[serde(rename = "type")]
	kind: Option<String>,
	id:   Option<String>,
	name: Option<String>,
}
impl ToolEvidence {
	pub fn patch_applied(&self) -> Option<bool> {
		(self.edit_calls > 0).then_some(
			self.successful_edits > 0 && self.failed_edits == 0 && self.unresolved_edits == 0,
		)
	}
}
pub fn tool_evidence(messages: &[Box<RawValue>]) -> Result<ToolEvidence> {
	let mut result = ToolEvidence::default();
	let mut pending = BTreeMap::new();
	let mut pending_calls = BTreeSet::new();
	for raw in messages {
		let message: Message = serde_json::from_str(raw.get())?;
		if message.role == "assistant" {
			if let Some(content) = message.content {
				let blocks: Vec<Block> = serde_json::from_str(content.get())?;
				for block in blocks {
					if block.kind.as_deref() != Some("toolCall") {
						continue;
					}
					if let (Some(id), Some(name)) = (block.id, block.name) {
						ensure!(pending_calls.insert(id.clone()), "ambiguous pending tool call identity");
						if !matches!(name.as_str(), "read" | "grep") {
							result.mutation_calls += 1;
						}
						match name.as_str() {
							"read" => result.read_calls += 1,
							"edit" => result.edit_calls += 1,
							"write" => {
								result.write_calls += 1;
								continue;
							},
							_ => continue,
						}
						ensure!(
							pending.insert(id, name).is_none(),
							"ambiguous pending tool call identity"
						);
					}
				}
			}
		} else if message.role == "toolResult" {
			if let Some(id) = &message.call_id {
				pending_calls.remove(id);
			}
			if let Some(name) = message.call_id.and_then(|id| pending.remove(&id)) {
				match (name.as_str(), message.is_error) {
					("read", Some(false)) => result.successful_reads += 1,
					("read", Some(true)) => result.failed_reads += 1,
					("read", None) => result.unresolved_reads += 1,
					("edit", Some(false)) if message.details.is_some_and(|d| d.noop == Some(true)) => {
						result.unchanged_edits += 1
					},
					("edit", Some(false)) => result.successful_edits += 1,
					("edit", Some(true)) => result.failed_edits += 1,
					("edit", None) => result.unresolved_edits += 1,
					_ => {},
				}
			}
		}
	}
	for name in pending.values() {
		match name.as_str() {
			"read" => result.unresolved_reads += 1,
			"edit" => result.unresolved_edits += 1,
			_ => {},
		}
	}
	result.unresolved_calls = pending_calls.len();
	Ok(result)
}
