//! Bounded native read windows share the edit engine's view and snapshot keys.
use std::path::Path;

use anyhow::{Context, Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use pi_edit::{
	EditStore, Resolved,
	files::FileRead,
	modes::hashline::format::{
		compute_file_hash, format_hashline_header, format_numbered_line, split_addressable_file_lines,
	},
};
use serde::Deserialize;

use super::Mode;
use crate::tools::ToolOutput;

mod selector;

const OUTPUT_BYTES: usize = 20_000;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
	path:     String,
	selector: Option<String>,
	/// One-based start, in the editable view (not raw notebook JSON).
	offset:   Option<u32>,
	limit:    Option<u32>,
}

pub(crate) fn invoke(
	root: &Path,
	arguments: &str,
	mode: Mode,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<ToolOutput> {
	let mut args: Arguments =
		serde_json::from_str(arguments).context("read arguments are not valid JSON")?;
	ensure!(!args.path.is_empty() && args.path.len() <= 4096, "read path exceeds limit");
	let offset = args.offset.unwrap_or(1);
	let limit = args.limit.unwrap_or(u32::MAX);
	ensure!(offset > 0 && limit > 0, "read offset and limit must be positive");
	let fs = JailedFs::open(root)?;
	let (path, embedded) = selector::split(&fs, &args.path)?;
	ensure!(
		args.selector.is_none() || embedded.is_none(),
		"cannot combine embedded and explicit selectors"
	);
	let selected = args.selector.take().or(embedded);
	ensure!(
		selected.is_none() || (args.offset.is_none() && args.limit.is_none()),
		"cannot combine selector and offset/limit"
	);
	args.path = path;
	let selection = selected
		.as_deref()
		.map(selector::Selection::parse)
		.transpose()?
		.unwrap_or_default();
	let absolute = fs.resource_path(Path::new(&args.path))?;
	let bytes = fs.read(&absolute, MAX_FILE_BYTES)?;
	cancellation.heartbeat()?;
	if absolute
		.extension()
		.is_some_and(|ext| ext.eq_ignore_ascii_case("docx"))
	{
		// Extracted document text is not an editable source view. Never register
		// its displayed rows as authority to edit the underlying archive bytes.
		ensure!(
			args.offset.is_none() && args.limit.is_none() && selected.is_none(),
			"document reads do not accept line windows"
		);
		return Ok(ToolOutput::text(crate::tools::bounded(&harvest_media::docx::markdown(
			&bytes,
			cancellation,
		)?)));
	}
	ensure!(!bytes.contains(&0), "read target is not a text file");
	let raw = String::from_utf8(bytes).context("read target is not UTF-8")?;
	let read =
		FileRead::from_raw(Resolved { absolute: absolute.clone(), display: args.path.clone() }, raw)?;
	ensure!(
		fs.resource_path(Path::new(&args.path))? == absolute,
		"read path changed while loading; retry"
	);
	let lines = split_addressable_file_lines(&read.text);
	let total = lines.len();
	let ranges = selection.ranges(total, offset, limit)?;

	if mode != Mode::Hashline
		&& args.offset.is_none()
		&& args.limit.is_none()
		&& !read.is_notebook
		&& selected.is_none()
	{
		// Preserve existing exact-replace default output, including BOM/CRLF.
		store.record(&read.canonical, &read.text, Some(&[]));
		return Ok(ToolOutput::text(crate::tools::bounded(&read.raw)));
	}
	let tag = compute_file_hash(&read.text);
	let mut output = String::new();
	let hashline = mode == Mode::Hashline && !selection.raw;
	if hashline {
		ensure!(
			!args
				.path
				.chars()
				.any(|c| c.is_control() || matches!(c, '[' | ']' | '#')),
			"path cannot be represented in a hashline header; use another edit mode"
		);
		output.push_str(&format_hashline_header(&args.path, &tag));
		output.push('\n');
	}
	let mut shown = Vec::new();
	let mut next = 0usize;
	let mut complete = true;
	'ranges: for range in &ranges {
		let start = range.start_line as usize - 1;
		let end = range.end_line as usize;
		for (index, line) in lines.iter().enumerate().take(end).skip(start) {
			cancellation.heartbeat()?;
			let number = u32::try_from(index + 1)?;
			let row = if hashline {
				format_numbered_line(number, line)
			} else {
				(*line).to_owned()
			};
			if output.len().saturating_add(row.len()).saturating_add(128) > OUTPUT_BYTES {
				next = index;
				complete = false;
				break 'ranges;
			}
			output.push_str(&row);
			output.push('\n');
			if hashline {
				shown.push(number);
			}
			next = index + 1;
		}
	}

	if next < total {
		output.push_str(&format!("\n[Read continues at offset {} of {} lines.]", next + 1, total));
	}
	cancellation.heartbeat()?;
	// Snapshot first, then commit exactly the emitted complete row indices.
	// Do not infer provenance by parsing attacker-controlled file text.
	store.record(&read.canonical, &read.text, Some(&shown));
	Ok(ToolOutput {
		is_error: false,
		content:  output,
		details:  Some(serde_json::json!({
			 "path":args.path,"tag":tag,"offset":offset,"shownLines":shown,
		  "selectedRanges":ranges,"selectionComplete":complete,"raw":selection.raw,
			 "nextOffset":(next < total).then_some(next + 1),"totalLines":total,
			 "editableView":if read.is_notebook {"notebook"} else {"text"}
		})),
	})
}
