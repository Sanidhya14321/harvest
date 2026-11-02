//! The `read` tool for workspace text files, matching the legacy hashline
//! read contract: `path:selector` parsing, a `[path#TAG]` snapshot header
//! recorded in the shared edit store, `N:line` rows, the range context window,
//! out-of-range/continuation notices, and the 3000-line/50 KiB window.
use std::{fmt::Write as _, path::Path, sync::LazyLock};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use pi_edit::{
	EditStore,
	modes::hashline::format::{format_hashline_header, format_numbered_lines},
	path_policy::canonical_key,
};
use regex::Regex;
use serde::Deserialize;

const MAX_LINES: usize = 3000;
const MAX_BYTES: usize = 50 * 1024;
const LEADING_CONTEXT: usize = 1;
const TRAILING_CONTEXT: usize = 3;
const ENTITY: &str = "file";

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

/// An inclusive one-based range; `end: None` runs to EOF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
	start: usize,
	end:   Option<usize>,
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
fn split_path(raw: &str) -> (&str, Option<String>) {
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
fn parse_ranges(selector: &str) -> Result<Option<Vec<Range>>> {
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
	if lines.last() == Some(&"") {
		lines.pop();
	}
	lines
}

/// Head window: whole lines within 3000 lines and 50 KiB.
fn head_window(lines: &[&str]) -> (usize, bool) {
	let mut bytes = 0;
	for (index, line) in lines.iter().enumerate() {
		if index >= MAX_LINES {
			return (index, false);
		}
		let needed = line.len() + usize::from(index > 0);
		if bytes + needed > MAX_BYTES {
			return (index, true);
		}
		bytes += needed;
	}
	(lines.len(), false)
}

fn format_bytes(bytes: usize) -> String {
	if bytes < 1024 {
		format!("{bytes}B")
	} else {
		format!("{:.1}KB", bytes as f64 / 1024.0)
	}
}

pub(crate) fn read(
	root: &Path,
	args: &Args,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<String> {
	cancellation.heartbeat()?;
	let fs = JailedFs::open(root)?;
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
	if let Selector::Unsupported(kind) = selector {
		bail!("The :{kind} selector is not supported by this host yet");
	}
	let relative = fs.relative(Path::new(authored))?;
	let display = if relative.as_os_str().is_empty() {
		".".to_owned()
	} else {
		relative.display().to_string()
	};
	let bytes = fs.read(&relative, MAX_FILE_BYTES)?;
	if bytes.contains(&0) {
		bail!("{display} is not a text file");
	}
	let raw_text = String::from_utf8(bytes).with_context(|| format!("{display} is not UTF-8"))?;
	let mut text =
		pi_edit::text::normalize_to_lf(pi_edit::text::strip_bom(&raw_text).1).into_owned();
	// Notebooks read (and edit) as their editable cell view, not raw JSON.
	if pi_edit::notebook::is_notebook_path(&relative) && !matches!(selector, Selector::Raw) {
		text = pi_edit::notebook::notebook_to_editable_text(&text, &display)
			.map(|view| pi_edit::text::normalize_to_lf(&view).into_owned())
			.map_err(|error| anyhow::anyhow!("{error}"))?;
	}
	let raw = matches!(
		selector,
		Selector::Raw | Selector::Lines { raw: true, .. } | Selector::Tail { raw: true, .. }
	);
	let lines: Vec<&str> = if raw {
		text.split('\n').collect()
	} else {
		addressable_lines(&text)
	};
	let total = lines.len();
	let selector = match selector {
		Selector::Tail { count, raw } => {
			let start = total.saturating_sub(count) + 1;
			Selector::Lines { ranges: vec![Range { start, end: Some(start.max(total)) }], raw }
		},
		other => other,
	};
	let key = canonical_key(&fs.root().join(&relative));
	let header = (!raw).then(|| {
		let tag = store.record(&key, &text, None);
		(format_hashline_header(&display, &tag), tag)
	});
	let mut seen: Vec<u32> = Vec::new();
	let render = |start: usize, rows: &[&str], seen: &mut Vec<u32>| -> String {
		if raw {
			return rows.join("\n");
		}
		seen.extend((start..start + rows.len()).map(|line| line as u32));
		format_numbered_lines(&rows.join("\n"), start as u32)
	};

	let output = match &selector {
		Selector::Lines { ranges, .. } if ranges.len() > 1 => {
			let mut blocks = Vec::new();
			let mut notices = Vec::new();
			for range in ranges {
				if range.start > total {
					let bound = range
						.end
						.map_or_else(|| range.start.to_string(), |end| format!("{}-{end}", range.start));
					notices.push(format!(
						"[Range {bound} is beyond end of {ENTITY} ({total} lines total); skipped]"
					));
					continue;
				}
				let end = range.end.unwrap_or(total).min(total);
				blocks.push(render(range.start, &lines[range.start - 1..end], &mut seen));
			}
			let body = blocks.join(if raw { "\n\n…\n\n" } else { "\n…\n" });
			let body = match &header {
				Some((header, _)) if !body.is_empty() => format!("{header}\n{body}"),
				_ => body,
			};
			match (body.is_empty(), notices.is_empty()) {
				(_, true) => body,
				(true, false) => notices.join("\n"),
				(false, false) => format!("{body}\n{}", notices.join("\n")),
			}
		},
		_ => {
			let (offset, limit) = match &selector {
				Selector::Lines { ranges, .. } => {
					let first = ranges[0];
					(Some(first.start), first.end.map(|end| end - first.start + 1))
				},
				_ => (None, None),
			};
			let requested_start = offset.map_or(0, |offset| offset - 1);
			if requested_start >= total {
				let suggestion = if total == 0 {
					format!("The {ENTITY} is empty.")
				} else {
					format!("Use :1 to read from the start, or :{total} to read the last line.")
				};
				return Ok(format!(
					"Line {} is beyond end of {ENTITY} ({total} lines total). {suggestion}",
					requested_start + 1
				));
			}
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
			let window = &lines[start..end];
			let (shown, by_bytes) = head_window(window);
			let first = start + 1;
			let mut body = if shown == 0 && !window.is_empty() {
				let line_bytes = window[0].len();
				format!(
					"[Line {first} is {}, exceeds {} limit. Hashline output requires full lines; \
					 cannot emit an editable numbered preview for a truncated line.]",
					format_bytes(line_bytes),
					format_bytes(MAX_BYTES)
				)
			} else {
				render(first, &window[..shown], &mut seen)
			};
			if shown > 0
				&& let Some((header, _)) = &header
			{
				body = format!("{header}\n{body}");
			}
			if shown < window.len() && shown > 0 {
				let last = first + shown - 1;
				let limit = if by_bytes {
					format!(" ({} limit)", format_bytes(MAX_BYTES))
				} else {
					String::new()
				};
				let _ = write!(
					body,
					"\n\n[Showing lines {first}-{last} of {total}{limit}. Use :{} to continue]",
					last + 1
				);
			} else if let Some(limit) = limit
				&& requested_start + limit < total
			{
				let remaining = total - (requested_start + limit);
				let _ = write!(
					body,
					"\n\n[{remaining} more lines in {ENTITY}. Use :{} to continue]",
					requested_start + limit + 1
				);
			}
			body
		},
	};
	if let Some((_, tag)) = &header
		&& !seen.is_empty()
	{
		store.record_seen_lines(&key, tag, &seen);
	}
	Ok(output)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn run(root: &Path, path: &str, store: &EditStore) -> Result<String> {
		read(root, &Args { path: path.to_owned() }, store, &CancelToken::default())
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
		let (root, _) = fixture(20);
		let store = EditStore::new();
		let output = run(root.path(), "f.txt:5-6", &store)?;
		let body: Vec<&str> = output.lines().skip(1).collect();
		assert_eq!(body[..6], [
			"4:line 4", "5:line 5", "6:line 6", "7:line 7", "8:line 8", "9:line 9"
		]);
		assert!(output.ends_with("[14 more lines in file. Use :7 to continue]"), "{output}");
		assert_eq!(
			run(root.path(), "f.txt:raw:2-3", &store)?,
			"line 2\nline 3\n\n[18 more lines in file. Use :4 to continue]"
		);
		assert!(run(root.path(), "f.txt:-2", &store)?.ends_with("19:line 19\n20:line 20"));
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
	fn oversized_windows_truncate_on_whole_lines() -> Result<()> {
		let (root, _) = fixture(3500);
		let output = run(root.path(), "f.txt", &EditStore::new())?;
		assert!(
			output.ends_with("[Showing lines 1-3000 of 3500. Use :3001 to continue]"),
			"{output}"
		);
		Ok(())
	}
}
