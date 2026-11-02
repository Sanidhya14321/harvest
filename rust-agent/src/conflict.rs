//! Unresolved git merge conflicts surfaced by `read` and resolved by `write`
//! (`conflict-detect.ts`).
//!
//! `read` scans the lines it already collected for strict column-0
//! `<<<<<<<` / `|||||||` / `=======` / `>>>>>>>` blocks, registers each block
//! in the session's [`ConflictHistory`] under a stable id, and appends a
//! warning. `write conflict://N` splices the recorded region by content.
// The read-side half (scan, warning, summary, region render) is wired into
// `read` in the next slice; until then those helpers are only exercised by tests.
#![allow(dead_code, reason = "read-side conflict helpers are wired into read in the next slice")]

use std::{
	collections::BTreeMap,
	sync::{Arc, Mutex},
};

use anyhow::{Result, bail};

const OURS: &str = "<<<<<<<";
const BASE: &str = "|||||||";
const SEPARATOR: &str = "=======";
const THEIRS: &str = ">>>>>>>";
const SCAN_FILE_MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_ECHO_LINES: usize = 12;
const PREVIEW_SIDE_LINES: usize = 6;

/// One complete marker block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
	pub start_line:     usize,
	pub separator_line: usize,
	pub end_line:       usize,
	pub base_line:      Option<usize>,
	pub ours_label:     Option<String>,
	pub base_label:     Option<String>,
	pub theirs_label:   Option<String>,
	pub ours:           Vec<String>,
	pub base:           Option<Vec<String>>,
	pub theirs:         Vec<String>,
}

/// A registered block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
	pub id:            usize,
	pub absolute_path: std::path::PathBuf,
	pub display_path:  String,
	pub block:         Block,
}

fn strip_cr(line: &str) -> &str {
	line.strip_suffix('\r').unwrap_or(line)
}

/// The label after a strict column-0 marker: prefix alone, or prefix + space.
fn marker<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
	let rest = line.strip_prefix(prefix)?;
	if rest.is_empty() {
		return Some("");
	}
	rest.strip_prefix(' ')
}

fn label(text: &str) -> Option<String> {
	(!text.is_empty()).then(|| text.to_owned())
}

/// `scanConflictLines`: fully closed blocks in `lines`, numbered from `first`.
pub(crate) fn scan_lines<S: AsRef<str>>(lines: &[S], first: usize) -> Vec<Block> {
	#[derive(PartialEq)]
	enum Phase {
		Idle,
		Ours,
		Base,
		Theirs,
	}
	let mut blocks = Vec::new();
	let mut phase = Phase::Idle;
	let mut partial: Option<Block> = None;
	for (index, raw) in lines.iter().enumerate() {
		let line = strip_cr(raw.as_ref());
		let number = first + index;
		if let Some(text) = marker(line, OURS) {
			partial = Some(Block {
				start_line:     number,
				separator_line: 0,
				end_line:       0,
				base_line:      None,
				ours_label:     label(text),
				base_label:     None,
				theirs_label:   None,
				ours:           Vec::new(),
				base:           None,
				theirs:         Vec::new(),
			});
			phase = Phase::Ours;
			continue;
		}
		let Some(block) = partial.as_mut().filter(|_| phase != Phase::Idle) else {
			continue;
		};
		if let Some(text) = marker(line, BASE) {
			if phase == Phase::Ours {
				block.base_line = Some(number);
				block.base_label = label(text);
				block.base = Some(Vec::new());
				phase = Phase::Base;
			} else {
				partial = None;
				phase = Phase::Idle;
			}
			continue;
		}
		if line == SEPARATOR {
			if matches!(phase, Phase::Ours | Phase::Base) {
				block.separator_line = number;
				phase = Phase::Theirs;
			} else {
				partial = None;
				phase = Phase::Idle;
			}
			continue;
		}
		if let Some(text) = marker(line, THEIRS) {
			if phase == Phase::Theirs {
				let mut done = partial.take().expect("partial block");
				done.end_line = number;
				done.theirs_label = label(text);
				blocks.push(done);
			}
			partial = None;
			phase = Phase::Idle;
			continue;
		}
		match phase {
			Phase::Ours => block.ours.push(line.to_owned()),
			Phase::Base => block
				.base
				.get_or_insert_with(Vec::new)
				.push(line.to_owned()),
			Phase::Theirs => block.theirs.push(line.to_owned()),
			Phase::Idle => {},
		}
	}
	blocks
}

/// `scanFileForConflicts` over at most 10 MB.
pub(crate) fn scan_file(path: &std::path::Path) -> Result<(Vec<Block>, bool)> {
	let bytes = std::fs::read(path)?;
	let truncated = bytes.len() > SCAN_FILE_MAX_BYTES;
	let text = String::from_utf8_lossy(&bytes[..bytes.len().min(SCAN_FILE_MAX_BYTES)]).into_owned();
	let lines: Vec<&str> = text.split('\n').collect();
	Ok((scan_lines(&lines, 1), truncated))
}

#[derive(Default)]
struct HistoryState {
	next_id: usize,
	entries: BTreeMap<usize, Entry>,
}

/// Per-session, append-only log of conflict regions; ids stay stable.
#[derive(Clone, Default)]
pub struct ConflictHistory {
	inner: Arc<Mutex<HistoryState>>,
}

impl ConflictHistory {
	pub fn new() -> Self {
		Self::default()
	}

	fn state(&self) -> std::sync::MutexGuard<'_, HistoryState> {
		self
			.inner
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// Register a block, reusing the id of an earlier registration at the
	/// same path and start line (its recorded region is refreshed).
	pub(crate) fn register(
		&self,
		absolute_path: &std::path::Path,
		display_path: &str,
		block: Block,
	) -> Entry {
		let mut state = self.state();
		if let Some(existing) = state
			.entries
			.values()
			.find(|entry| {
				entry.absolute_path == absolute_path && entry.block.start_line == block.start_line
			})
			.map(|entry| entry.id)
		{
			let entry = Entry {
				id: existing,
				absolute_path: absolute_path.to_owned(),
				display_path: display_path.to_owned(),
				block,
			};
			state.entries.insert(existing, entry.clone());
			return entry;
		}
		state.next_id += 1;
		let id = state.next_id;
		let entry = Entry {
			id,
			absolute_path: absolute_path.to_owned(),
			display_path: display_path.to_owned(),
			block,
		};
		state.entries.insert(id, entry.clone());
		entry
	}

	pub(crate) fn get(&self, id: usize) -> Option<Entry> {
		self.state().entries.get(&id).cloned()
	}

	pub(crate) fn entries(&self) -> Vec<Entry> {
		self.state().entries.values().cloned().collect()
	}

	pub(crate) fn invalidate(&self, id: usize) {
		self.state().entries.remove(&id);
	}
}

/// A side `read conflict://N/<scope>` can render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
	Ours,
	Theirs,
	Base,
}

impl Scope {
	const fn name(self) -> &'static str {
		match self {
			Self::Ours => "ours",
			Self::Theirs => "theirs",
			Self::Base => "base",
		}
	}
}

/// A parsed `conflict://N[/scope]` or `conflict://*`; `None` id is `*`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Uri {
	pub id:               Option<usize>,
	pub scope:            Option<Scope>,
	pub recovered_prefix: Option<String>,
}

/// `parseConflictUri`: `Ok(None)` for non-conflict paths. A `<file>:` prefix
/// before the scheme is tolerated (the last `:conflict://` wins).
pub(crate) fn parse_uri(raw: &str) -> Result<Option<Uri>> {
	let Some(index) = raw.rfind("conflict://") else {
		return Ok(None);
	};
	let prefix = &raw[..index];
	let recovered_prefix = match prefix.strip_suffix(':') {
		Some(prefix) if !prefix.is_empty() => Some(prefix.to_owned()),
		_ if prefix.is_empty() => None,
		_ => return Ok(None),
	};
	let tail = &raw[index + "conflict://".len()..];
	if tail.is_empty() {
		return Ok(None);
	}
	let (id_part, scope_part) = tail
		.split_once('/')
		.map_or((tail, None), |(id, scope)| (id, Some(scope)));
	if id_part == "*" {
		if let Some(scope) = scope_part {
			bail!(
				"Invalid conflict URI '{raw}': wildcard 'conflict://*' does not accept a scope \
				 segment. Drop '/{scope}' or use a numeric id."
			);
		}
		return Ok(Some(Uri { id: None, scope: None, recovered_prefix }));
	}
	if id_part.is_empty() || !id_part.bytes().all(|byte| byte.is_ascii_digit()) {
		bail!(
			"Invalid conflict URI '{raw}': must be 'conflict://<N>', 'conflict://<N>/<scope>', or \
			 'conflict://*' where N is a positive integer surfaced by a prior `read`."
		);
	}
	let id: usize = id_part.parse().unwrap_or(usize::MAX);
	if id < 1 {
		bail!("Invalid conflict URI '{raw}': id must be ≥ 1.");
	}
	let scope = match scope_part {
		None => None,
		Some("ours") => Some(Scope::Ours),
		Some("theirs") => Some(Scope::Theirs),
		Some("base") => Some(Scope::Base),
		Some(_) => bail!(
			"Invalid conflict URI '{raw}': scope must be one of 'ours', 'theirs', 'base', or omitted \
			 (e.g. 'conflict://{id}/theirs')."
		),
	};
	Ok(Some(Uri { id: Some(id), scope, recovered_prefix }))
}

fn marker_line(prefix: &str, label: Option<&String>) -> String {
	label
		.filter(|label| !label.is_empty())
		.map_or_else(|| prefix.to_owned(), |label| format!("{prefix} {label}"))
}

/// The recorded marker block as it appears in the file.
fn recorded_region(block: &Block) -> Vec<String> {
	let mut out = vec![marker_line(OURS, block.ours_label.as_ref())];
	out.extend(block.ours.iter().cloned());
	if let Some(base) = &block.base {
		out.push(marker_line(BASE, block.base_label.as_ref()));
		out.extend(base.iter().cloned());
	}
	out.push(SEPARATOR.to_owned());
	out.extend(block.theirs.iter().cloned());
	out.push(marker_line(THEIRS, block.theirs_label.as_ref()));
	out
}

pub(crate) fn regions_equal(a: &Block, b: &Block) -> bool {
	recorded_region(a) == recorded_region(b)
}

pub(crate) fn region_present(content: &str, block: &Block) -> bool {
	content
		.replace("\r\n", "\n")
		.contains(&recorded_region(block).join("\n"))
}

fn matches_at(lines: &[&str], start: usize, expected: &[String]) -> bool {
	start + expected.len() <= lines.len()
		&& expected
			.iter()
			.enumerate()
			.all(|(offset, line)| strip_cr(lines[start + offset]) == line)
}

/// Nearest content match of `expected`, trying the recorded position first.
fn locate(lines: &[&str], expected: &[String], preferred: usize) -> Option<(usize, usize)> {
	if expected.is_empty() || expected.len() > lines.len() {
		return None;
	}
	if matches_at(lines, preferred, expected) {
		return Some((preferred, preferred + expected.len() - 1));
	}
	(0..=lines.len() - expected.len())
		.filter(|&start| matches_at(lines, start, expected))
		.min_by_key(|&start| start.abs_diff(preferred))
		.map(|start| (start, start + expected.len() - 1))
}

fn delimiter_balance<S: AsRef<str>>(lines: &[S]) -> i64 {
	lines
		.iter()
		.flat_map(|line| line.as_ref().chars())
		.map(|ch| match ch {
			'{' | '(' | '[' => 1,
			'}' | ')' | ']' => -1,
			_ => 0,
		})
		.sum()
}

/// Drop replacement lines that echo the context right outside the region:
/// multi-line echoes always, single-line echoes only when that restores the
/// delimiter balance both recorded sides agree on.
fn trim_boundary_echo(
	mut lines: Vec<String>,
	file: &[&str],
	(start, end): (usize, usize),
	block: &Block,
) -> (Vec<String>, usize, usize) {
	let ours = delimiter_balance(&block.ours);
	let expected = (ours == delimiter_balance(&block.theirs)).then_some(ours);
	let justified = |with: &[String], without: &[String]| {
		expected.is_some_and(|expected| {
			delimiter_balance(with) != expected && delimiter_balance(without) == expected
		})
	};
	let after: Vec<&str> = file[end + 1..]
		.iter()
		.take(MAX_ECHO_LINES)
		.map(|line| strip_cr(line))
		.collect();
	let mut trailing = 0;
	for k in (1..=after.len().min(lines.len().saturating_sub(1))).rev() {
		if !after[..k]
			.iter()
			.enumerate()
			.all(|(index, line)| lines[lines.len() - k + index] == *line)
		{
			continue;
		}
		if k >= 2 || justified(&lines, &lines[..lines.len() - 1]) {
			trailing = k;
			lines.truncate(lines.len() - k);
		}
		break;
	}
	let before: Vec<&str> = file[start.saturating_sub(MAX_ECHO_LINES)..start]
		.iter()
		.map(|line| strip_cr(line))
		.collect();
	let mut leading = 0;
	for k in (1..=before.len().min(lines.len().saturating_sub(1))).rev() {
		if !before[before.len() - k..]
			.iter()
			.enumerate()
			.all(|(index, line)| lines[index] == *line)
		{
			continue;
		}
		if k >= 2 || justified(&lines, &lines[1..]) {
			leading = k;
			lines.drain(..k);
		}
		break;
	}
	(lines, leading, trailing)
}

/// `spliceConflict`: the new file text and the echo lines trimmed.
pub(crate) fn splice(
	original: &str,
	entry: &Entry,
	replacement: &str,
) -> Result<(String, usize, usize)> {
	let lines: Vec<&str> = original.split('\n').collect();
	let expected = recorded_region(&entry.block);
	let Some(found) = locate(&lines, &expected, entry.block.start_line - 1) else {
		bail!(
			"Conflict #{} no longer present in '{}': the recorded marker block can't be located. The \
			 file changed since the conflict was registered — re-read it to re-register conflicts.",
			entry.id,
			entry.display_path
		);
	};
	let trimmed = replacement
		.strip_suffix("\r\n")
		.or_else(|| replacement.strip_suffix('\n'))
		.unwrap_or(replacement);
	let replacement_lines: Vec<String> = trimmed
		.split('\n')
		.map(|line| strip_cr(line).to_owned())
		.collect();
	let (mut replacement_lines, leading, trailing) =
		trim_boundary_echo(replacement_lines, &lines, found, &entry.block);
	if lines[found.0].ends_with('\r') {
		let following = found.1 + 1 < lines.len();
		let count = replacement_lines.len();
		for (index, line) in replacement_lines.iter_mut().enumerate() {
			if index + 1 < count || following {
				line.push('\r');
			}
		}
	}
	let mut next: Vec<String> = lines[..found.0]
		.iter()
		.map(|line| (*line).to_owned())
		.collect();
	next.extend(replacement_lines);
	next.extend(lines[found.1 + 1..].iter().map(|line| (*line).to_owned()));
	Ok((next.join("\n"), leading, trailing))
}

/// `expandContentTokens`: whole-line `@ours` / `@theirs` / `@base` / `@both`.
pub(crate) fn expand_tokens(content: &str, entry: &Entry) -> Result<String> {
	let mut out: Vec<String> = Vec::new();
	for raw in content.split('\n') {
		match strip_cr(raw) {
			"@ours" => out.extend(entry.block.ours.iter().cloned()),
			"@theirs" => out.extend(entry.block.theirs.iter().cloned()),
			"@base" => match &entry.block.base {
				Some(base) => out.extend(base.iter().cloned()),
				None => bail!(
					"Conflict #{} has no base section (2-way merge). `@base` is only valid for diff3 \
					 conflicts.",
					entry.id
				),
			},
			"@both" => {
				out.extend(entry.block.ours.iter().cloned());
				out.extend(entry.block.theirs.iter().cloned());
			},
			_ => out.push(raw.to_owned()),
		}
	}
	Ok(out.join("\n"))
}

/// `renderConflictRegion`: the lines to show and the file line of the first.
pub(crate) fn render_region(entry: &Entry, scope: Option<Scope>) -> Result<(Vec<String>, usize)> {
	let block = &entry.block;
	Ok(match scope {
		Some(Scope::Ours) => (block.ours.clone(), block.start_line + 1),
		Some(Scope::Theirs) => (block.theirs.clone(), block.separator_line + 1),
		Some(Scope::Base) => match (&block.base, block.base_line) {
			(Some(base), Some(line)) => (base.clone(), line + 1),
			_ => bail!(
				"Conflict #{} has no base section (2-way merge). 'conflict://{}/base' is only valid \
				 for diff3 conflicts.",
				entry.id,
				entry.id
			),
		},
		None => (recorded_region(block), block.start_line),
	})
}

fn pick_label(entries: &[Entry], get: impl Fn(&Entry) -> Option<&String>) -> Option<String> {
	entries
		.iter()
		.filter_map(&get)
		.find(|label| !label.trim().is_empty())
		.cloned()
}

fn append_body(out: &mut Vec<String>, section: &[String]) {
	if section.is_empty() {
		out.push("(empty)".to_owned());
		return;
	}
	out.extend(section.iter().take(PREVIEW_SIDE_LINES).cloned());
	let hidden = section.len().saturating_sub(PREVIEW_SIDE_LINES);
	if hidden > 0 {
		out.push(format!("… ({hidden} more line{})", if hidden == 1 { "" } else { "s" }));
	}
}

fn range(block: &Block) -> String {
	if block.start_line == block.end_line {
		format!("L{}", block.start_line)
	} else {
		format!("L{}-{}", block.start_line, block.end_line)
	}
}

fn label_lines(entries: &[Entry], out: &mut Vec<String>) {
	if let Some(ours) = pick_label(entries, |entry| entry.block.ours_label.as_ref()) {
		out.push(format!("- ours = {ours}"));
	}
	if let Some(theirs) = pick_label(entries, |entry| entry.block.theirs_label.as_ref()) {
		out.push(format!("- theirs = {theirs}"));
	}
	if entries.iter().any(|entry| entry.block.base.is_some()) {
		let base = pick_label(entries, |entry| {
			entry
				.block
				.base
				.as_ref()
				.and(entry.block.base_label.as_ref())
		});
		out.push(format!("- base = {}", base.as_deref().unwrap_or("(no label)")));
	}
}

/// `formatConflictWarning`, appended after the read window.
pub(crate) fn format_warning(
	entries: &[Entry],
	total: usize,
	display_path: &str,
	scan_truncated: bool,
) -> String {
	if entries.is_empty() {
		return String::new();
	}
	let word = if total == 1 { "conflict" } else { "conflicts" };
	let mut out = vec![String::new()];
	if total > entries.len() {
		out.push(format!(
			"⚠ {} of {total} unresolved {word} visible in this window (read \
			 `{display_path}:conflicts` for the full list).",
			entries.len()
		));
	} else {
		out.push(format!("⚠ {total} unresolved {word} detected"));
	}
	if scan_truncated {
		out.push(
			"- note: file scan hit the byte cap; additional conflicts may exist beyond the scanned \
			 prefix."
				.to_owned(),
		);
	}
	label_lines(entries, &mut out);
	out.extend(
		[
			"NOTICE: Inspect a block by reading `conflict://<N>` (add `/ours` / `/theirs` / `/base` \
			 to render a single side). Resolve with `write({ path: \"conflict://<N>\", content })`, \
			 or bulk-resolve every registered conflict with `write({ path: \"conflict://*\", content \
			 })`. Writes replace ONLY the marker block (markers + all sides) — never repeat the \
			 lines before/after it; they stay in place.",
			"`content` shorthand: a line that is exactly `@ours` / `@theirs` / `@base` / `@both` \
			 expands to that recorded section. `@both` is ours-then-theirs with no separator — only \
			 for additive conflicts where each side adds something different; NEVER for competing \
			 edits of the same lines (pick a side or write the combined text). Lines that are not a \
			 token pass through verbatim, so `\"// keep both\\n@ours\\n@theirs\"` literally writes \
			 the comment, then ours, then theirs.",
			"Per-id bulk: `write({ path: \"conflict://*\", content: \"1: @ours\\n2: @theirs\\n…\" \
			 })` resolves each listed id with that side in ONE call — the cheapest way through many \
			 pick-one conflicts; unlisted ids stay registered.",
			"Resolve each block faithfully: keep one side (`@ours`/`@theirs`), or combine them when \
			 both intents apply — never invent content beyond the recorded sides, and never stack \
			 both sides of competing edits. Resolve several conflicts in a single turn by issuing \
			 multiple `write` calls at once; ids stay valid as earlier blocks are resolved.",
		]
		.map(str::to_owned),
	);
	for entry in entries {
		let block = &entry.block;
		out.push(String::new());
		out.push(format!("──── #{}  {} ────", entry.id, range(block)));
		out.push("<<< ours".to_owned());
		append_body(&mut out, &block.ours);
		if let Some(base) = &block.base {
			if *base == block.ours {
				out.push("=== base ≡ ours".to_owned());
			} else if *base == block.theirs {
				out.push("=== base ≡ theirs".to_owned());
			} else {
				out.push("=== base".to_owned());
				append_body(&mut out, base);
			}
		}
		if block.theirs == block.ours {
			out.push(">>> theirs ≡ ours".to_owned());
		} else {
			out.push(">>> theirs".to_owned());
			append_body(&mut out, &block.theirs);
		}
	}
	out.join("\n")
}

/// `formatConflictSummary` for the `:conflicts` selector.
pub(crate) fn format_summary(
	entries: &[Entry],
	display_path: &str,
	scan_truncated: bool,
) -> String {
	let word = if entries.len() == 1 {
		"conflict"
	} else {
		"conflicts"
	};
	let display = if display_path.is_empty() {
		"<file>"
	} else {
		display_path
	};
	let mut out = vec![format!("⚠ {} unresolved {word} in {display}", entries.len())];
	if scan_truncated {
		out.push(
			"- note: file scan hit the byte cap; additional conflicts may exist beyond the scanned \
			 prefix."
				.to_owned(),
		);
	}
	label_lines(entries, &mut out);
	out.push(
		"NOTICE: Bulk-resolve with `write({ path: \"conflict://*\", content })`, or address a \
		 single block with `write({ path: \"conflict://<N>\", content })`. Inspect a block by \
		 reading `conflict://<N>` (add `/ours` / `/theirs` / `/base` for a single side)."
			.to_owned(),
	);
	out.push(
		"`content` shorthand: `@ours` / `@theirs` / `@base` / `@both` lines expand to the recorded \
		 sections; `@both` = ours-then-theirs (additive conflicts only — never for competing edits \
		 of the same lines). Per-id bulk: content of `<id>: @side` lines (e.g. \"1: @ours\\n2: \
		 @theirs\") resolves each listed id in one call. Non-token lines pass through verbatim. \
		 Writes replace ONLY the marker block — never repeat the surrounding lines. Keep one side \
		 or combine faithfully; never invent content beyond the recorded sides."
			.to_owned(),
	);
	out.push(String::new());
	let width = entries.last().map_or(1, |entry| entry.id.to_string().len());
	for entry in entries {
		let kind = if entry.block.base.is_some() {
			"  (3-way)"
		} else {
			""
		};
		out.push(format!("#{:>width$}  {}{kind}", entry.id, range(&entry.block)));
	}
	out.join("\n")
}

/// `parseBulkDirectives`: `<id>: @side` lines; `None` when no line is a
/// directive (uniform mode).
pub(crate) fn parse_directives(content: &str) -> Result<Option<BTreeMap<usize, String>>> {
	static DIRECTIVE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
		regex::Regex::new(r"^#?(\d+)\s*[:=]\s*(@ours|@theirs|@base|@both)$").expect("regex")
	});
	static HEAD: std::sync::LazyLock<regex::Regex> =
		std::sync::LazyLock::new(|| regex::Regex::new(r"^#?\d+\s*[:=]").expect("regex"));
	let mut map = BTreeMap::new();
	let mut stray = Vec::new();
	for raw in content.split('\n') {
		let line = raw.trim();
		if line.is_empty() {
			continue;
		}
		let Some(captures) = DIRECTIVE.captures(line) else {
			stray.push(line);
			continue;
		};
		let id: usize = captures[1].parse().unwrap_or(usize::MAX);
		if map.insert(id, captures[2].to_owned()).is_some() {
			bail!("Bulk directive lists conflict #{id} twice — each id may appear once.");
		}
	}
	if map.is_empty() {
		return Ok(None);
	}
	if let Some(sample) = stray.first() {
		let shown = if sample.chars().count() > 60 {
			format!("{}…", sample.chars().take(57).collect::<String>())
		} else {
			(*sample).to_owned()
		};
		let hint = if HEAD.is_match(sample) {
			"Per-id bulk only accepts the tokens @ours/@theirs/@base/@both — one side per id, single \
			 line. "
		} else {
			""
		};
		bail!(
			"Malformed `conflict://*` per-id block: {} line(s) are not `<id>: @side` directives \
			 (first: `{shown}`). {hint}Literal or multi-line replacement content isn't supported in \
			 a per-id block — resolve those blocks with individual `write({{ path: \
			 \"conflict://<N>\", content }})` calls (you can issue several at once). For a pure \
			 pick-a-side pass, make every non-empty line `<id>: @ours` (or @theirs/@base/@both).",
			stray.len()
		);
	}
	Ok(Some(map))
}

/// `resolveBulkDirectives`: prefer the pre-strip content.
pub(crate) fn resolve_directives(
	raw: &str,
	stripped: &str,
) -> Result<Option<BTreeMap<usize, String>>> {
	if raw == stripped {
		return parse_directives(raw);
	}
	match parse_directives(raw) {
		Ok(Some(map)) => Ok(Some(map)),
		Ok(None) => parse_directives(stripped),
		Err(error) => match parse_directives(stripped) {
			Ok(Some(map)) => Ok(Some(map)),
			_ => Err(error),
		},
	}
}

// ── Resolution through `write` ─────────────────────────────────────────────

static LOOSE_HEADER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
	regex::Regex::new(r"^\s*\[[^#\r\n]+#[^ \t\r\n]*\]\s*$").expect("regex")
});

/// `stripWriteContent` in hashline mode: drop `[path#TAG]` headers and `N:`
/// prefixes the model copied from `read` output.
pub(crate) fn strip_write_content(content: &str) -> (String, bool) {
	let lines: Vec<String> = content.split('\n').map(str::to_owned).collect();
	let original = lines.join("\n");
	let cleaned = pi_edit::modes::hashline::prefixes::strip_hashline_prefixes(&lines).join("\n");
	if cleaned != original {
		return (cleaned, true);
	}
	let Some(header) = lines.iter().position(|line| !line.trim().is_empty()) else {
		return (original, false);
	};
	if !LOOSE_HEADER.is_match(&lines[header]) {
		return (original, false);
	}
	let mut without = lines;
	without.remove(header);
	let text = without.join("\n");
	let cleaned = pi_edit::modes::hashline::prefixes::strip_hashline_prefixes(&without).join("\n");
	if cleaned == text {
		(original, false)
	} else {
		(cleaned, true)
	}
}

/// Shared state a conflict write touches.
pub(crate) struct Resolver<'a> {
	pub fs:      &'a harvest_runtime::JailedFs,
	pub history: &'a ConflictHistory,
	pub store:   &'a pi_edit::EditStore,
}

const STRIPPED_NOTE: &str =
	"Note: auto-stripped hashline display prefixes from content before writing.";

impl Resolver<'_> {
	fn read(&self, entry: &Entry) -> Result<String> {
		let bytes = self
			.fs
			.read(&entry.absolute_path, harvest_runtime::MAX_FILE_BYTES * 4)?;
		Ok(String::from_utf8_lossy(&bytes).into_owned())
	}

	fn write(&self, entry: &Entry, text: &str) -> Result<String> {
		self
			.fs
			.atomic_write(&entry.absolute_path, text.as_bytes())?;
		let key = pi_edit::path_policy::canonical_key(&entry.absolute_path);
		self.store.invalidate(&key);
		// `maybeWriteSnapshotHeader`: a fresh tag with empty seen-line provenance.
		let normalized = pi_edit::text::normalize_to_lf(text);
		let tag = self.store.record(&key, &normalized, Some(&[]));
		Ok(pi_edit::modes::hashline::format::format_hashline_header(&entry.display_path, &tag))
	}

	fn exists(&self, entry: &Entry) -> bool {
		self.fs.exists(&entry.absolute_path).unwrap_or(false)
	}

	/// `write conflict://N`.
	pub(crate) fn resolve_one(&self, id: usize, content: &str, stripped: bool) -> Result<String> {
		let Some(entry) = self.history.get(id) else {
			bail!(
				"Conflict #{id} not found. Conflict ids are registered when `read` surfaces a marker \
				 block; re-read the file to get a current id."
			);
		};
		if !self.exists(&entry) {
			bail!("Conflict #{} target '{}' no longer exists.", entry.id, entry.display_path);
		}
		let expanded = expand_tokens(content, &entry)?;
		let (text, leading, trailing) = splice(&self.read(&entry)?, &entry, &expanded)?;
		let header = self.write(&entry, &text)?;
		self.history.invalidate(entry.id);
		for other in self.history.entries() {
			if other.absolute_path == entry.absolute_path
				&& regions_equal(&other.block, &entry.block)
				&& !region_present(&text, &other.block)
			{
				self.history.invalidate(other.id);
			}
		}
		let block = &entry.block;
		let range = if block.start_line == block.end_line {
			format!("line {}", block.start_line)
		} else {
			format!("lines {}\u{2013}{}", block.start_line, block.end_line)
		};
		let mut result =
			format!("{header}\nResolved conflict #{} at {range} in {}.", entry.id, entry.display_path);
		if stripped {
			result.push('\n');
			result.push_str(STRIPPED_NOTE);
		}
		let echoed = leading + trailing;
		if echoed > 0 {
			let _ = std::fmt::Write::write_fmt(
				&mut result,
				format_args!(
					"\nNote: dropped {echoed} content line(s) that duplicated the code adjacent to the \
					 conflict region — writes replace only the marker block; surrounding lines stay in \
					 place."
				),
			);
		}
		Ok(result)
	}

	/// `write conflict://*`: uniform content, or `<id>: @side` directives.
	/// Files are spliced bottom-up and written all-or-nothing per file. Any
	/// failure surfaces as an error carrying the full report.
	pub(crate) fn resolve_all(&self, content: &str, stripped: bool, raw: &str) -> Result<String> {
		let all = self.history.entries();
		if all.is_empty() {
			bail!(
				"`conflict://*` has nothing to resolve — no conflicts are currently registered. \
				 Re-read the file(s) with conflicts first."
			);
		}
		let directives = resolve_directives(raw, content)?;
		if let Some(directives) = &directives {
			let unknown: Vec<String> = directives
				.keys()
				.filter(|id| !all.iter().any(|entry| entry.id == **id))
				.map(|id| format!("#{id}"))
				.collect();
			if !unknown.is_empty() {
				bail!(
					"Bulk directive references unknown conflict id(s) {}. Currently registered: {}.",
					unknown.join(", "),
					all.iter()
						.map(|entry| format!("#{}", entry.id))
						.collect::<Vec<_>>()
						.join(", ")
				);
			}
		}
		let selected: Vec<&Entry> = all
			.iter()
			.filter(|entry| {
				directives
					.as_ref()
					.is_none_or(|map| map.contains_key(&entry.id))
			})
			.collect();
		let mut by_file: Vec<(std::path::PathBuf, Vec<&Entry>)> = Vec::new();
		for entry in selected.iter().copied() {
			match by_file
				.iter_mut()
				.find(|(path, _)| *path == entry.absolute_path)
			{
				Some((_, bucket)) => bucket.push(entry),
				None => by_file.push((entry.absolute_path.clone(), vec![entry])),
			}
		}
		let word = |count: usize, one: &str, many: &str| {
			if count == 1 {
				one.to_owned()
			} else {
				many.to_owned()
			}
		};
		let mut succeeded: Vec<(String, usize, String)> = Vec::new();
		let mut failed: Vec<(String, usize, String)> = Vec::new();
		let (mut resolved_total, mut echoed_total) = (0, 0);
		for (_, mut entries) in by_file {
			let sample = entries[0];
			if !self.exists(sample) {
				failed.push((
					sample.display_path.clone(),
					entries.len(),
					"file no longer exists".to_owned(),
				));
				continue;
			}
			entries.sort_by_key(|entry| std::cmp::Reverse(entry.block.start_line));
			let mut text = match self.read(sample) {
				Ok(text) => text,
				Err(error) => {
					failed.push((sample.display_path.clone(), entries.len(), format!("{error:#}")));
					continue;
				},
			};
			let (mut resolved, mut stale): (Vec<&Entry>, Vec<&Entry>) = (Vec::new(), Vec::new());
			let mut failure = None;
			for entry in &entries {
				let side = directives
					.as_ref()
					.and_then(|map| map.get(&entry.id))
					.map_or(content, String::as_str);
				match expand_tokens(side, entry).and_then(|expanded| splice(&text, entry, &expanded)) {
					Ok((next, leading, trailing)) => {
						text = next;
						echoed_total += leading + trailing;
						resolved.push(entry);
					},
					Err(_)
						if resolved
							.iter()
							.any(|done| regions_equal(&done.block, &entry.block)) =>
					{
						stale.push(entry);
					},
					Err(error) => {
						failure = Some(format!("{error:#}"));
						break;
					},
				}
			}
			if let Some(error) = failure {
				failed.push((sample.display_path.clone(), entries.len(), error));
				continue;
			}
			let header = match self.write(sample, &text) {
				Ok(header) => header,
				Err(error) => {
					failed.push((sample.display_path.clone(), entries.len(), format!("{error:#}")));
					continue;
				},
			};
			for entry in resolved.iter().chain(&stale) {
				self.history.invalidate(entry.id);
			}
			resolved_total += resolved.len();
			succeeded.push((sample.display_path.clone(), resolved.len(), header));
		}
		let mut lines = Vec::new();
		if !succeeded.is_empty() {
			lines.push(format!(
				"Resolved {resolved_total} {} across {} {}:",
				word(resolved_total, "conflict", "conflicts"),
				succeeded.len(),
				word(succeeded.len(), "file", "files")
			));
			for (path, count, _) in &succeeded {
				lines.push(format!("  {path}: {count} {}", word(*count, "conflict", "conflicts")));
			}
		}
		if let Some(directives) = &directives
			&& selected.len() < all.len()
		{
			let remaining: Vec<String> = all
				.iter()
				.filter(|entry| !directives.contains_key(&entry.id))
				.map(|entry| format!("#{}", entry.id))
				.collect();
			lines.push(format!(
				"Directive mode: {} unlisted {} still registered ({}).",
				remaining.len(),
				word(remaining.len(), "conflict", "conflicts"),
				remaining.join(", ")
			));
		}
		if echoed_total > 0 {
			lines.push(format!(
				"Note: dropped {echoed_total} content line(s) that duplicated code adjacent to \
				 conflict regions — writes replace only the marker block; surrounding lines stay in \
				 place."
			));
		}
		if !failed.is_empty() {
			lines.push(format!(
				"Failed to resolve {} {} — registered entries left intact for retry:",
				failed.len(),
				word(failed.len(), "file", "files")
			));
			for (path, count, error) in &failed {
				lines.push(format!(
					"  {path}: {count} {} ({error})",
					word(*count, "conflict", "conflicts")
				));
			}
		}
		if !succeeded.is_empty() {
			lines.push("Snapshots:".to_owned());
			for (_, _, header) in &succeeded {
				lines.push(format!("  {header}"));
			}
		}
		if stripped && directives.is_none() {
			lines.push(STRIPPED_NOTE.to_owned());
		}
		let report = lines.join("\n");
		if failed.is_empty() {
			Ok(report)
		} else {
			bail!("{report}")
		}
	}
}

#[cfg(test)]
mod tests;
