//! UTF-16 positions and reviewed capability-relative workspace text edits.
use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
	chat::{FunctionCall, ToolCall},
	tools::Approval,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Position {
	pub line:      u32,
	pub character: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Range {
	pub start: Position,
	pub end:   Position,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TextEdit {
	pub range:              Range,
	#[serde(rename = "newText")]
	pub new_text:           String,
	#[serde(rename = "insertTextFormat")]
	pub insert_text_format: Option<u32>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
	pub applied:                 bool,
	pub synchronization_pending: bool,
	pub changed_files:           Vec<PathBuf>,
	pub failure_reason:          Option<&'static str>,
	pub failed_file_index:       Option<usize>,
}

pub fn uri_path(filesystem: &JailedFs, uri: &str) -> Result<PathBuf> {
	let uri = reqwest::Url::parse(uri).map_err(|_| anyhow::anyhow!("invalid workspace edit URI"))?;
	ensure!(
		uri.scheme() == "file" && uri.query().is_none() && uri.fragment().is_none(),
		"workspace edit requires a local file URI"
	);
	let path = uri
		.to_file_path()
		.map_err(|()| anyhow::anyhow!("invalid local file URI"))?;
	// Pin and reuse the resolved path; never reopen an ambient lexical alias.
	filesystem.resource_path(&path)
}
pub fn position_offset(text: &str, position: Position) -> Result<usize> {
	let mut line_start = 0;
	for _ in 0..position.line {
		let next = text[line_start..]
			.find('\n')
			.context("LSP line is beyond the document")?;
		line_start += next + 1;
	}
	let line_end = text[line_start..]
		.find('\n')
		.map_or(text.len(), |offset| line_start + offset);
	let content_end =
		if line_end > line_start && text.as_bytes()[line_end - 1] == b'\r' && line_end < text.len() {
			line_end - 1
		} else {
			line_end
		};
	let mut units = 0;
	for (offset, character) in text[line_start..content_end].char_indices() {
		if units == position.character {
			return Ok(line_start + offset);
		}
		units += character.len_utf16() as u32;
		ensure!(units <= position.character, "LSP position splits a UTF-16 surrogate pair");
	}
	// LSP positions beyond the line end are clamped to that line's end.
	Ok(content_end)
}
pub fn end_position(text: &str) -> Position {
	let line = text.bytes().filter(|byte| *byte == b'\n').count() as u32;
	let tail = text.rsplit('\n').next().unwrap_or("");
	Position { line, character: tail.encode_utf16().count() as u32 }
}
pub fn apply_text_edits(text: &str, edits: &[TextEdit]) -> Result<String> {
	ensure!(edits.len() <= 10_000, "too many LSP text edits");
	let mut indexed: Vec<_> = edits.iter().enumerate().collect();
	indexed.sort_by(|(left_index, left), (right_index, right)| {
		right
			.range
			.start
			.cmp(&left.range.start)
			.then(right_index.cmp(left_index))
	});
	let mut unique: Vec<&TextEdit> = Vec::new();
	for (_, edit) in indexed {
		ensure!(
			edit.range.start <= edit.range.end && edit.insert_text_format != Some(2),
			"invalid or snippet-formatted LSP text edit"
		);
		if unique.last().is_some_and(|previous| {
			edit.range.start != edit.range.end
				&& previous.range == edit.range
				&& previous.new_text == edit.new_text
		}) {
			continue;
		}
		if let Some(later) = unique.last() {
			ensure!(edit.range.end <= later.range.start, "overlapping LSP text edits");
		}
		unique.push(edit);
	}
	let mut output = text.to_owned();
	let mut earliest_modified = text.len();
	for edit in unique {
		let start = position_offset(text, edit.range.start)?;
		let end = position_offset(text, edit.range.end)?;
		ensure!(
			start <= end && end <= earliest_modified && end <= output.len(),
			"invalid or overlapping LSP edit byte range"
		);
		let length = output
			.len()
			.checked_sub(end - start)
			.and_then(|length| length.checked_add(edit.new_text.len()))
			.context("LSP edit size overflow")?;
		ensure!(length <= MAX_FILE_BYTES, "LSP edit result exceeds file limit");
		output.replace_range(start..end, &edit.new_text);
		earliest_modified = start;
	}
	Ok(output)
}

pub fn apply_workspace_edit(
	root: &Path,
	edit: &Value,
	versions: &BTreeMap<String, i64>,
	approval: &dyn Approval,
	cancellation: &CancelToken,
) -> Result<ApplyResult> {
	cancellation.heartbeat()?;
	ensure!(edit.is_object(), "workspace edit must be an object");
	let filesystem = JailedFs::open(root)?;
	let mut grouped: BTreeMap<String, Vec<TextEdit>> = BTreeMap::new();
	if let Some(changes) = edit.get("changes") {
		for (uri, edits) in changes.as_object().context("invalid LSP changes mapping")? {
			uri_path(&filesystem, uri)?;
			grouped.entry(uri.clone()).or_default().extend(
				serde_json::from_value::<Vec<TextEdit>>(edits.clone())
					.map_err(|_| anyhow::anyhow!("invalid LSP text edits"))?,
			);
		}
	}
	if let Some(changes) = edit.get("documentChanges") {
		for change in changes.as_array().context("invalid LSP document changes")? {
			ensure!(
				change.get("kind").is_none(),
				"LSP create/rename/delete resource operations are not converted; no edits were applied"
			);
			let document = &change["textDocument"];
			let uri = document["uri"]
				.as_str()
				.context("LSP document edit has no URI")?;
			uri_path(&filesystem, uri)?;
			if let Some(version) = document.get("version").filter(|version| !version.is_null()) {
				ensure!(
					version
						.as_i64()
						.is_some_and(|version| versions.get(uri) == Some(&version)),
					"stale or unknown LSP document version"
				);
			}
			grouped.entry(uri.into()).or_default().extend(
				serde_json::from_value::<Vec<TextEdit>>(change["edits"].clone())
					.map_err(|_| anyhow::anyhow!("invalid LSP text edits"))?,
			);
		}
	}
	ensure!(grouped.len() <= 256, "too many workspace edit files");
	let mut plan = Vec::new();
	let mut paths = std::collections::BTreeSet::new();
	let mut bytes = 0;
	for (uri, edits) in &grouped {
		cancellation.heartbeat()?;
		let path = uri_path(&filesystem, uri)?;
		ensure!(paths.insert(path.clone()), "multiple LSP URI aliases target the same file");
		let original = filesystem.read(&path, MAX_FILE_BYTES)?;
		let text = std::str::from_utf8(&original).context("LSP target is not UTF-8")?;
		ensure!(!text.contains('\0'), "LSP target is not text");
		let changed = apply_text_edits(text, edits)?;
		bytes += original.len() + changed.len();
		ensure!(bytes <= 32 * 1024 * 1024, "workspace edit plan exceeds byte limit");
		plan.push((path, original, changed));
	}
	let final_arguments =
		serde_json::to_string(&json!({"action":"apply_workspace_edit","edit":edit}))?;
	ensure!(
		final_arguments.len() <= harvest_runtime::MAX_EVENT_BYTES,
		"workspace edit approval exceeds frame limit"
	);
	let call = ToolCall {
		id:       approval_id()?,
		kind:     "function".into(),
		function: FunctionCall { name: "lsp".into(), arguments: final_arguments },
	};
	if !approval.approve(&call)? {
		return Ok(ApplyResult {
			synchronization_pending: false,
			applied:                 false,
			changed_files:           Vec::new(),
			failure_reason:          Some("host_approval_denied"),
			failed_file_index:       None,
		});
	}
	cancellation.heartbeat()?;
	// Revalidate every source before the first write. Individual file replacement
	// is atomic; report the exact partial result if a later mutation fails.
	for (path, original, _) in &plan {
		ensure!(
			filesystem.read(path, MAX_FILE_BYTES)? == *original,
			"workspace edit target changed during approval"
		);
	}
	let mut result = ApplyResult {
		synchronization_pending: false,
		applied:                 true,
		changed_files:           Vec::new(),
		failure_reason:          None,
		failed_file_index:       None,
	};
	for (index, (path, original, changed)) in plan.iter().enumerate() {
		let write = (|| -> Result<()> {
			cancellation.heartbeat()?;
			ensure!(
				filesystem.read(path, MAX_FILE_BYTES)? == *original,
				"workspace edit target changed before write"
			);
			filesystem.atomic_write(path, changed.as_bytes())
		})();
		if write.is_err() {
			result.applied = false;
			result.failed_file_index = Some(index);
			result.failure_reason = Some("workspace_edit_write_failed_or_cancelled");
			break;
		}
		result.changed_files.push(path.clone());
		result.synchronization_pending = true;
	}
	Ok(result)
}

fn approval_id() -> Result<String> {
	static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
	let id = ID
		.try_update(
			std::sync::atomic::Ordering::Relaxed,
			std::sync::atomic::Ordering::Relaxed,
			|id| id.checked_add(1),
		)
		.map_err(|_| anyhow::anyhow!("LSP approval IDs exhausted"))?;
	Ok(format!("lsp-workspace-edit-{id}"))
}
