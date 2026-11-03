//! Tool results, their bracketed notices, and the legacy artifact spill.
//!
//! A tool returns its body and its notices separately (`OutputMeta`). The host
//! then spills bodies over 50 KB into a session artifact, replacing the body
//! with a head + tail window and the tool's truncation notice with one that
//! references `artifact://N`, before appending the notices
//! (`formatOutputNotice`).
use std::{
	fmt::Write as _,
	io::Write as _,
	path::{Path, PathBuf},
	sync::LazyLock,
};

use anyhow::{Context, Result};
use regex::Regex;

/// `tools.artifactSpillThreshold` (KB).
const SPILL_THRESHOLD: usize = 50 * 1024;
/// `tools.artifactHeadBytes` / `tools.artifactTailBytes` (KB).
const HEAD_BYTES: usize = 20 * 1024;
const TAIL_BYTES: usize = 20 * 1024;
/// `tools.artifactTailLines`; the head window gets the same line budget.
const TAIL_LINES: usize = 500;

/// A tool result before notices are appended.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ToolOutput {
	pub text:        String,
	/// The truncation notice (rendered first).
	pub truncation:  Option<String>,
	/// `nextOffset` carried into a spill notice that replaces `truncation`.
	pub next_offset: Option<usize>,
	/// Limit notices, in `formatOutputNotice` order.
	pub limits:      Vec<String>,
}

impl ToolOutput {
	pub fn text(text: impl Into<String>) -> Self {
		Self { text: text.into(), ..Self::default() }
	}

	/// `appendOutputNotice`: notices joined by `. ` inside one bracket.
	pub fn render(self) -> String {
		let mut parts: Vec<String> = self.truncation.into_iter().collect();
		parts.extend(self.limits);
		let mut text = self.text;
		if !parts.is_empty() {
			let _ = write!(text, "\n\n[{}]", parts.join(". "));
		}
		text
	}
}

/// `Number#toFixed(1)` for non-negative values: exact binary ties round up
/// (Rust's formatter rounds them to even).
fn to_fixed_1(value: f64) -> String {
	let scaled = value * 10.0;
	if scaled.fract() == 0.5 && scaled < 9.0e15 {
		return format!("{:.1}", (scaled.ceil()) / 10.0);
	}
	format!("{value:.1}")
}

/// `formatBytes`.
pub fn format_bytes(bytes: usize) -> String {
	if bytes < 1024 {
		format!("{bytes}B")
	} else if bytes < 1024 * 1024 {
		format!("{}KB", to_fixed_1(bytes as f64 / 1024.0))
	} else if bytes < 1024 * 1024 * 1024 {
		format!("{}MB", to_fixed_1(bytes as f64 / (1024.0 * 1024.0)))
	} else {
		format!("{}GB", to_fixed_1(bytes as f64 / (1024.0 * 1024.0 * 1024.0)))
	}
}

/// `Number#toLocaleString` for non-negative integers.
fn grouped(value: usize) -> String {
	let digits = value.to_string();
	let mut out = String::new();
	for (index, ch) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index).is_multiple_of(3) {
			out.push(',');
		}
		out.push(ch);
	}
	out
}

/// Largest char-aligned head of `text` within `max` bytes.
fn head_bytes(text: &str, max: usize) -> &str {
	if text.len() <= max {
		return text;
	}
	let mut end = max;
	while !text.is_char_boundary(end) {
		end -= 1;
	}
	&text[..end]
}

/// Largest char-aligned tail of `text` within `max` bytes.
fn tail_bytes(text: &str, max: usize) -> &str {
	if text.len() <= max {
		return text;
	}
	let mut start = text.len() - max;
	while !text.is_char_boundary(start) {
		start += 1;
	}
	&text[start..]
}

/// One truncation outcome (`TruncationResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Truncation {
	content:       String,
	truncated:     bool,
	total_lines:   usize,
	total_bytes:   usize,
	output_lines:  usize,
	output_bytes:  usize,
	first_exceeds: bool,
	partial:       bool,
	last_partial:  bool,
	elided_lines:  usize,
	elided_bytes:  usize,
	head_lines:    Option<usize>,
	tail_lines:    Option<usize>,
}

fn untruncated(content: &str) -> Truncation {
	let lines = content.matches('\n').count() + 1;
	Truncation {
		content:       content.to_owned(),
		truncated:     false,
		total_lines:   lines,
		total_bytes:   content.len(),
		output_lines:  lines,
		output_bytes:  content.len(),
		first_exceeds: false,
		partial:       false,
		last_partial:  false,
		elided_lines:  0,
		elided_bytes:  0,
		head_lines:    None,
		tail_lines:    None,
	}
}

/// `truncateHead`: whole leading lines within the budgets.
fn truncate_head(content: &str, max_bytes: usize, max_lines: usize) -> Truncation {
	let mut result = untruncated(content);
	if result.total_lines <= max_lines && result.total_bytes <= max_bytes {
		return result;
	}
	let (mut lines, mut used, mut cut) = (0, 0, 0);
	let mut cursor = 0;
	while lines < max_lines {
		let line_end = content[cursor..]
			.find('\n')
			.map_or(content.len(), |offset| cursor + offset);
		let separator = usize::from(lines > 0);
		let length = line_end - cursor;
		if used + separator + length > max_bytes {
			if lines == 0 {
				result.content.clear();
				result.truncated = true;
				result.output_lines = 0;
				result.output_bytes = 0;
				result.first_exceeds = true;
				return result;
			}
			break;
		}
		used += separator + length;
		lines += 1;
		cut = line_end;
		if line_end == content.len() {
			break;
		}
		cursor = line_end + 1;
	}
	content[..cut].clone_into(&mut result.content);
	result.truncated = true;
	result.output_lines = lines;
	result.output_bytes = used;
	result
}

/// `truncateHead` with only a byte budget plus its `direction: "head"`
/// notice (`OutputMetaBuilder.truncation` + `formatTruncationMetaNotice`).
pub fn head_window(content: &str, max_bytes: usize) -> (String, Option<String>) {
	let result = truncate_head(content, max_bytes, usize::MAX);
	if !result.truncated {
		return (result.content, None);
	}
	if result.first_exceeds {
		let notice = format!(
			"Showing line 1 (partial, {} of {}) of {}",
			format_bytes(result.output_bytes),
			format_bytes(result.total_bytes),
			result.total_lines
		);
		return (result.content, Some(notice));
	}
	let end = result.output_lines;
	let notice = format!(
		"Showing lines 1-{end} of {} ({} limit). Use :{} to continue",
		result.total_lines,
		format_bytes(result.output_bytes),
		end + 1
	);
	(result.content, Some(notice))
}

/// `truncateTail`: whole trailing lines within the budgets, or a byte window
/// of an oversized last line.
fn truncate_tail(content: &str, max_bytes: usize, max_lines: usize) -> Truncation {
	let mut result = untruncated(content);
	if result.total_lines <= max_lines && result.total_bytes <= max_bytes {
		return result;
	}
	let (mut lines, mut used) = (0, 0);
	let mut start = content.len();
	let mut end = content.len();
	while lines < max_lines {
		let line_start = content[..end].rfind('\n').map_or(0, |index| index + 1);
		let separator = usize::from(lines > 0);
		let Some(remaining) = max_bytes.checked_sub(used + separator) else {
			break;
		};
		let line = &content[line_start..end];
		if line.len() <= remaining {
			used += separator + line.len();
			lines += 1;
			start = line_start;
			if line_start == 0 {
				break;
			}
			end = line_start - 1;
			continue;
		}
		if line.len() > max_bytes || lines == 0 {
			let window = tail_bytes(line, remaining);
			if !window.is_empty() || lines == 0 {
				let complete = if lines > 0 { &content[start..] } else { "" };
				let separator = if lines > 0 && !window.is_empty() {
					"\n"
				} else {
					""
				};
				result.content = format!("{window}{separator}{complete}");
				result.truncated = true;
				result.output_lines = lines + usize::from(!window.is_empty());
				result.output_bytes = used + separator.len() + window.len();
				result.partial = !window.is_empty();
				result.last_partial = !window.is_empty() && lines == 0;
				return result;
			}
		}
		break;
	}
	content[start..].clone_into(&mut result.content);
	result.truncated = true;
	result.output_lines = lines;
	result.output_bytes = used;
	result
}

fn elision_marker(lines: usize, bytes: usize) -> String {
	if lines <= 1 {
		format!("[…{bytes}B elided…]")
	} else {
		format!("[…{lines}ln elided…]")
	}
}

/// `truncateMiddle` with the spill budgets: a head and a tail window around an
/// elision marker.
fn truncate_middle(content: &str) -> Truncation {
	let (max_bytes, max_lines) = (HEAD_BYTES + TAIL_BYTES, TAIL_LINES * 2);
	let mut result = untruncated(content);
	if result.total_bytes <= max_bytes && result.total_lines <= max_lines {
		return result;
	}
	let head = truncate_head(content, HEAD_BYTES, TAIL_LINES);
	let tail = truncate_tail(content, TAIL_BYTES, TAIL_LINES);
	let tail_partial = tail.partial || tail.last_partial;
	if head.output_lines == 0 || head.first_exceeds || tail.output_lines == 0 {
		let byte_head = head.output_lines == 0 || head.first_exceeds;
		let head_text = if byte_head {
			head_bytes(content, HEAD_BYTES.min(content.len()))
		} else {
			&head.content
		};
		let head_count = if byte_head { 1 } else { head.output_lines };
		let remaining = content.len() - head_text.len();
		let byte_tail = tail.output_lines == 0;
		let tail_text = if byte_tail {
			tail_bytes(content, TAIL_BYTES.min(remaining)).to_owned()
		} else {
			truncate_tail(content, TAIL_BYTES.min(remaining), TAIL_LINES).content
		};
		let tail_count = if byte_tail {
			1
		} else {
			tail.output_lines.min(TAIL_LINES)
		};
		let elided = content
			.len()
			.saturating_sub(head_text.len() + tail_text.len());
		if elided == 0 {
			return result;
		}
		let composed = format!("{head_text}\n{}\n{tail_text}", elision_marker(0, elided));
		result.output_lines = head_count + tail_count + 1;
		result.output_bytes = composed.len();
		result.content = composed;
		result.truncated = true;
		result.elided_lines = result.total_lines.saturating_sub(head_count + tail_count);
		result.elided_bytes = elided;
		result.head_lines = Some(head_count);
		result.tail_lines = Some(tail_count);
		result.partial = byte_head || byte_tail || tail_partial;
		return result;
	}
	if head.output_lines + tail.output_lines >= result.total_lines && !tail_partial {
		return result;
	}
	let elided_lines = result.total_lines - head.output_lines - tail.output_lines;
	let elided_bytes = result
		.total_bytes
		.saturating_sub(head.output_bytes + tail.output_bytes);
	let marker = elision_marker(elided_lines, elided_bytes);
	let composed = format!("{}\n{marker}\n{}", head.content, tail.content);
	result.output_lines = head.output_lines + tail.output_lines + 1;
	result.output_bytes = head.output_bytes + tail.output_bytes + marker.len() + 2;
	result.content = composed;
	result.truncated = true;
	result.elided_lines = elided_lines;
	result.elided_bytes = elided_bytes;
	result.head_lines = Some(head.output_lines);
	result.tail_lines = Some(tail.output_lines);
	result.partial = tail_partial;
	result
}

/// Middle-direction notice (`formatTruncationMetaNotice`).
fn middle_notice(
	truncation: &Truncation,
	next_offset: Option<usize>,
	artifact: Option<&str>,
) -> String {
	let total = truncation.total_lines;
	let kept = truncation.output_lines.saturating_sub(1);
	let head = truncation.head_lines.unwrap_or_else(|| kept.div_ceil(2));
	let tail = truncation.tail_lines.unwrap_or(kept - head);
	let ranges = total > 1 && !truncation.partial;
	let head_part = (ranges && head > 0).then(|| format!("lines 1-{head}"));
	let tail_part = (ranges && tail > 0).then(|| format!("{}-{total}", total - tail + 1));
	let mut notice = match (head_part, tail_part) {
		(Some(head), Some(tail)) => format!(
			"Showing {head} and {tail} of {total}; {} middle line{} ({}) elided",
			grouped(truncation.elided_lines),
			if truncation.elided_lines == 1 {
				""
			} else {
				"s"
			},
			format_bytes(truncation.elided_bytes)
		),
		_ if truncation.elided_bytes > 0 => format!(
			"Showing head and tail bytes of {} line{}; {} elided",
			grouped(total),
			if total == 1 { "" } else { "s" },
			format_bytes(truncation.elided_bytes)
		),
		_ => {
			format!("Showing {} of {total} lines; middle elided", truncation.output_lines.min(total))
		},
	};
	if let Some(next) = next_offset {
		let _ = write!(notice, ". Use :{next} to continue");
	}
	if let Some(id) = artifact {
		let _ = write!(notice, ". Read artifact://{id} for full output");
	}
	notice
}

static ARTIFACT_NAME: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^(\d+)\..*\.log$").expect("regex"));

/// Session-scoped artifact storage beside the session file
/// (`<session>.jsonl` → `<session>/<id>.<tool>.log`).
#[derive(Debug, Clone)]
pub struct ArtifactStore {
	dir: PathBuf,
}

impl ArtifactStore {
	/// The store for a session file; `None` when it is not a `.jsonl` path.
	pub fn for_session(session_file: &Path) -> Option<Self> {
		let name = session_file.to_str()?.strip_suffix(".jsonl")?;
		Some(Self { dir: PathBuf::from(name) })
	}

	pub fn dir(&self) -> &Path {
		&self.dir
	}

	fn sanitize(tool: &str) -> String {
		static DISALLOWED: LazyLock<Regex> =
			LazyLock::new(|| Regex::new("[^A-Za-z0-9_-]+").expect("regex"));
		let replaced = DISALLOWED.replace_all(tool, "_");
		let capped: String = replaced.chars().take(64).collect();
		let trimmed = capped.trim_matches('_');
		if trimmed.is_empty() {
			"tool".to_owned()
		} else {
			trimmed.to_owned()
		}
	}

	fn next_id(&self) -> usize {
		std::fs::read_dir(&self.dir)
			.into_iter()
			.flatten()
			.filter_map(Result::ok)
			.filter_map(|entry| {
				let name = entry.file_name();
				ARTIFACT_NAME.captures(name.to_str()?)?[1]
					.parse::<usize>()
					.ok()
			})
			.max()
			.map_or(0, |max| max + 1)
	}

	/// Atomically save `content` and return its id.
	pub fn save(&self, content: &str, tool: &str) -> Result<String> {
		std::fs::create_dir_all(&self.dir).context("could not create the artifact directory")?;
		let id = self.next_id().to_string();
		let path = self.dir.join(format!("{id}.{}.log", Self::sanitize(tool)));
		let nonce = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |elapsed| elapsed.as_nanos());
		let staged = self
			.dir
			.join(format!(".{id}.tmp-{}-{nonce}", std::process::id()));
		let written = (|| -> Result<()> {
			let mut file = std::fs::File::create_new(&staged)?;
			file.write_all(content.as_bytes())?;
			file.sync_all()?;
			if file.metadata()?.len() != content.len() as u64 {
				anyhow::bail!("Artifact write incomplete");
			}
			std::fs::rename(&staged, &path)?;
			Ok(())
		})();
		if written.is_err() {
			let _ = std::fs::remove_file(&staged);
		}
		written.map(|()| id)
	}

	/// Path of artifact `id`, when it exists.
	pub fn path(&self, id: &str) -> Option<PathBuf> {
		let prefix = format!("{id}.");
		std::fs::read_dir(&self.dir)
			.ok()?
			.filter_map(Result::ok)
			.find(|entry| {
				entry
					.file_name()
					.to_str()
					.is_some_and(|name| name.starts_with(&prefix))
			})
			.map(|entry| entry.path())
	}
}

/// `spillLargeResultToArtifact`: bodies over 50 KB are saved whole and
/// replaced by a head + tail window. A failed save still truncates, without
/// a recovery link. Reads of artifacts are never re-spilled.
pub fn spill(
	mut output: ToolOutput,
	tool: &str,
	store: &ArtifactStore,
	reading_artifact: bool,
) -> ToolOutput {
	if reading_artifact || output.text.len() <= SPILL_THRESHOLD {
		return output;
	}
	let id = store.save(&output.text, tool).ok();
	let truncation = truncate_middle(&output.text);
	output.truncation = Some(middle_notice(&truncation, output.next_offset, id.as_deref()));
	output.text = truncation.content;
	output
}

#[cfg(test)]
mod tests {
	use super::*;

	fn lines(count: usize, width: usize) -> String {
		(1..=count)
			.map(|n| format!("{n:0>width$}"))
			.collect::<Vec<_>>()
			.join("\n")
	}

	#[test]
	fn byte_sizes_round_exact_ties_up_like_to_fixed() {
		assert_eq!(format_bytes(3328), "3.3KB");
		assert_eq!(format_bytes(3276), "3.2KB");
		assert_eq!(format_bytes(1023), "1023B");
		assert_eq!(format_bytes(51_200), "50.0KB");
	}

	#[test]
	fn small_outputs_render_notices_in_one_bracket() {
		let output = ToolOutput {
			text:        "body".into(),
			truncation:  Some("Showing lines 1-3 of 9".into()),
			next_offset: None,
			limits:      vec!["Some lines truncated to 768 chars".into()],
		};
		assert_eq!(
			output.render(),
			"body\n\n[Showing lines 1-3 of 9. Some lines truncated to 768 chars]"
		);
	}

	#[test]
	fn oversized_outputs_spill_with_a_head_tail_window_and_artifact_link() -> Result<()> {
		let dir = tempfile::tempdir()?;
		let store = ArtifactStore::for_session(&dir.path().join("s.jsonl")).unwrap();
		let text = lines(5000, 20);
		let spilled = spill(ToolOutput::text(text.clone()), "bash", &store, false);
		assert_eq!(std::fs::read_to_string(store.path("0").unwrap())?, text);
		// 20 KB windows of 21-byte rows: 975 head lines exceed the 500-line cap.
		assert!(spilled.text.starts_with(&lines(500, 20)), "head window");
		assert!(spilled.text.contains("\n[…4000ln elided…]\n"), "marker");
		assert_eq!(
			spilled.truncation.as_deref(),
			Some(
				"Showing lines 1-500 and 4501-5000 of 5000; 4,000 middle lines (82.0KB) elided. Read \
				 artifact://0 for full output"
			)
		);
		let second = spill(ToolOutput::text(text), "my tool/../x", &store, false);
		assert!(
			second
				.truncation
				.unwrap()
				.ends_with("artifact://1 for full output")
		);
		assert!(store.dir().join("1.my_tool_x.log").exists());
		Ok(())
	}

	#[test]
	fn single_giant_lines_use_byte_windows() {
		let dir = tempfile::tempdir().unwrap();
		let store = ArtifactStore::for_session(&dir.path().join("s.jsonl")).unwrap();
		let spilled = spill(ToolOutput::text("y".repeat(60 * 1024)), "read", &store, false);
		assert_eq!(spilled.text.len(), 40 * 1024 + "\n[…20480B elided…]\n".len());
		assert_eq!(
			spilled.truncation.as_deref(),
			Some(
				"Showing head and tail bytes of 1 line; 20.0KB elided. Read artifact://0 for full \
				 output"
			)
		);
	}

	#[test]
	fn artifact_reads_and_small_bodies_pass_through() {
		let dir = tempfile::tempdir().unwrap();
		let store = ArtifactStore::for_session(&dir.path().join("s.jsonl")).unwrap();
		let big = ToolOutput::text("z".repeat(60 * 1024));
		assert_eq!(spill(big.clone(), "read", &store, true), big);
		assert_eq!(spill(ToolOutput::text("ok"), "read", &store, false).render(), "ok");
	}
}
