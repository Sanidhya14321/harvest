//! Oracle-assisted hashline guidance. Separate policy from historical scores;
//! patches never grant read provenance and are regenerated on every attempt.
use std::path::Path;

use anyhow::Result;
use harvest_runtime::{JailedFs, cancel::CancelToken, limits::BoundedWriter};
use pi_edit::{
	Resolved,
	files::FileRead,
	modes::hashline::{
		apply::{ApplyOptions, EmptyPaste, apply_edits},
		format::{
			compute_file_hash, format_cut_header, format_hashline_header, format_insert_header,
			format_replace_header, split_addressable_file_lines,
		},
		parser::parse_patch,
		types::{Anchor, Cursor},
	},
};
use serde_json::json;

use super::Task;

pub(super) fn with_guidance(
	prompt: &str,
	task: &Task,
	workspace: &JailedFs,
	artifacts: &JailedFs,
	attempt: u32,
	cancellation: &CancelToken,
) -> Result<String> {
	cancellation.heartbeat()?;
	let result = build(task, workspace, cancellation);
	cancellation.heartbeat()?;
	let (arguments, reason) = match result {
		Ok(Some(arguments)) => (Some(arguments), None),
		Ok(None) => (None, Some("no_change".to_owned())),
		Err(error) => (None, Some(format!("{error:#}"))),
	};
	// Expose bounded, deterministic skip evidence; never silently manufacture a
	// patch for unsupported/budgeted input or count omitted guidance as applied.
	let evidence = json!({"version":"rust-hashline-guidance-v1","attempt":attempt,
        "arguments":arguments,"skippedReason":reason});
	artifacts.atomic_write(
		Path::new(&format!("guidance-{attempt:04}.json")),
		&serde_json::to_vec(&evidence)?,
	)?;
	let Some(arguments) = arguments else {
		return Ok(prompt.to_owned());
	};
	let mut templates = handlebars::Handlebars::new();
	templates.register_escape_fn(handlebars::no_escape);
	let mut output = BoundedWriter::new(harvest_runtime::MAX_EVENT_BYTES);
	templates.render_template_to_write(
		include_str!("../prompts/benchmark-guidance.md"),
		&json!({"prompt":prompt,"arguments":serde_json::to_string_pretty(&arguments)?}),
		&mut output,
	)?;
	Ok(String::from_utf8(output.into_inner())?)
}

fn build(
	task: &Task,
	workspace: &JailedFs,
	cancellation: &CancelToken,
) -> Result<Option<serde_json::Value>> {
	use anyhow::{Context, ensure};
	let path = task.files.first().context("guidance has no target file")?;
	let display = path.to_str().context("guidance target is not UTF-8")?;
	ensure!(
		display.len() <= 4096
			&& !display
				.chars()
				.any(|c| c.is_control() || matches!(c, '[' | ']' | '#')),
		"guidance target cannot be represented as hashline"
	);
	let expected = JailedFs::open(&task.expected_directory)?;
	let load = |fs: &JailedFs| -> Result<FileRead> {
		let absolute = fs.resource_path(path)?;
		let raw = String::from_utf8(fs.read(&absolute, 256 * 1024)?)?;
		ensure!(!raw.contains('\0'), "guidance target is binary");
		Ok(FileRead::from_raw(Resolved { absolute, display: display.into() }, raw)?)
	};
	let actual = load(workspace)?;
	let expected = load(&expected)?;
	if actual.text == expected.text {
		return Ok(None);
	}
	// Grammar normalizes line boundaries. Do not advertise a patch which only
	// approximates an expected final-newline transformation.
	ensure!(
		actual.text.ends_with('\n') == expected.text.ends_with('\n'),
		"guidance final-newline change is unsupported"
	);
	let placements = super::hunks::placements_from_diff(&actual.text, &expected.text, cancellation)?;
	ensure!(!placements.is_empty() && placements.len() <= 25, "guidance operation limit exceeded");
	let count = split_addressable_file_lines(&actual.text).len();
	let mut body = String::new();
	for placement in placements {
		cancellation.heartbeat()?;
		let first = u32::try_from(placement.start + 1)?;
		let header = if placement.old_len == 0 {
			let cursor = if placement.start == 0 {
				Cursor::Bof
			} else if placement.start >= count {
				Cursor::Eof
			} else {
				Cursor::BeforeAnchor(Anchor { line: first })
			};
			format_insert_header(cursor)
		} else {
			let last = u32::try_from(placement.start + placement.old_len)?;
			if placement.new_lines.is_empty() {
				format_cut_header(first, last)
			} else {
				format_replace_header(first, last)
			}
		};
		body.push_str(&header);
		body.push('\n');
		for line in placement.new_lines {
			body.push('+');
			body.push_str(&line);
			body.push('\n');
		}
		ensure!(body.len() <= 16 * 1024, "guidance patch byte limit exceeded");
	}
	// Existing parser/application decides syntax and semantics. Never emit a
	// patch that repairs/normalizes to different bytes than the target view.
	let parsed = parse_patch(&body).map_err(|e| anyhow::anyhow!("{e:?}"))?;
	let applied = apply_edits(&actual.text, &parsed.edits, ApplyOptions {
		clipboard:      None,
		path:           Some(display),
		on_empty_paste: EmptyPaste::Throw,
	})?;
	ensure!(applied.text == expected.text, "guidance does not reproduce expected view exactly");
	let input =
		format!("{}\n{body}", format_hashline_header(display, &compute_file_hash(&actual.text)));
	let arguments = json!({"input":input});
	ensure!(
		serde_json::to_vec(&arguments)?.len() <= 20_000,
		"guidance argument byte limit exceeded"
	);
	Ok(Some(arguments))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		chat::{FunctionCall, ToolCall},
		edit::Mode,
		tools::{Approval, ToolRuntime},
	};
	struct Allow;
	impl Approval for Allow {
		fn approve(&self, _: &ToolCall) -> Result<bool> {
			Ok(true)
		}
	}
	#[test]
	fn guided_patch_requires_a_real_read_and_repairs_the_current_workspace() -> Result<()> {
		let root = tempfile::tempdir()?;
		std::fs::create_dir(root.path().join("workspace"))?;
		std::fs::create_dir(root.path().join("expected"))?;
		std::fs::create_dir(root.path().join("artifacts"))?;
		std::fs::write(root.path().join("workspace/a.txt"), "one\nwrong\nthree\n")?;
		std::fs::write(root.path().join("expected/a.txt"), "one\ntwo\nthree\n")?;
		let workspace = JailedFs::open(&root.path().join("workspace"))?;
		let artifacts = JailedFs::open(&root.path().join("artifacts"))?;
		let task = Task {
			id:                 "repair".into(),
			prompt:             "repair a.txt".into(),
			input_directory:    root.path().join("workspace"),
			expected_directory: root.path().join("expected"),
			files:              vec!["a.txt".into()],
			metadata:           None,
		};
		with_guidance(&task.prompt, &task, &workspace, &artifacts, 1, &Default::default())?;
		let evidence: serde_json::Value =
			serde_json::from_slice(&std::fs::read(root.path().join("artifacts/guidance-0001.json"))?)?;
		let edit = ToolCall {
			id:       "edit".into(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "edit".into(),
				arguments: evidence["arguments"].to_string(),
			},
		};
		let mut runtime = ToolRuntime::default();
		runtime.configure_edit(Mode::Hashline);
		assert!(
			runtime
				.invoke_detailed(workspace.root(), &edit, &Allow, &Default::default())?
				.is_error
		);
		let read = ToolCall {
			id:       "read".into(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "read".into(),
				arguments: json!({"path":"a.txt"}).to_string(),
			},
		};
		runtime.invoke_detailed(workspace.root(), &read, &Allow, &Default::default())?;
		assert!(
			!runtime
				.invoke_detailed(workspace.root(), &edit, &Allow, &Default::default())?
				.is_error
		);
		assert_eq!(std::fs::read_to_string(workspace.root().join("a.txt"))?, "one\ntwo\nthree\n");
		// A retry regenerates against landed bytes and records a no-change skip.
		with_guidance(&task.prompt, &task, &workspace, &artifacts, 2, &Default::default())?;
		let retry: serde_json::Value =
			serde_json::from_slice(&std::fs::read(root.path().join("artifacts/guidance-0002.json"))?)?;
		assert_eq!(retry["skippedReason"], "no_change");
		assert!(retry["arguments"].is_null());
		Ok(())
	}
}
