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

const OUTPUT_BYTES: usize = 20_000;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
	path:   String,
	/// One-based start, in the editable view (not raw notebook JSON).
	offset: Option<u32>,
	limit:  Option<u32>,
}

pub(crate) fn invoke(
	root: &Path,
	arguments: &str,
	mode: Mode,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<ToolOutput> {
	let args: Arguments =
		serde_json::from_str(arguments).context("read arguments are not valid JSON")?;
	ensure!(!args.path.is_empty() && args.path.len() <= 4096, "read path exceeds limit");
	let offset = args.offset.unwrap_or(1);
	let limit = args.limit.unwrap_or(u32::MAX);
	ensure!(offset > 0 && limit > 0, "read offset and limit must be positive");
	let fs = JailedFs::open(root)?;
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
			args.offset.is_none() && args.limit.is_none(),
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
	let start = (offset as usize).saturating_sub(1);
	ensure!(
		start < total || (start == 0 && read.text.is_empty()),
		"read offset is past end of file"
	);
	let end = total.min(start.saturating_add(limit as usize));
	if mode != Mode::Hashline && args.offset.is_none() && args.limit.is_none() && !read.is_notebook {
		// Preserve existing exact-replace default output, including BOM/CRLF.
		store.record(&read.canonical, &read.text, Some(&[]));
		return Ok(ToolOutput::text(crate::tools::bounded(&read.raw)));
	}
	let tag = compute_file_hash(&read.text);
	let mut output = String::new();
	let hashline = mode == Mode::Hashline;
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
	let mut next = start;
	for (index, line) in lines.iter().enumerate().take(end).skip(start) {
		cancellation.heartbeat()?;
		let number = u32::try_from(index + 1)?;
		let row = if hashline {
			format_numbered_line(number, line)
		} else {
			(*line).to_owned()
		};
		// Reserve space for the continuation notice. A partial source line is
		// never shown or marked seen, even for a single oversized source row.
		if output.len().saturating_add(row.len()).saturating_add(128) > OUTPUT_BYTES {
			break;
		}
		output.push_str(&row);
		output.push('\n');
		if hashline {
			shown.push(number);
		}
		next = index + 1;
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
			 "nextOffset":(next < total).then_some(next + 1),"totalLines":total,
			 "editableView":if read.is_notebook {"notebook"} else {"text"}
		})),
	})
}
