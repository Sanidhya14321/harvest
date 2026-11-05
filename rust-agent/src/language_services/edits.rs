//! UTF-16 positions and reviewed capability-relative workspace text edits.
use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::Approval;

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
	pub failed_change_index:     Option<usize>,
	pub executed_changes:        Vec<WorkspaceChange>,
}

/// Only mutations that actually landed; skipped resource operations are absent.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceChange {
	Edit {
		path: PathBuf,
	},
	Create {
		path: PathBuf,
	},
	Delete {
		path: PathBuf,
	},
	Rename {
		#[serde(rename = "oldPath")]
		old_path: PathBuf,
		#[serde(rename = "newPath")]
		new_path: PathBuf,
	},
}

impl WorkspaceChange {
	pub(crate) fn paths(&self) -> Vec<&Path> {
		match self {
			Self::Edit { path } | Self::Create { path } | Self::Delete { path } => {
				vec![path.as_path()]
			},
			Self::Rename { old_path, new_path } => vec![old_path.as_path(), new_path.as_path()],
		}
	}
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
	super::workspace::apply(root, edit, versions, approval, cancellation, None)
}
