//! The `read` tool for workspace text files, matching the legacy hashline
//! read contract: `path:selector` parsing, a `[path#TAG]` snapshot header
//! recorded in the shared edit store, `N:line` rows, the range context window,
//! out-of-range/continuation notices, and the 3000-line/50 KiB window.
use std::{fmt::Write as _, path::Path, sync::LazyLock};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use pi_edit::{
	EditStore, modes::hashline::format::format_hashline_header, path_policy::canonical_key,
};
use regex::Regex;
use serde::Deserialize;

use crate::output::{ToolOutput, format_bytes};

const MAX_LINES: usize = 3000;
const MAX_BYTES: usize = 50 * 1024;
const LEADING_CONTEXT: usize = 1;
const TRAILING_CONTEXT: usize = 3;
/// `read.defaultLimit` default.
const DEFAULT_LIMIT: usize = 300;
/// `tools.outputMaxColumns` default.
const OUTPUT_MAX_COLUMNS: usize = 768;
const BINARY_SNIFF_BYTES: usize = 8192;

const RANGE_CHUNK: &str = r"L?\d+(?:(?:[-+]|\.\.)L?\d+|-|\.\.)?";
static FILE_SELECTOR: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(&format!(r"(?i)^(?:{RANGE_CHUNK}(?:,{RANGE_CHUNK})*|-\d+|raw|conflicts|img)$"))
		.expect("regex")
});
static RANGE_ONLY: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(&format!(r"(?i)^(?:{RANGE_CHUNK}(?:,{RANGE_CHUNK})*|-\d+)$")).expect("regex")
});
static RANGE_CHUNK_PARTS: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"(?i)^L?(\d+)(?:(\.\.|[-+])L?(\d+)?)?$").expect("regex"));
static TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-(\d+)$").expect("regex"));

#[derive(Debug, Deserialize)]
pub(crate) struct Args {
	path: String,
}

impl Args {
	/// Reads of `artifact://` are never re-spilled.
	pub(crate) fn is_artifact(&self) -> bool {
		self.path.to_ascii_lowercase().starts_with("artifact://")
	}
}

/// An inclusive one-based range; `end: None` runs to EOF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
	pub(crate) start: usize,
	pub(crate) end:   Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Selector {
	None,
	Raw,
	Lines {
		ranges: Vec<Range>,
		raw:    bool,
	},
	Tail {
		count: usize,
		raw:   bool,
	},
	/// Recognized by the legacy tool but not supported by this host yet.
	Unsupported(&'static str),
}

/// Split a trailing `:selector` (`splitPathAndSel`), including the compound
/// `range:raw` / `raw:range` form.
pub(crate) fn split_path(raw: &str) -> (&str, Option<String>) {
	let Some(colon) = raw.rfind(':').filter(|&index| index > 0) else {
		return (raw, None);
	};
	let candidate = &raw[colon + 1..];
	if !FILE_SELECTOR.is_match(candidate) {
		return (raw, None);
	}
	let mut base = &raw[..colon];
	let mut selector = candidate.to_owned();
	if let Some(inner) = base.rfind(':').filter(|&index| index > 0) {
		let inner_candidate = &base[inner + 1..];
		let is_raw = |text: &str| text.eq_ignore_ascii_case("raw");
		if (is_raw(inner_candidate) && RANGE_ONLY.is_match(candidate))
			|| (RANGE_ONLY.is_match(inner_candidate) && is_raw(candidate))
		{
			selector = format!("{inner_candidate}:{candidate}");
			base = &base[..inner];
		}
	}
	(base, Some(selector))
}

fn parse_chunk(chunk: &str) -> Result<Option<Range>> {
	let Some(parts) = RANGE_CHUNK_PARTS.captures(chunk) else {
		return Ok(None);
	};
	let start: usize = parts[1].parse().unwrap_or(usize::MAX);
	if start < 1 {
		bail!("Line selector 0 is invalid; lines are 1-indexed. Use :1.");
	}
	let rhs: Option<usize> = parts.get(3).and_then(|value| value.as_str().parse().ok());
	let end = match parts.get(2).map(|sep| sep.as_str()) {
		Some("+") => match rhs {
			Some(count) if count >= 1 => Some(start + count - 1),
			_ => bail!("Invalid range {start}+{}: count must be >= 1.", rhs.unwrap_or(0)),
		},
		Some("-" | "..") => match rhs {
			Some(end) if end < start => bail!("Invalid range {start}-{end}: end must be >= start."),
			other => other,
		},
		_ => None,
	};
	Ok(Some(Range { start, end }))
}

/// Parse a comma list, sorted with overlapping/adjacent ranges merged.
pub(crate) fn parse_ranges(selector: &str) -> Result<Option<Vec<Range>>> {
	let mut parsed = Vec::new();
	for chunk in selector.split(',') {
		match parse_chunk(chunk)? {
			Some(range) => parsed.push(range),
			None => return Ok(None),
		}
	}
	parsed.sort_by_key(|range| range.start);
	let mut merged: Vec<Range> = Vec::new();
	for current in parsed {
		let Some(last) = merged.last_mut() else {
			merged.push(current);
			continue;
		};
		let Some(last_end) = last.end else { continue };
		if current.start <= last_end + 1 {
			if current.end.is_none_or(|end| end > last_end) {
				last.end = current.end;
			}
			continue;
		}
		merged.push(current);
	}
	Ok(Some(merged))
}

fn parse_tail(selector: &str) -> Result<Option<usize>> {
	let Some(captures) = TAIL.captures(selector) else {
		return Ok(None);
	};
	let count: usize = captures[1].parse().unwrap_or(usize::MAX);
	if count < 1 {
		bail!("Tail selector -0 is invalid; use :-N with N >= 1 to read the last N lines.");
	}
	Ok(Some(count))
}

fn range_or_tail(chunk: &str, raw: bool) -> Result<Option<Selector>> {
	if let Some(ranges) = parse_ranges(chunk)? {
		return Ok(Some(Selector::Lines { ranges, raw }));
	}
	Ok(parse_tail(chunk)?.map(|count| Selector::Tail { count, raw }))
}

fn invalid(selector: &str) -> anyhow::Error {
	anyhow::anyhow!(
		"Invalid selector ':{selector}'. Use :N, :N-M, :N+K, :N- (open-ended), :-N (last N lines), \
		 a comma-separated list of ranges, :raw, :img for SVG rendering, or a range combined with \
		 raw (e.g. :raw:50-100)."
	)
}

/// `parseSel`.
pub(crate) fn parse_selector(selector: Option<&str>) -> Result<Selector> {
	let Some(selector) = selector.filter(|selector| !selector.is_empty()) else {
		return Ok(Selector::None);
	};
	if selector.contains(':') {
		let chunks: Vec<&str> = selector.split(':').collect();
		if let [a, b] = chunks.as_slice() {
			let (range, raw) = if a.eq_ignore_ascii_case("raw") {
				(Some(*b), true)
			} else if b.eq_ignore_ascii_case("raw") {
				(Some(*a), true)
			} else {
				(None, false)
			};
			if let Some(range) = range.filter(|_| raw)
				&& let Some(parsed) = range_or_tail(range, true)?
			{
				return Ok(parsed);
			}
		}
		let read_like = |chunk: &&str| {
			let lower = chunk.to_ascii_lowercase();
			matches!(lower.as_str(), "raw" | "conflicts" | "img")
				|| TAIL.is_match(chunk)
				|| Regex::new(r"^-\d+[-+]\d+$").is_ok_and(|re| re.is_match(chunk))
				|| parse_ranges(chunk).ok().flatten().is_some()
		};
		if chunks.iter().all(read_like) {
			return Err(invalid(selector));
		}
		return Ok(Selector::None);
	}
	match selector.to_ascii_lowercase().as_str() {
		"raw" => return Ok(Selector::Raw),
		"conflicts" => return Ok(Selector::Unsupported("conflicts")),
		"img" => return Ok(Selector::Unsupported("img")),
		_ => {},
	}
	Ok(range_or_tail(selector, false)?.unwrap_or(Selector::None))
}

fn addressable_lines(text: &str) -> Vec<&str> {
	let mut lines: Vec<&str> = text.split('\n').collect();
	if lines.len() > 1 && lines.last() == Some(&"") {
		lines.pop();
	}
	lines
}

/// `truncateLine`: cap at `max` UTF-16 units with a trailing `…`.
fn truncate_columns(line: &str, max: usize) -> Option<String> {
	if line.encode_utf16().count() <= max {
		return None;
	}
	let mut units = 0;
	let mut end = 0;
	for (index, ch) in line.char_indices() {
		if units + ch.len_utf16() > max {
			break;
		}
		units += ch.len_utf16();
		end = index + ch.len_utf8();
	}
	Some(format!("{}…", &line[..end]))
}

/// `isProbablyBinaryHeader`: NUL bytes or invalid UTF-8 in the first 8 KiB
/// (a multibyte sequence cut by the header boundary is tolerated).
fn looks_binary(bytes: &[u8]) -> bool {
	let header = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
	header.contains(&0)
		|| std::str::from_utf8(header).is_err_and(|error| error.error_len().is_some())
}

/// One rendered row: a numbered line or an elision marker.
enum Entry {
	Line(usize, String),
	Ellipsis,
}

/// `buildLineEntriesWithBlockContext`: visible spans plus off-window block
/// boundary lines, with `…` across gaps. `text_for` supplies display text.
fn block_entries(
	full: &[&str],
	spans: &[(usize, usize)],
	path: &str,
	source_text: Option<&str>,
	text_for: &dyn Fn(usize, &str) -> String,
) -> Vec<Entry> {
	let total = full.len();
	let mut normalized: Vec<(usize, usize)> = spans
		.iter()
		.map(|&(start, end)| (start.max(1), end.min(total)))
		.filter(|(start, end)| end >= start)
		.collect();
	normalized.sort_unstable();
	let mut merged: Vec<(usize, usize)> = Vec::new();
	for span in normalized {
		match merged.last_mut() {
			Some(previous) if span.0 <= previous.1 + 1 => previous.1 = previous.1.max(span.1),
			_ => merged.push(span),
		}
	}
	let visible: Vec<u32> = merged
		.iter()
		.flat_map(|&(start, end)| start..=end)
		.map(|line| line as u32)
		.collect();
	let block_source = pi_edit::diff_string::BlockContextSource { path: Some(path), lang: None };
	let context = match source_text {
		Some(text) => pi_edit::diff_string::find_block_context_lines_with_text(
			full,
			&visible,
			&block_source,
			text,
		),
		None => pi_edit::diff_string::find_block_context_lines(full, &visible, &block_source),
	};
	let mut lines: Vec<usize> = visible.iter().map(|&line| line as usize).collect();
	lines.extend(context.iter().map(|(line, _)| *line as usize));
	lines.sort_unstable();
	lines.dedup();
	let mut entries = Vec::new();
	let mut previous: Option<usize> = None;
	for line in lines {
		if previous.is_some_and(|previous| line > previous + 1) {
			entries.push(Entry::Ellipsis);
		}
		entries.push(Entry::Line(line, text_for(line, full.get(line - 1).copied().unwrap_or(""))));
		previous = Some(line);
	}
	entries
}

fn format_entries(entries: &[Entry], numbered: bool) -> String {
	entries
		.iter()
		.map(|entry| match entry {
			Entry::Ellipsis => "…".to_owned(),
			Entry::Line(line, text) if numbered => format!("{line}:{text}"),
			Entry::Line(_, text) => text.clone(),
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// Head-direction truncation notice (`formatTruncationMetaNotice`).
fn truncation_notice(
	first: usize,
	shown: usize,
	total: usize,
	by_bytes: Option<usize>,
	next: Option<usize>,
) -> String {
	let mut notice = if shown > 0 {
		format!("Showing lines {first}-{} of {total}", first + shown - 1)
	} else {
		format!("Showing {shown} of {total} lines")
	};
	if let Some(bytes) = by_bytes {
		let _ = write!(notice, " ({} limit)", format_bytes(bytes));
	}
	if let Some(next) = next {
		let _ = write!(notice, ". Use :{next} to continue");
	}
	notice
}

/// Byte-faithful views of one buffered file (`deriveBufferedFileText`).
struct Buffered<'a> {
	raw_segments: Vec<&'a str>,
	normalized:   String,
	ends_with_lf: bool,
}

impl<'a> Buffered<'a> {
	fn new(raw: &'a str, bytes: &[u8]) -> Self {
		let stripped = raw.strip_prefix('\u{feff}').unwrap_or(raw);
		Self {
			raw_segments: raw.split('\n').collect(),
			normalized:   pi_edit::text::normalize_to_lf(stripped).into_owned(),
			ends_with_lf: bytes.last() == Some(&b'\n'),
		}
	}

	fn addressable(&self) -> Vec<&str> {
		let mut lines: Vec<&str> = self.normalized.split('\n').collect();
		if lines.last() == Some(&"") && self.normalized.ends_with('\n') {
			lines.pop();
		}
		lines
	}

	const fn total_lines(&self, raw: bool) -> usize {
		let segments = self.raw_segments.len();
		if self.ends_with_lf && !raw && segments > 1 {
			segments - 1
		} else {
			segments
		}
	}
}

/// The window `collectLineWindowFromBuffer` collects.
struct Window {
	lines:            Vec<usize>,
	collected_bytes:  usize,
	stopped_by_bytes: bool,
	limit_line:       Option<(usize, usize)>,
	first_line_bytes: Option<usize>,
}

fn collect_window(
	segments: &[&str],
	total: usize,
	start: usize,
	max_lines: usize,
	max_bytes: usize,
) -> Window {
	let mut window = Window {
		lines:            Vec::new(),
		collected_bytes:  0,
		stopped_by_bytes: false,
		limit_line:       None,
		first_line_bytes: None,
	};
	for (index, segment) in segments.iter().enumerate().take(total).skip(start) {
		let length = segment.len();
		let separator = usize::from(!window.lines.is_empty());
		if window.lines.len() >= max_lines {
			break;
		} else if window.lines.is_empty() && length > max_bytes {
			window.stopped_by_bytes = true;
			window.limit_line = Some((index, length));
			window.first_line_bytes.get_or_insert(length);
			break;
		} else if !window.lines.is_empty() && window.collected_bytes + separator + length > max_bytes
		{
			window.stopped_by_bytes = true;
			window.limit_line = Some((index, length));
			break;
		}
		window.lines.push(index);
		window.collected_bytes += separator + length;
		window.first_line_bytes.get_or_insert(length);
		if window.lines.len() >= max_lines {
			break;
		}
	}
	window
}

struct ReadContext<'a> {
	store:     &'a EditStore,
	key:       std::path::PathBuf,
	display:   String,
	conflicts: &'a crate::conflict::ConflictHistory,
	absolute:  std::path::PathBuf,
}

impl ReadContext<'_> {
	fn header(&self, normalized: &str) -> (String, String) {
		let tag = self.store.record(&self.key, normalized, None);
		(format_hashline_header(&self.display, &tag), tag)
	}

	fn record_seen(&self, tag: &str, body: &str) {
		let lines = pi_edit::store::seen_lines_from_body(body);
		if !lines.is_empty() {
			self.store.record_seen_lines(&self.key, tag, &lines);
		}
	}
}

/// The local-file reader for buffered (≤ 4 MiB) text files.
fn read_file_text(
	context: &ReadContext<'_>,
	raw_text: &str,
	bytes: &[u8],
	selector: &Selector,
) -> ToolOutput {
	let raw = matches!(selector, Selector::Raw | Selector::Lines { raw: true, .. });
	let buffered = Buffered::new(raw_text, bytes);
	let total = buffered.total_lines(raw);
	let addressable = buffered.addressable();
	let numbered = !raw;
	let column_cap = if raw { 0 } else { OUTPUT_MAX_COLUMNS };
	let mut column_truncated = false;
	let display = |line: &str, truncated: &mut bool| -> String {
		if column_cap == 0 {
			return line.to_owned();
		}
		truncate_columns(line, column_cap).map_or_else(
			|| line.to_owned(),
			|text| {
				*truncated = true;
				text
			},
		)
	};

	if let Selector::Lines { ranges, .. } = selector
		&& ranges.len() > 1
	{
		let mut notices = Vec::new();
		let mut spans = Vec::new();
		let mut shown: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
		let mut blocks = Vec::new();
		for range in ranges {
			let start = range.start - 1;
			let requested = range.end.map_or(DEFAULT_LIMIT, |end| end - range.start + 1);
			let max_lines = requested.min(MAX_LINES);
			let lines: Vec<usize> = if numbered {
				(start..addressable.len()).take(max_lines).collect()
			} else {
				collect_window(
					&buffered.raw_segments,
					total,
					start,
					max_lines,
					MAX_BYTES.max(max_lines * 512),
				)
				.lines
			};
			let range_total = if numbered { addressable.len() } else { total };
			if start >= range_total {
				let bound = range
					.end
					.map_or_else(|| range.start.to_string(), |end| format!("{}-{end}", range.start));
				notices.push(format!(
					"[Range {bound} is beyond end of file ({range_total} lines total); skipped]"
				));
				continue;
			}
			if lines.is_empty() {
				continue;
			}
			let source = if numbered {
				&addressable
			} else {
				&buffered.raw_segments
			};
			let rows: Vec<String> = lines
				.iter()
				.map(|&index| display(source[index], &mut column_truncated))
				.collect();
			spans.push((range.start, range.start + rows.len() - 1));
			for (offset, row) in rows.iter().enumerate() {
				shown.insert(range.start + offset, row.clone());
			}
			if !numbered {
				blocks.push(rows.join("\n"));
			}
		}
		let mut output = if numbered && !spans.is_empty() {
			let entries = block_entries(
				&addressable,
				&spans,
				&context.display,
				Some(&buffered.normalized),
				&|line, source| {
					shown.get(&line).cloned().unwrap_or_else(|| {
						truncate_columns(source, column_cap).unwrap_or_else(|| source.to_owned())
					})
				},
			);
			format_entries(&entries, true)
		} else {
			blocks.join("\n\n…\n\n")
		};
		if numbered && !output.is_empty() {
			let (header, tag) = context.header(&buffered.normalized);
			context.record_seen(&tag, &output);
			output = format!("{header}\n{output}");
		}
		if !notices.is_empty() {
			output = if output.is_empty() {
				notices.join("\n")
			} else {
				format!("{output}\n{}", notices.join("\n"))
			};
		}
		let limits = if column_truncated {
			vec![format!("Some lines truncated to {column_cap} chars")]
		} else {
			vec![]
		};
		return ToolOutput { text: output, limits, ..ToolOutput::default() };
	}

	let (offset, limit) = match selector {
		Selector::Lines { ranges, .. } => {
			(Some(ranges[0].start), ranges[0].end.map(|end| end - ranges[0].start + 1))
		},
		_ => (None, None),
	};
	let requested_start = offset.map_or(0, |offset| offset - 1);
	let leading = if !raw && offset.is_some_and(|offset| offset > 1) {
		requested_start.min(LEADING_CONTEXT)
	} else {
		0
	};
	let trailing = if !raw && limit.is_some() {
		TRAILING_CONTEXT
	} else {
		0
	};
	let start = requested_start - leading;
	let first = start + 1;
	let effective_limit = limit.unwrap_or(DEFAULT_LIMIT);
	let max_lines = (effective_limit + leading + trailing).min(MAX_LINES);
	let max_bytes = MAX_BYTES.max(max_lines * 512);
	let window = collect_window(&buffered.raw_segments, total, start, max_lines, max_bytes);
	if requested_start >= total {
		let suggestion = if total == 0 {
			"The file is empty.".to_owned()
		} else {
			format!("Use :1 to read from the start, or :{total} to read the last line.")
		};
		return ToolOutput::text(format!(
			"Line {} is beyond end of file ({total} lines total). {suggestion}",
			requested_start + 1
		));
	}
	let rows: Vec<String> = window
		.lines
		.iter()
		.map(|&index| display(buffered.raw_segments[index], &mut column_truncated))
		.collect();
	let truncated = window.lines.len() < total - start || window.stopped_by_bytes;
	let first_line_exceeds = window
		.first_line_bytes
		.is_some_and(|bytes| bytes > max_bytes);
	let omitted = window.limit_line.filter(|&(index, _)| {
		!raw && leading > 0 && index == requested_start && window.lines.len() == leading
	});
	let hash = (numbered && !window.lines.is_empty() && !first_line_exceeds)
		.then(|| context.header(&buffered.normalized));
	let render = |rows: &[String]| -> String {
		let body = if numbered {
			let spans = [(first, first + rows.len().saturating_sub(1))];
			let entries = block_entries(
				&addressable,
				&spans,
				&context.display,
				Some(&buffered.normalized),
				&|line, source| {
					line
						.checked_sub(first)
						.and_then(|offset| rows.get(offset))
						.cloned()
						.unwrap_or_else(|| {
							truncate_columns(source, column_cap).unwrap_or_else(|| source.to_owned())
						})
				},
			);
			format_entries(&entries, true)
		} else {
			rows.join("\n")
		};
		match &hash {
			Some((header, _)) => format!("{header}\n{body}"),
			None => body,
		}
	};

	let mut truncation = None;
	let mut next_offset = None;
	let output = if first_line_exceeds {
		let line_bytes = window.first_line_bytes.unwrap_or(0);
		let shown = usize::from(!numbered);
		truncation =
			Some(truncation_notice(first, shown, total, Some(line_bytes.min(max_bytes)), None));
		format!(
			"[Line {first} is {}, exceeds {} limit. Hashline output requires full lines; cannot emit \
			 an editable numbered preview for a truncated line.]",
			format_bytes(line_bytes),
			format_bytes(max_bytes)
		)
	} else if truncated {
		let mut text = render(&rows);
		if let Some((index, length)) = omitted {
			let line = index + 1;
			let _ = write!(
				text,
				"\n\n[Line {line} is {} and could not fit after preceding context in the {} read \
				 budget. Use :raw:{line}-{line} to read that line without context (byte-capped if it \
				 exceeds the budget), or widen the requested range to increase the budget.]",
				format_bytes(length),
				format_bytes(max_bytes)
			);
		}
		let next = (omitted.is_none()).then_some(first + rows.len());
		let by_bytes = window.stopped_by_bytes.then_some(window.collected_bytes);
		next_offset = next;
		truncation = Some(truncation_notice(first, rows.len(), total, by_bytes, next));
		text
	} else {
		render(&rows)
	};
	if let Some((_, tag)) = &hash {
		context.record_seen(tag, &output);
	}
	let mut output = output;
	if !first_line_exceeds && !window.lines.is_empty() {
		let collected: Vec<&str> = window
			.lines
			.iter()
			.map(|&index| buffered.raw_segments[index])
			.collect();
		let blocks = crate::conflict::scan_lines(&collected, first);
		if !blocks.is_empty() {
			let entries: Vec<crate::conflict::Entry> = blocks
				.into_iter()
				.map(|block| {
					context
						.conflicts
						.register(&context.absolute, &context.display, block)
				})
				.collect();
			// Cheap full-file scan only once the window already showed a conflict.
			let (total, scan_truncated) = crate::conflict::scan_file(&context.absolute)
				.map_or((entries.len(), false), |(file, truncated)| {
					(entries.len().max(file.len()), truncated)
				});
			output.push_str(&crate::conflict::format_warning(
				&entries,
				total,
				&context.display,
				scan_truncated,
			));
		}
	}
	let limits = if column_truncated {
		vec![format!("Some lines truncated to {column_cap} chars")]
	} else {
		Vec::new()
	};
	ToolOutput { text: output, truncation, next_offset, limits }
}

/// The in-memory reader (`buildInMemoryTextResult`), used for notebooks.
fn read_in_memory(
	context: &ReadContext<'_>,
	text: &str,
	selector: &Selector,
	entity: &str,
) -> ToolOutput {
	let raw = matches!(selector, Selector::Raw | Selector::Lines { raw: true, .. });
	let all: Vec<&str> = if raw {
		text.split('\n').collect()
	} else {
		addressable_lines(text)
	};
	let total = all.len();
	let hash = (!raw).then(|| context.header(text));
	let numbered = !raw;
	let finish = |body: String, truncation: Option<(String, usize)>| -> ToolOutput {
		if let Some((_, tag)) = &hash {
			context.record_seen(tag, &body);
		}
		let (truncation, next_offset) = truncation.unzip();
		ToolOutput { text: body, truncation, next_offset, limits: Vec::new() }
	};
	let entries_for = |spans: &[(usize, usize)]| -> String {
		let entries =
			block_entries(&all, spans, &context.display, Some(text), &|_, source| source.to_owned());
		format_entries(&entries, true)
	};
	let prefix = |body: String| match &hash {
		Some((header, _)) if !body.is_empty() => format!("{header}\n{body}"),
		_ => body,
	};

	if let Selector::Lines { ranges, .. } = selector
		&& ranges.len() > 1
	{
		let mut spans = Vec::new();
		let mut parts = Vec::new();
		let mut notices = Vec::new();
		for range in ranges {
			if range.start > total {
				let bound = range
					.end
					.map_or_else(|| range.start.to_string(), |end| format!("{}-{end}", range.start));
				notices.push(format!(
					"[Range {bound} is beyond end of {entity} ({total} lines total); skipped]"
				));
				continue;
			}
			let end = range.end.unwrap_or(total).min(total);
			spans.push((range.start, end));
			parts.push(all[range.start - 1..end].join("\n"));
		}
		let body = if raw {
			parts.join("\n\n…\n\n")
		} else if spans.is_empty() {
			String::new()
		} else {
			prefix(entries_for(&spans))
		};
		let body = match (body.is_empty(), notices.is_empty()) {
			(_, true) => body,
			(true, false) => notices.join("\n"),
			(false, false) => format!("{body}\n{}", notices.join("\n")),
		};
		return finish(body, None);
	}
	let (offset, limit) = match selector {
		Selector::Lines { ranges, .. } => {
			(Some(ranges[0].start), ranges[0].end.map(|end| end - ranges[0].start + 1))
		},
		_ => (None, None),
	};
	let requested_start = offset.map_or(0, |offset| offset - 1);
	let requested_end = limit.map_or(total, |limit| (requested_start + limit).min(total));
	let start = if !raw && offset.is_some_and(|offset| offset > 1) {
		requested_start.saturating_sub(LEADING_CONTEXT)
	} else {
		requested_start
	};
	let end = if !raw && limit.is_some() {
		(requested_end + TRAILING_CONTEXT).min(total)
	} else {
		requested_end
	};
	if requested_start >= total {
		let suggestion = if total == 0 {
			format!("The {entity} is empty.")
		} else {
			format!("Use :1 to read from the start, or :{total} to read the last line.")
		};
		return ToolOutput::text(format!(
			"Line {} is beyond end of {entity} ({total} lines total). {suggestion}",
			requested_start + 1
		));
	}
	let window = &all[start..end];
	// truncateHead: whole lines within 3000 lines and 50 KiB.
	let content_bytes =
		window.iter().map(|line| line.len()).sum::<usize>() + window.len().saturating_sub(1);
	let (shown, by_bytes, first_exceeds) = if window.len() <= MAX_LINES && content_bytes <= MAX_BYTES
	{
		(window.len(), false, false)
	} else {
		let mut used = 0;
		let mut count = 0;
		let mut by_bytes = false;
		while count < MAX_LINES && count < window.len() {
			let needed = window[count].len() + usize::from(count > 0);
			if used + needed > MAX_BYTES {
				by_bytes = true;
				break;
			}
			used += needed;
			count += 1;
		}
		(count, by_bytes, count == 0)
	};
	let first = start + 1;
	let render = |rows: &[&str]| -> String {
		if numbered {
			prefix(entries_for(&[(first, first + rows.len().saturating_sub(1))]))
		} else {
			rows.join("\n")
		}
	};
	if first_exceeds {
		let bytes = window[0].len();
		return finish(
			format!(
				"[Line {first} is {}, exceeds {} limit. Hashline output requires full lines; cannot \
				 emit an editable numbered preview for a truncated line.]",
				format_bytes(bytes),
				format_bytes(MAX_BYTES)
			),
			None,
		);
	}
	if shown < window.len() {
		let output_bytes =
			window[..shown].iter().map(|line| line.len()).sum::<usize>() + shown.saturating_sub(1);
		let notice = truncation_notice(
			first,
			shown,
			total,
			by_bytes.then_some(output_bytes),
			Some(first + shown),
		);
		return finish(render(&window[..shown]), Some((notice, first + shown)));
	}
	let mut body = render(window);
	if limit.is_some() && end < total {
		let _ =
			write!(body, "\n\n[{} more lines in {entity}. Use :{} to continue]", total - end, end + 1);
	}
	finish(body, None)
}

pub(crate) fn read(
	root: &Path,
	args: &Args,
	store: &EditStore,
	artifacts: Option<&crate::output::ArtifactStore>,
	// Conflict-history integration into `read` is pending.
	conflicts: &crate::conflict::ConflictHistory,
	cancellation: &CancelToken,
) -> Result<ToolOutput> {
	cancellation.heartbeat()?;
	if args.is_artifact() {
		return read_artifact(&args.path, artifacts);
	}
	let fs = JailedFs::open(root)?;
	if let Some(uri) = crate::conflict::parse_uri(&args.path)? {
		let Some(id) = uri.id else {
			bail!(
				"Reading `conflict://*` is not supported — wildcards are write-only. Use the \
				 `<path>:conflicts` read selector for the full list of conflicts in a file, or read \
				 `conflict://<N>` to inspect a single block."
			);
		};
		return read_conflict_region(conflicts, store, id, uri.scope);
	}
	// A real file whose name ends in selector syntax keeps its colon.
	let literal_exists = || {
		fs.relative(Path::new(&args.path))
			.is_ok_and(|path| fs.exists(&path).unwrap_or(true))
	};
	let (authored, selector) = match split_path(&args.path) {
		(_, Some(_)) if literal_exists() => (args.path.as_str(), None),
		split => split,
	};
	let selector = parse_selector(selector.as_deref())?;
	let relative = fs.relative(Path::new(authored))?;
	match fs.file_type(&relative) {
		Err(_) if !fs.exists(&relative).unwrap_or(true) => bail!("Path '{authored}' not found"),
		Ok(kind) if kind.is_dir() => {
			if matches!(&selector, Selector::Lines { ranges, .. } if ranges.len() > 1) {
				bail!("Multi-range line selectors are not supported for directory listings.");
			}
			return Ok(read_directory(&fs.root().join(&relative), selector));
		},
		_ => {},
	}
	if matches!(selector, Selector::Unsupported("conflicts")) {
		let absolute = fs.root().join(&relative);
		let display = relative.display().to_string();
		let (blocks, scan_truncated) = crate::conflict::scan_file(&absolute)?;
		let entries: Vec<crate::conflict::Entry> = blocks
			.into_iter()
			.map(|block| conflicts.register(&absolute, &display, block))
			.collect();
		return Ok(ToolOutput::text(if entries.is_empty() {
			format!("No unresolved git merge conflicts in {display}.")
		} else {
			crate::conflict::format_summary(&entries, &display, scan_truncated)
		}));
	}
	if let Selector::Unsupported(kind) = selector {
		bail!("The :{kind} selector is not supported by this host yet");
	}
	let display = if relative.as_os_str().is_empty() {
		".".to_owned()
	} else {
		relative.display().to_string()
	};
	let bytes = fs.read(&relative, MAX_FILE_BYTES)?;
	let raw = matches!(
		selector,
		Selector::Raw | Selector::Lines { raw: true, .. } | Selector::Tail { raw: true, .. }
	);
	let absolute = fs.root().join(&relative);
	let context = ReadContext { store, key: canonical_key(&absolute), display, conflicts, absolute };

	if pi_edit::notebook::is_notebook_path(&relative) && !raw {
		let json =
			String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", context.display))?;
		let view = pi_edit::notebook::notebook_to_editable_text(&json, &context.display)
			.map_err(|error| anyhow::anyhow!("{error}"))?;
		let view = pi_edit::text::normalize_to_lf(&view).into_owned();
		let selector = resolve_tail(selector, addressable_lines(&view).len());
		return Ok(read_in_memory(&context, &view, &selector, "notebook"));
	}
	if !raw && looks_binary(&bytes) {
		return Ok(ToolOutput::text(format!(
			"[Cannot read binary file '{}' ({}); not valid UTF-8 text. Use ':raw' to read bytes \
			 verbatim.]",
			context.display,
			format_bytes(bytes.len())
		)));
	}
	let raw_text = String::from_utf8_lossy(&bytes).into_owned();
	if matches!(selector, Selector::None)
		&& bytes.len() <= MAX_SUMMARY_BYTES
		&& !is_prose(&relative)
		&& let Some(summary) = summarize(
			&context,
			&fs.root().join(&relative),
			&raw_text,
			&Buffered::new(&raw_text, &bytes).normalized,
		) {
		return Ok(summary);
	}
	let total = Buffered::new(&raw_text, &bytes).total_lines(raw);
	let selector = resolve_tail(selector, total);
	Ok(read_file_text(&context, &raw_text, &bytes, &selector))
}

const MAX_SUMMARY_BYTES: usize = 2 * 1024 * 1024;
const MAX_SUMMARY_LINES: usize = 20_000;
/// `read.summarize.*` defaults.
const SUMMARY_MIN_TOTAL_LINES: usize = 100;
const SUMMARY_MIN_BODY_LINES: u32 = 4;
const SUMMARY_MIN_COMMENT_LINES: u32 = 6;
const SUMMARY_UNFOLD_UNTIL: u32 = 50;
const SUMMARY_UNFOLD_LIMIT: u32 = 100;
const FOOTER_RANGE_SAMPLES: usize = 2;

/// Markdown and `.txt` skip summarization (`isProseSummaryPath`).
fn is_prose(path: &Path) -> bool {
	path
		.extension()
		.and_then(|ext| ext.to_str())
		.is_some_and(|ext| {
			matches!(
				ext.to_ascii_lowercase().as_str(),
				"md" | "markdown" | "mdx" | "mdc" | "mkd" | "mdown" | "txt"
			)
		})
}

/// `canMergeBracePair`.
fn can_merge_brace_pair(head: &str, tail: &str) -> bool {
	let head = head.trim_end();
	let tail = tail.trim();
	let closer = match head.chars().last() {
		Some('{') => '}',
		Some('(') => ')',
		Some('[') => ']',
		_ => return false,
	};
	tail.strip_prefix(closer).is_some_and(|rest| {
		rest
			.chars()
			.all(|ch| matches!(ch, ';' | ',' | ')' | ']' | '}'))
	})
}

/// A bare code read: tree-sitter summary with elided bodies
/// (`trySummarize` + `renderSummary` + `formatSummaryElisionFooter`), or
/// `None` when the file is unsuitable.
fn summarize(
	context: &ReadContext<'_>,
	absolute: &Path,
	raw_text: &str,
	normalized: &str,
) -> Option<ToolOutput> {
	let code = raw_text.strip_prefix('\u{feff}').unwrap_or(raw_text);
	let lines = if code.is_empty() {
		0
	} else {
		code.matches('\n').count() + 1
	};
	if !(SUMMARY_MIN_TOTAL_LINES..=MAX_SUMMARY_LINES).contains(&lines) {
		return None;
	}
	let summary = pi_ast::summary::summarize_code(pi_ast::summary::SummaryOptions {
		code:               code.to_owned(),
		lang:               None,
		path:               Some(absolute.to_string_lossy().into_owned()),
		min_body_lines:     Some(SUMMARY_MIN_BODY_LINES),
		min_comment_lines:  Some(SUMMARY_MIN_COMMENT_LINES),
		unfold_until_lines: Some(SUMMARY_UNFOLD_UNTIL),
		unfold_limit_lines: Some(SUMMARY_UNFOLD_LIMIT),
	})
	.ok()?;
	if !(summary.parsed && summary.elided) {
		return None;
	}
	enum Unit {
		Line(u32, String),
		Elided(u32, u32),
		Merged(u32, u32, String, String),
	}
	let mut flat = Vec::new();
	for segment in summary.segments {
		if segment.kind == "elided" {
			flat.push(Unit::Elided(segment.start_line, segment.end_line));
			continue;
		}
		let text = segment.text.unwrap_or_default();
		if text.is_empty() {
			continue;
		}
		for (offset, line) in text.split('\n').enumerate() {
			flat.push(Unit::Line(segment.start_line + offset as u32, line.to_owned()));
		}
	}
	let mut units: Vec<Unit> = Vec::new();
	let mut flat = flat.into_iter().peekable();
	while let Some(unit) = flat.next() {
		if let Unit::Elided(..) = unit
			&& let Some(Unit::Line(_, previous)) = units.last()
			&& let Some(Unit::Line(_, next)) = flat.peek()
			&& can_merge_brace_pair(previous, next)
		{
			let Some(Unit::Line(start, head)) = units.pop() else {
				unreachable!()
			};
			let Some(Unit::Line(end, tail)) = flat.next() else {
				unreachable!()
			};
			units.push(Unit::Merged(start, end, head, tail));
			continue;
		}
		units.push(unit);
	}
	let mut parts = Vec::new();
	let mut ranges: Vec<(u32, u32)> = Vec::new();
	let mut elided_lines = 0;
	for unit in &units {
		match unit {
			Unit::Elided(start, end) => {
				parts.push("…".to_owned());
				ranges.push((*start, *end));
				elided_lines += end - start + 1;
			},
			Unit::Merged(start, end, head, tail) => {
				parts.push(format!("{start}-{end}:{} … {}", head.trim_end(), tail.trim()));
				ranges.push((*start, *end));
				elided_lines += end.saturating_sub(*start + 1);
			},
			Unit::Line(line, text) => parts.push(format!("{line}:{text}")),
		}
	}
	let text = parts.join("\n");
	let samples = ranges.len().min(FOOTER_RANGE_SAMPLES);
	let selector = ranges[..samples]
		.iter()
		.map(|(start, end)| format!("{start}-{end}"))
		.collect::<Vec<_>>()
		.join(",");
	let example = format!("{}:{selector}", context.display);
	let footer = if ranges.is_empty() {
		String::new()
	} else if ranges.len() > samples {
		format!("[…{elided_lines}ln elided; re-read needed ranges, e.g. {example}]")
	} else {
		format!("[…{elided_lines}ln elided; re-read needed ranges with {example}]")
	};
	let (header, tag) = context.header(normalized);
	context.record_seen(&tag, &text);
	let body = if footer.is_empty() {
		text
	} else {
		format!("{text}\n\n{footer}")
	};
	Some(ToolOutput::text(format!("{header}\n{body}")))
}

/// `#readDirectory`: a depth-2 tree, 12 children per directory, sliced by
/// line selectors.
fn read_directory(absolute: &Path, selector: Selector) -> ToolOutput {
	let tree = crate::tree::build(absolute, 2, Some(12), None, None);
	let output = if tree.total_lines <= 1 {
		"(empty directory)".to_owned()
	} else {
		tree.rendered
	};
	let limits = if tree.truncated {
		vec!["1 results limit reached. Use limit=2 for more".to_owned()]
	} else {
		Vec::new()
	};
	let all: Vec<&str> = output.split('\n').collect();
	let selector = resolve_tail(selector, all.len());
	if let Selector::Lines { ranges, .. } = &selector {
		let start = ranges[0].start - 1;
		if start >= all.len() {
			return ToolOutput::text(format!(
				"Line {} is beyond end of listing ({} lines total). Use :1 to read from the start, or \
				 :{} to read the last line.",
				start + 1,
				all.len(),
				all.len()
			));
		}
		let end = ranges[0]
			.end
			.map_or(all.len(), |end| (start + end - ranges[0].start + 1).min(all.len()));
		let mut text = all[start..end].join("\n");
		if end < all.len() {
			let _ = write!(
				text,
				"\n\n[{} more lines in listing. Use :{} to continue]",
				all.len() - end,
				end + 1
			);
		}
		return ToolOutput { text, limits, ..ToolOutput::default() };
	}
	// truncateHead with no line cap: whole leading lines within 50 KiB.
	if output.len() <= MAX_BYTES {
		return ToolOutput { text: output, limits, ..ToolOutput::default() };
	}
	let mut used = 0;
	let mut shown = 0;
	for line in &all {
		let needed = line.len() + usize::from(shown > 0);
		if used + needed > MAX_BYTES {
			break;
		}
		used += needed;
		shown += 1;
	}
	ToolOutput {
		text: all[..shown].join("\n"),
		truncation: Some(truncation_notice(1, shown, all.len(), Some(used), Some(shown + 1))),
		next_offset: Some(shown + 1),
		limits,
	}
}

/// `read conflict://N[/scope]` (`#readConflictRegion`): the recorded block,
/// numbered at its original file lines under a fresh snapshot header.
fn read_conflict_region(
	conflicts: &crate::conflict::ConflictHistory,
	store: &EditStore,
	id: usize,
	scope: Option<crate::conflict::Scope>,
) -> Result<ToolOutput> {
	let Some(entry) = conflicts.get(id) else {
		bail!(
			"Conflict #{id} not found. Conflict ids are registered when `read` surfaces a marker \
			 block; re-read the file to get a current id."
		);
	};
	let (lines, start) = crate::conflict::render_region(&entry, scope)?;
	let body =
		pi_edit::modes::hashline::format::format_numbered_lines(&lines.join("\n"), start as u32);
	Ok(ToolOutput::text(match store.record_file(&entry.absolute_path, None) {
		Some(tag) => format!("{}\n{body}", format_hashline_header(&entry.display_path, &tag)),
		None => body,
	}))
}

/// `~` for the home directory (`shortenPath`).
fn shorten_path(path: &Path) -> String {
	let text = path.display().to_string();
	match std::env::var("HOME") {
		Ok(home) if !home.is_empty() && text.starts_with(&home) => {
			format!("~{}", &text[home.len()..])
		},
		_ => text,
	}
}

/// `read artifact://N[:selector]` (`#readArtifactFile`): immutable output, so
/// no hashline anchors and no line numbers.
fn read_artifact(
	path: &str,
	artifacts: Option<&crate::output::ArtifactStore>,
) -> Result<ToolOutput> {
	let rest = &path["artifact://".len()..];
	let (id, selector) = rest
		.split_once(':')
		.map_or((rest, None), |(id, selector)| (id, Some(selector)));
	if id.is_empty() {
		bail!("artifact:// URL requires a numeric ID: artifact://0");
	}
	if !id.bytes().all(|byte| byte.is_ascii_digit()) {
		bail!("artifact:// ID must be numeric, got: {id}");
	}
	let Some(store) = artifacts else {
		bail!("No session - artifacts unavailable")
	};
	let Ok(entries) = std::fs::read_dir(store.dir()) else {
		bail!("No artifacts directory found")
	};
	let names: Vec<String> = entries
		.filter_map(Result::ok)
		.filter_map(|entry| entry.file_name().into_string().ok())
		.collect();
	let prefix = format!("{id}.");
	let Some(file) = names
		.iter()
		.find(|name| name.starts_with(&prefix))
		.map(|name| store.dir().join(name))
	else {
		let mut available: Vec<u64> = names
			.iter()
			.filter_map(|name| name.split_once('.').map(|(head, _)| head))
			.filter_map(|head| head.parse().ok())
			.collect();
		available.sort_unstable();
		available.dedup();
		let available = if available.is_empty() {
			"none".to_owned()
		} else {
			available
				.iter()
				.map(u64::to_string)
				.collect::<Vec<_>>()
				.join(", ")
		};
		bail!("Artifact {id} not found. Available: {available}");
	};
	if file.is_dir() {
		bail!("Artifact {id} resolved to a directory, not a file");
	}
	let url = format!("artifact://{id}");
	let bytes = std::fs::read(&file).with_context(|| format!("could not read {url}"))?;
	let size = bytes.len();
	let selector = parse_selector(selector)?;
	if let Selector::Unsupported(kind) = selector {
		bail!("The :{kind} selector is not supported for artifacts");
	}
	if matches!(selector, Selector::Raw) && size > MAX_BYTES {
		return Ok(ToolOutput::text(format!(
			"Unbounded raw read blocked for {url} ({}). Reading the whole artifact verbatim can \
			 exhaust memory. Use {url}:raw:1-3000 for bounded verbatim chunks, {url}:1-3000 for \
			 numbered exploration, and the artifact file path for search/copy workflows: {}",
			format_bytes(size),
			shorten_path(&file)
		)));
	}
	let raw = matches!(
		selector,
		Selector::Raw | Selector::Lines { raw: true, .. } | Selector::Tail { raw: true, .. }
	);
	let text = String::from_utf8_lossy(&bytes).into_owned();
	let buffered = Buffered::new(&text, &bytes);
	let total = buffered.total_lines(raw);
	let selector = resolve_tail(selector, total);
	let workflow = (!raw && size > MAX_BYTES).then(|| {
		format!(
			"Artifact storage: {} ({}). Use {url}:N-M to page, {url}:raw:N-M for verbatim chunks, \
			 and the artifact file path for search/copy workflows.",
			shorten_path(&file),
			format_bytes(size)
		)
	});
	let with_workflow = |mut text: String| {
		if let Some(workflow) = &workflow {
			text = if text.is_empty() {
				workflow.clone()
			} else {
				format!("{text}\n\n[{workflow}]")
			};
		}
		text
	};

	if let Selector::Lines { ranges, .. } = &selector
		&& ranges.len() > 1
	{
		let addressable = buffered.addressable();
		let mut notices = Vec::new();
		let mut spans = Vec::new();
		let mut blocks = Vec::new();
		for range in ranges {
			let start = range.start - 1;
			let max_lines = range
				.end
				.map_or(DEFAULT_LIMIT, |end| end - range.start + 1)
				.min(MAX_LINES);
			let (range_total, lines): (usize, Vec<usize>) = if raw {
				let window = collect_window(
					&buffered.raw_segments,
					total,
					start,
					max_lines,
					MAX_BYTES.max(max_lines * 512),
				);
				(total, window.lines)
			} else {
				(addressable.len(), (start..addressable.len()).take(max_lines).collect())
			};
			if start >= range_total {
				let bound = range
					.end
					.map_or_else(|| range.start.to_string(), |end| format!("{}-{end}", range.start));
				notices.push(format!(
					"[Range {bound} is beyond end of file ({range_total} lines total); skipped]"
				));
				continue;
			}
			if lines.is_empty() {
				continue;
			}
			spans.push((range.start, range.start + lines.len() - 1));
			if raw {
				blocks.push(
					lines
						.iter()
						.map(|&index| buffered.raw_segments[index])
						.collect::<Vec<_>>()
						.join("\n"),
				);
			}
		}
		let mut output = if raw {
			blocks.join("\n\n…\n\n")
		} else if spans.is_empty() {
			String::new()
		} else {
			format_entries(
				&block_entries(&addressable, &spans, &url, Some(&buffered.normalized), &|_, source| {
					source.to_owned()
				}),
				false,
			)
		};
		if !notices.is_empty() {
			output = if output.is_empty() {
				notices.join("\n")
			} else {
				format!("{output}\n{}", notices.join("\n"))
			};
		}
		return Ok(ToolOutput::text(with_workflow(output)));
	}

	let (offset, limit) = match &selector {
		Selector::Lines { ranges, .. } => {
			(Some(ranges[0].start), ranges[0].end.map(|end| end - ranges[0].start + 1))
		},
		_ => (None, None),
	};
	let requested_start = offset.map_or(0, |offset| offset - 1);
	let leading = if !raw && offset.is_some_and(|offset| offset > 1) {
		requested_start.min(LEADING_CONTEXT)
	} else {
		0
	};
	let trailing = if !raw && limit.is_some() {
		TRAILING_CONTEXT
	} else {
		0
	};
	let start = requested_start - leading;
	let first = start + 1;
	let max_lines = (limit.unwrap_or(DEFAULT_LIMIT) + leading + trailing).min(MAX_LINES);
	let max_bytes = MAX_BYTES.max(max_lines * 512);
	let window = collect_window(&buffered.raw_segments, total, start, max_lines, max_bytes);
	if requested_start >= total {
		let suggestion = if total == 0 {
			"The artifact is empty.".to_owned()
		} else {
			format!("Use {url}:1 to read from the start, or {url}:{total} to read the last line.")
		};
		return Ok(ToolOutput::text(format!(
			"Line {} is beyond end of artifact ({total} lines total). {suggestion}",
			requested_start + 1
		)));
	}
	let rows: Vec<&str> = window
		.lines
		.iter()
		.map(|&index| buffered.raw_segments[index])
		.collect();
	let truncated = window.lines.len() < total - start || window.stopped_by_bytes;
	let first_line_exceeds = window
		.first_line_bytes
		.is_some_and(|bytes| bytes > max_bytes);
	let omitted = window.limit_line.filter(|&(index, _)| {
		!raw && leading > 0 && index == requested_start && window.lines.len() == leading
	});
	let mut output = ToolOutput::default();
	if first_line_exceeds {
		let line = buffered.raw_segments[start];
		let preview = {
			let mut end = max_bytes.min(line.len());
			while !line.is_char_boundary(end) {
				end -= 1;
			}
			&line[..end]
		};
		output.text = if preview.is_empty() {
			format!(
				"[Line {first} is {}, exceeds {} limit. Unable to display a valid UTF-8 snippet.]",
				format_bytes(line.len()),
				format_bytes(max_bytes)
			)
		} else {
			preview.to_owned()
		};
		output.truncation = Some(truncation_notice(
			first,
			usize::from(!preview.is_empty()),
			total,
			Some(max_bytes),
			None,
		));
	} else {
		output.text = rows.join("\n");
		if truncated {
			if let Some((index, length)) = omitted {
				let line = index + 1;
				let _ = write!(
					output.text,
					"\n\n[Line {line} is {} and could not fit after preceding context in the {} read \
					 budget. Use {url}:raw:{line}-{line} to read that line without context \
					 (byte-capped if it exceeds the budget), or widen the requested range to increase \
					 the budget.]",
					format_bytes(length),
					format_bytes(max_bytes)
				);
			}
			let next = omitted.is_none().then_some(first + rows.len());
			output.next_offset = next;
			let by_bytes = window.stopped_by_bytes.then_some(max_bytes);
			output.truncation = Some(truncation_notice(first, rows.len(), total, by_bytes, next));
		}
	}
	output.text = with_workflow(output.text);
	Ok(output)
}

/// Pin `:-N` to absolute lines once the line count is known.
fn resolve_tail(selector: Selector, total: usize) -> Selector {
	match selector {
		Selector::Tail { count, raw } => {
			let start = total.saturating_sub(count) + 1;
			Selector::Lines { ranges: vec![Range { start, end: Some(start.max(total)) }], raw }
		},
		other => other,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn run(root: &Path, path: &str, store: &EditStore) -> Result<String> {
		read(
			root,
			&Args { path: path.to_owned() },
			store,
			None,
			&crate::conflict::ConflictHistory::new(),
			&CancelToken::default(),
		)
		.map(ToolOutput::render)
	}

	fn fixture(lines: usize) -> (tempfile::TempDir, String) {
		let root = tempfile::tempdir().unwrap();
		let text = (1..=lines).fold(String::new(), |mut text, n| {
			let _ = writeln!(text, "line {n}");
			text
		});
		std::fs::write(root.path().join("f.txt"), &text).unwrap();
		(root, text)
	}

	#[test]
	fn whole_file_reads_carry_a_recorded_snapshot_header() -> Result<()> {
		let (root, text) = fixture(3);
		let store = EditStore::new();
		let output = run(root.path(), "f.txt", &store)?;
		let tag = pi_edit::store::file_hash(&text);
		assert_eq!(output, format!("[f.txt#{tag}]\n1:line 1\n2:line 2\n3:line 3"));
		let key = canonical_key(&root.path().canonicalize()?.join("f.txt"));
		assert_eq!(store.head(&key).map(|snapshot| snapshot.hash), Some(tag));
		Ok(())
	}

	#[test]
	fn ranges_add_context_and_continuation_notices() -> Result<()> {
		// Expected strings follow the legacy local-file reader (verified against
		// the TypeScript tool on the same fixture shape).
		let (root, _) = fixture(20);
		let store = EditStore::new();
		let output = run(root.path(), "f.txt:5-6", &store)?;
		let body: Vec<&str> = output.lines().skip(1).take(6).collect();
		assert_eq!(body, ["4:line 4", "5:line 5", "6:line 6", "7:line 7", "8:line 8", "9:line 9"]);
		assert!(output.ends_with("[Showing lines 4-9 of 20. Use :10 to continue]"), "{output}");
		assert_eq!(
			run(root.path(), "f.txt:raw:2-3", &store)?,
			"line 2\nline 3\n\n[Showing lines 2-3 of 21. Use :4 to continue]"
		);
		assert!(
			run(root.path(), "f.txt:-2", &store)?.ends_with("18:line 18\n19:line 19\n20:line 20")
		);
		Ok(())
	}

	#[test]
	fn multi_ranges_and_out_of_bounds_follow_the_legacy_wording() -> Result<()> {
		let (root, _) = fixture(5);
		let store = EditStore::new();
		let output = run(root.path(), "f.txt:1-1,3-3,9-10", &store)?;
		let lines: Vec<&str> = output.lines().collect();
		assert_eq!(lines[1..], [
			"1:line 1",
			"…",
			"3:line 3",
			"[Range 9-10 is beyond end of file (5 lines total); skipped]"
		]);
		assert_eq!(
			run(root.path(), "f.txt:9", &store)?,
			"Line 9 is beyond end of file (5 lines total). Use :1 to read from the start, or :5 to \
			 read the last line."
		);
		Ok(())
	}

	#[test]
	fn invalid_selectors_and_literal_colon_names() -> Result<()> {
		let (root, _) = fixture(2);
		let store = EditStore::new();
		let error = |path: &str| run(root.path(), path, &store).unwrap_err().to_string();
		assert_eq!(error("f.txt:0"), "Line selector 0 is invalid; lines are 1-indexed. Use :1.");
		assert_eq!(error("f.txt:5-2"), "Invalid range 5-2: end must be >= start.");
		assert_eq!(
			error("f.txt:-0"),
			"Tail selector -0 is invalid; use :-N with N >= 1 to read the last N lines."
		);
		std::fs::write(root.path().join("log:1-2"), "literal\n")?;
		assert!(run(root.path(), "log:1-2", &store)?.ends_with("1:literal"));
		Ok(())
	}

	#[test]
	fn bare_code_reads_summarize_and_name_the_elided_ranges() -> Result<()> {
		let root = tempfile::tempdir()?;
		let body: String = (0..20)
			.map(|n| format!("    let value_{n} = {n};\n"))
			.collect::<Vec<_>>()
			.concat();
		let source: String = (0..8)
			.map(|n| format!("pub fn f{n}() {{\n{body}}}\n\n"))
			.collect::<Vec<_>>()
			.concat();
		std::fs::write(root.path().join("lib.rs"), &source)?;
		let store = EditStore::new();
		let output = run(root.path(), "lib.rs", &store)?;
		assert!(output.starts_with("[lib.rs#"), "{output}");
		// Elided bodies collapse to `N-M:pub fn fK() { … }` and the footer names
		// re-readable ranges; the summary is much shorter than the source.
		assert!(output.lines().any(|line| line.ends_with("() { … }")), "{output}");
		assert!(output.contains("ln elided; re-read needed ranges"), "{output}");
		assert!(output.lines().count() < source.lines().count() / 2, "{output}");
		// A selector bypasses the summary; prose never summarizes.
		assert!(run(root.path(), "lib.rs:2-2", &store)?.contains("2:    let value_0 = 0;"));
		std::fs::write(root.path().join("notes.md"), &source)?;
		assert!(!run(root.path(), "notes.md", &store)?.contains(" … "));
		Ok(())
	}

	#[test]
	fn oversized_windows_truncate_on_whole_lines() -> Result<()> {
		let (root, _) = fixture(3500);
		let output = run(root.path(), "f.txt", &EditStore::new())?;
		assert!(output.ends_with("[Showing lines 1-300 of 3500. Use :301 to continue]"), "{output}");
		Ok(())
	}
}
