//! Changelog normalization.
//!
//! Promotes items added to released sections back into `[Unreleased]`, drop
//! `[Unreleased]` duplicates of released bullets, compact list spacing,
//! merge/remove category headings, and collapse oversized files
//! behind an archive link.
use std::{
	collections::{BTreeMap, BTreeSet, HashSet},
	env, fs,
	path::{Path, PathBuf},
	process::{Command, Stdio},
	sync::LazyLock,
};

use anyhow::{Context, Result, bail};
use regex::Regex;

const ORDERED_SECTION_TITLES: [&str; 5] =
	["Breaking Changes", "Added", "Changed", "Fixed", "Removed"];
const BASELINE_REF: &str = "refs/clog";
const BASELINE_NAME: &str = "clog";
/// Per-file size ceiling; larger changelogs get their oldest releases
/// collapsed.
pub const MAX_CHANGELOG_BYTES: usize = 256 * 1024;
const GIT_OUTPUT_LIMIT: usize = 256 * 1024 * 1024;
const UNRELEASED: &str = "Unreleased";

static RELEASE_HEADING: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^## \[[^\]]+\]").expect("regex"));
static RELEASE_TITLE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^## \[([^\]]+)\]").expect("regex"));
static SUBSECTION_HEADING: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^###\s+\S").expect("regex"));
static HUNK: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").expect("regex"));
static ARCHIVE_LINK: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"^Older entries are archived in \[[^\]]+@[0-9a-f]{7,40}\]\(https://[^)]+\)\.$")
		.expect("regex")
});

// ── Document model ─────────────────────────────────────────────────────────

/// A source line with its one-based number; synthetic lines use 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
	pub text:   String,
	pub number: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subsection {
	pub title: String,
	pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
	pub heading:     String,
	pub title:       String,
	pub leading:     Vec<Line>,
	pub subsections: Vec<Subsection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
	pub prefix:   Vec<Line>,
	pub sections: Vec<Release>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
	pub start: usize,
	pub end:   usize,
	pub lines: Vec<String>,
}

fn is_release_heading(line: &str) -> bool {
	RELEASE_HEADING.is_match(line)
}

fn is_subsection_heading(line: &str) -> bool {
	SUBSECTION_HEADING.is_match(line)
}

fn is_list_item(line: &str) -> bool {
	line.trim_start().starts_with("- ")
}

fn is_blank(line: &str) -> bool {
	line.trim().is_empty()
}

fn release_title(heading: &str) -> String {
	RELEASE_TITLE.captures(heading).map_or_else(
		|| {
			let rest = heading.strip_prefix("##").map_or(heading, str::trim_start);
			rest.trim().to_owned()
		},
		|captures| captures[1].to_owned(),
	)
}

fn subsection_title(heading: &str) -> String {
	heading
		.strip_prefix("###")
		.map_or(heading, str::trim_start)
		.trim()
		.to_owned()
}

fn split_lines(content: &str) -> Vec<String> {
	let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
	let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
	body.split('\n').map(str::to_owned).collect()
}

fn trim_blank(lines: &[String]) -> Vec<String> {
	let start = lines
		.iter()
		.position(|line| !is_blank(line))
		.unwrap_or(lines.len());
	let end = lines
		.iter()
		.rposition(|line| !is_blank(line))
		.map_or(start, |index| index + 1);
	lines[start..end.max(start)].to_vec()
}

fn texts(lines: &[Line]) -> Vec<String> {
	lines.iter().map(|line| line.text.clone()).collect()
}

fn synthetic(lines: &[String]) -> Vec<Line> {
	lines
		.iter()
		.map(|text| Line { text: text.clone(), number: 0 })
		.collect()
}

fn trimmed(lines: &[Line]) -> Vec<String> {
	trim_blank(&texts(lines))
}

/// Parse a changelog into a prefix plus release sections.
pub fn parse(content: &str) -> Document {
	let lines: Vec<Line> = split_lines(content)
		.into_iter()
		.enumerate()
		.map(|(index, text)| Line { text, number: index + 1 })
		.collect();
	let first = lines
		.iter()
		.position(|line| is_release_heading(&line.text))
		.unwrap_or(lines.len());
	let prefix = lines[..first].to_vec();
	let mut sections = Vec::new();
	let mut index = first;
	while index < lines.len() {
		let heading = &lines[index];
		index += 1;
		let start = index;
		while index < lines.len() && !is_release_heading(&lines[index].text) {
			index += 1;
		}
		sections.push(parse_release(&heading.text, &lines[start..index]));
	}
	Document { prefix, sections }
}

fn parse_release(heading: &str, body: &[Line]) -> Release {
	let first = body
		.iter()
		.position(|line| is_subsection_heading(&line.text))
		.unwrap_or(body.len());
	let mut subsections = Vec::new();
	let mut index = first;
	while index < body.len() {
		let title = subsection_title(&body[index].text);
		index += 1;
		let start = index;
		while index < body.len() && !is_subsection_heading(&body[index].text) {
			index += 1;
		}
		subsections.push(Subsection { title, lines: body[start..index].to_vec() });
	}
	Release {
		heading: heading.to_owned(),
		title: release_title(heading),
		leading: body[..first].to_vec(),
		subsections,
	}
}

/// Split list items (a `- ` line plus its continuation lines).
pub fn parse_items(lines: &[Line]) -> Vec<Item> {
	let mut items = Vec::new();
	let mut index = 0;
	while index < lines.len() {
		if !is_list_item(&lines[index].text) {
			index += 1;
			continue;
		}
		let start = index;
		index += 1;
		while index < lines.len() && !is_list_item(&lines[index].text) {
			index += 1;
		}
		let block = &lines[start..index];
		items.push(Item {
			start: block[0].number,
			end:   block[block.len() - 1].number,
			lines: trimmed(block),
		});
	}
	items
}

fn line_range_set(items: &[Item]) -> HashSet<usize> {
	items
		.iter()
		.flat_map(|item| item.start..=item.end)
		.collect()
}

fn item_key(lines: &[String]) -> String {
	trim_blank(lines).join("\n")
}

fn append_lines(target: &mut Subsection, source: &[String]) {
	let source = trim_blank(source);
	if source.is_empty() {
		return;
	}
	let existing = trimmed(&target.lines);
	if existing.is_empty() {
		target.lines = synthetic(&source);
		return;
	}
	let separator = is_list_item(existing.last().expect("non-empty")) && is_list_item(&source[0]);
	let mut merged = existing;
	if !separator {
		merged.push(String::new());
	}
	merged.extend(source);
	target.lines = synthetic(&merged);
}

fn has_item(subsection: &Subsection, lines: &[String]) -> bool {
	let wanted = item_key(lines);
	wanted.is_empty()
		|| parse_items(&subsection.lines)
			.iter()
			.any(|item| item_key(&item.lines) == wanted)
}

fn released_keys(document: &Document) -> HashSet<String> {
	document
		.sections
		.iter()
		.filter(|section| section.title != UNRELEASED)
		.flat_map(|section| &section.subsections)
		.flat_map(|subsection| parse_items(&subsection.lines))
		.map(|item| item_key(&item.lines))
		.filter(|key| !key.is_empty())
		.collect()
}

/// Drop `[Unreleased]` items that already appear verbatim in a released
/// section (or in `historical` released bullets).
fn drop_released_duplicates(document: &mut Document, historical: &HashSet<String>) -> usize {
	let mut keys = released_keys(document);
	keys.extend(historical.iter().cloned());
	let Some(unreleased) = document
		.sections
		.iter_mut()
		.find(|section| section.title == UNRELEASED)
	else {
		return 0;
	};
	if keys.is_empty() {
		return 0;
	}
	let mut dropped = 0;
	for subsection in &mut unreleased.subsections {
		let duplicates: Vec<Item> = parse_items(&subsection.lines)
			.into_iter()
			.filter(|item| keys.contains(&item_key(&item.lines)))
			.collect();
		if duplicates.is_empty() {
			continue;
		}
		let remove = line_range_set(&duplicates);
		subsection
			.lines
			.retain(|line| !remove.contains(&line.number));
		dropped += duplicates.len();
	}
	dropped
}

fn title_order(title: &str) -> usize {
	ORDERED_SECTION_TITLES
		.iter()
		.position(|known| *known == title)
		.unwrap_or(ORDERED_SECTION_TITLES.len())
}

fn compact_list_spacing(lines: &[String]) -> Vec<String> {
	let lines = trim_blank(lines);
	if lines.is_empty() {
		return lines;
	}
	let items = parse_items(&synthetic(&lines));
	if items.is_empty() {
		return lines;
	}
	let flattened: Vec<String> = items.into_iter().flat_map(|item| item.lines).collect();
	let original: Vec<&String> = lines.iter().filter(|line| !is_blank(line)).collect();
	let compacted: Vec<&String> = flattened.iter().filter(|line| !is_blank(line)).collect();
	if original != compacted {
		return lines;
	}
	flattened
}

/// Returns (merged duplicate headings, removed empty headings).
fn normalize_section(section: &mut Release) -> (usize, usize) {
	let (mut merged, mut removed) = (0, 0);
	let mut normalized: Vec<Subsection> = Vec::new();
	for subsection in &section.subsections {
		let lines = compact_list_spacing(&trimmed(&subsection.lines));
		if lines.is_empty() {
			removed += 1;
			continue;
		}
		if let Some(existing) = normalized
			.iter_mut()
			.find(|existing| existing.title == subsection.title)
		{
			append_lines(existing, &lines);
			merged += 1;
			continue;
		}
		normalized.push(Subsection { title: subsection.title.clone(), lines: synthetic(&lines) });
	}
	if section.title == UNRELEASED {
		normalized.sort_by_key(|subsection| title_order(&subsection.title));
	}
	section.leading = synthetic(&trimmed(&section.leading));
	section.subsections = normalized;
	(merged, removed)
}

fn clone_release(section: &Release) -> Release {
	Release {
		heading:     section.heading.clone(),
		title:       section.title.clone(),
		leading:     synthetic(&trimmed(&section.leading)),
		subsections: section
			.subsections
			.iter()
			.map(|subsection| Subsection {
				title: subsection.title.clone(),
				lines: synthetic(&trimmed(&subsection.lines)),
			})
			.collect(),
	}
}

fn has_content(section: &Release) -> bool {
	!trimmed(&section.leading).is_empty()
		|| section
			.subsections
			.iter()
			.any(|subsection| !trimmed(&subsection.lines).is_empty())
}

/// JavaScript `Number.parseInt(part, 10)`, `NaN` as `None`.
fn parse_int(text: &str) -> Option<f64> {
	let text = text.trim_start();
	let (sign, digits) = match text.as_bytes().first() {
		Some(b'-') => (-1.0, &text[1..]),
		Some(b'+') => (1.0, &text[1..]),
		_ => (1.0, text),
	};
	let end = digits.bytes().take_while(u8::is_ascii_digit).count();
	(end > 0).then(|| sign * digits[..end].parse::<f64>().unwrap_or(f64::NAN))
}

/// Descending version order; a `NaN` segment compares as equal.
fn compare_versions_desc(left: &str, right: &str) -> std::cmp::Ordering {
	let parts = |title: &str| title.split('.').map(parse_int).collect::<Vec<_>>();
	let (left, right) = (parts(left), parts(right));
	for index in 0..left.len().max(right.len()) {
		let value = |parts: &[Option<f64>]| match parts.get(index) {
			None => Some(0.0),
			Some(part) => *part,
		};
		let (Some(left), Some(right)) = (value(&left), value(&right)) else {
			return std::cmp::Ordering::Equal;
		};
		let difference = right - left;
		if difference != 0.0 {
			return if difference < 0.0 {
				std::cmp::Ordering::Less
			} else {
				std::cmp::Ordering::Greater
			};
		}
	}
	std::cmp::Ordering::Equal
}

fn sort_releases(document: &mut Document) {
	let (unreleased, mut released): (Vec<Release>, Vec<Release>) = document
		.sections
		.drain(..)
		.partition(|section| section.title == UNRELEASED);
	released.sort_by(|left, right| compare_versions_desc(&left.title, &right.title));
	document.sections = unreleased.into_iter().chain(released).collect();
}

/// Replace released sections with their authoritative historical copies.
fn rebuild_from_history(content: &str, history: &[(String, Release)]) -> String {
	if history.is_empty() {
		return content.to_owned();
	}
	let mut document = parse(content);
	let mut unreleased = Vec::new();
	let mut released = Vec::new();
	let mut seen = HashSet::new();
	for section in document.sections.drain(..) {
		if section.title == UNRELEASED {
			unreleased.push(section);
			continue;
		}
		if !seen.insert(section.title.clone()) {
			continue;
		}
		match history.iter().find(|(title, _)| *title == section.title) {
			Some((_, historical)) => {
				let mut rebuilt = clone_release(historical);
				rebuilt.heading = section.heading;
				rebuilt.title = section.title;
				released.push(rebuilt);
			},
			None => released.push(section),
		}
	}
	for (title, section) in history {
		if !seen.contains(title) {
			released.push(clone_release(section));
		}
	}
	document.sections = unreleased.into_iter().chain(released).collect();
	sort_releases(&mut document);
	render(&document)
}

/// Render a document with canonical blank-line spacing.
pub fn render(document: &Document) -> String {
	let mut output: Vec<String> = Vec::new();
	let prefix = trimmed(&document.prefix);
	if !prefix.is_empty() {
		output.extend(prefix);
		output.push(String::new());
	}
	for section in &document.sections {
		output.push(section.heading.clone());
		let leading = trimmed(&section.leading);
		if !leading.is_empty() {
			output.push(String::new());
			output.extend(leading);
		}
		for subsection in &section.subsections {
			let lines = trimmed(&subsection.lines);
			if lines.is_empty() {
				continue;
			}
			output.push(String::new());
			output.push(format!("### {}", subsection.title));
			output.push(String::new());
			output.extend(lines);
		}
		output.push(String::new());
	}
	while output.last().is_some_and(String::is_empty) {
		output.pop();
	}
	format!("{}\n", output.join("\n"))
}

fn archive_repo() -> String {
	["HARVEST_REPO", "OMP_REPO", "GITHUB_REPOSITORY"]
		.iter()
		.find_map(|name| env::var(name).ok())
		.unwrap_or_else(|| "harvest/harvest".to_owned())
}

fn archive_link(path: &str, commit: &str) -> String {
	format!(
		"Older entries are archived in [{path}@{}](https://github.com/{}/blob/{commit}/{path}).",
		&commit[..commit.len().min(12)],
		archive_repo()
	)
}

/// Split a trailing archive-link footer from the body.
pub fn split_archive_footer(content: &str) -> (String, Option<String>) {
	let lines = split_lines(content);
	let end = lines
		.iter()
		.rposition(|line| !is_blank(line))
		.map_or(0, |index| index + 1);
	let Some(last) = end.checked_sub(1).map(|index| &lines[index]) else {
		return (content.to_owned(), None);
	};
	if !ARCHIVE_LINK.is_match(last) {
		return (content.to_owned(), None);
	}
	let body_end = lines[..end - 1]
		.iter()
		.rposition(|line| !is_blank(line))
		.map_or(0, |index| index + 1);
	(format!("{}\n", lines[..body_end].join("\n")), Some(last.clone()))
}

/// Collapse the oldest release sections until body plus footer fits
/// `max_bytes`; `[Unreleased]` and the newest release are always kept.
pub fn collapse_tail(content: &str, link: &str, max_bytes: usize) -> (String, usize) {
	let footer = link.len() + 2;
	if content.len() + footer <= max_bytes {
		return (content.to_owned(), 0);
	}
	let lines = split_lines(content);
	let headings: Vec<usize> = lines
		.iter()
		.enumerate()
		.filter(|(_, line)| is_release_heading(line))
		.map(|(index, _)| index)
		.collect();
	if headings.len() <= 2 {
		return (content.to_owned(), 0);
	}
	let mut cumulative = vec![0usize; lines.len() + 1];
	for (index, line) in lines.iter().enumerate() {
		cumulative[index + 1] = cumulative[index] + line.len() + 1;
	}
	let body_end = |heading: usize| {
		let mut end = heading;
		while end > 0 && is_blank(&lines[end - 1]) {
			end -= 1;
		}
		end
	};
	let budget = max_bytes as isize - footer as isize;
	let kept = (2..headings.len())
		.rev()
		.find(|&candidate| cumulative[body_end(headings[candidate])] as isize <= budget)
		.unwrap_or(2);
	let end = body_end(headings[kept]);
	(format!("{}\n", lines[..end].join("\n")), headings.len() - kept)
}

/// Counters plus the rewritten content for one changelog.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Fixed {
	pub content:                     String,
	pub promoted_items:              usize,
	pub merged_duplicate_headings:   usize,
	pub removed_empty_headings:      usize,
	pub dropped_released_duplicates: usize,
}

/// Promote added released-section items (by start line), drop released
/// duplicates, then normalize every section.
fn fix_content(content: &str, promotable: &HashSet<usize>, historical: &HashSet<String>) -> Fixed {
	let mut document = parse(content);
	let dropped = drop_released_duplicates(&mut document, historical);
	let mut promoted = 0;
	let mut moved: Vec<(String, Vec<String>)> = Vec::new();
	for section in &mut document.sections {
		if section.title == UNRELEASED {
			continue;
		}
		for subsection in &mut section.subsections {
			let items: Vec<Item> = parse_items(&subsection.lines)
				.into_iter()
				.filter(|item| promotable.contains(&item.start))
				.collect();
			if items.is_empty() {
				continue;
			}
			let remove = line_range_set(&items);
			subsection
				.lines
				.retain(|line| !remove.contains(&line.number));
			for item in items {
				moved.push((subsection.title.clone(), item.lines));
			}
		}
	}
	if !moved.is_empty() {
		if !document
			.sections
			.iter()
			.any(|section| section.title == UNRELEASED)
		{
			document.sections.insert(0, Release {
				heading:     "## [Unreleased]".to_owned(),
				title:       UNRELEASED.to_owned(),
				leading:     Vec::new(),
				subsections: Vec::new(),
			});
		}
		let unreleased = document
			.sections
			.iter_mut()
			.find(|section| section.title == UNRELEASED)
			.expect("created above");
		for (title, lines) in moved {
			let index = if let Some(index) = unreleased
				.subsections
				.iter()
				.rposition(|subsection| subsection.title == title)
			{
				index
			} else {
				unreleased
					.subsections
					.push(Subsection { title, lines: Vec::new() });
				unreleased.subsections.len() - 1
			};
			let target = &mut unreleased.subsections[index];
			if !has_item(target, &lines) {
				append_lines(target, &lines);
			}
			promoted += 1;
		}
	}
	let (mut merged, mut removed) = (0, 0);
	for section in &mut document.sections {
		let (section_merged, section_removed) = normalize_section(section);
		merged += section_merged;
		removed += section_removed;
	}
	sort_releases(&mut document);
	Fixed {
		content:                     render(&document),
		promoted_items:              promoted,
		merged_duplicate_headings:   merged,
		removed_empty_headings:      removed,
		dropped_released_duplicates: dropped,
	}
}

// ── Diff analysis ──────────────────────────────────────────────────────────

/// New-file line numbers of list items genuinely added since the baseline.
///
/// Excludes items paired with an identical removal (a move), not offset by an
/// unpaired removal in the same hunk (an edit), and not inside a hunk that adds
/// a release heading (a new release section).
fn promotable_lines(diff: &str) -> BTreeMap<String, BTreeSet<usize>> {
	struct Added {
		path:   String,
		line:   usize,
		text:   String,
		hunk:   (String, usize),
		paired: bool,
	}
	struct Removed {
		path:   String,
		text:   String,
		hunk:   (String, usize),
		paired: bool,
	}
	let mut added: Vec<Added> = Vec::new();
	let mut removed: Vec<Removed> = Vec::new();
	let mut heading_hunks: HashSet<(String, usize)> = HashSet::new();
	let (mut path, mut new_line, mut hunk) = (String::new(), 0usize, None::<usize>);
	for raw in diff.replace("\r\n", "\n").split('\n') {
		if let Some(rest) = raw.strip_prefix("+++ b/") {
			rest.clone_into(&mut path);
			continue;
		}
		if raw.starts_with("diff --git ") {
			path.clear();
			hunk = None;
			continue;
		}
		if let Some(captures) = HUNK.captures(raw) {
			new_line = captures[2].parse().unwrap_or(0);
			hunk = Some(hunk.map_or(0, |index| index + 1));
			continue;
		}
		let Some(index) = hunk else { continue };
		if path.is_empty() || raw.is_empty() {
			continue;
		}
		let key = (path.clone(), index);
		match raw.as_bytes()[0] {
			b'+' => {
				let text = &raw[1..];
				if raw.starts_with("+## [") {
					heading_hunks.insert(key.clone());
				}
				if is_list_item(text) {
					added.push(Added {
						path:   path.clone(),
						line:   new_line,
						text:   text.to_owned(),
						hunk:   key,
						paired: false,
					});
				}
				new_line += 1;
			},
			b'-' => {
				let text = &raw[1..];
				if is_list_item(text) {
					removed.push(Removed {
						path:   path.clone(),
						text:   text.to_owned(),
						hunk:   key,
						paired: false,
					});
				}
			},
			b' ' => new_line += 1,
			_ => {},
		}
	}
	for candidate in &mut added {
		if let Some(removal) = removed.iter_mut().find(|removal| {
			!removal.paired
				&& removal.path == candidate.path
				&& removal.text.trim() == candidate.text.trim()
		}) {
			removal.paired = true;
			candidate.paired = true;
		}
	}
	let mut unpaired: BTreeMap<(String, usize), usize> = BTreeMap::new();
	for removal in removed.iter().filter(|removal| !removal.paired) {
		*unpaired.entry(removal.hunk.clone()).or_default() += 1;
	}
	let mut lines: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
	for candidate in added {
		if candidate.paired || heading_hunks.contains(&candidate.hunk) {
			continue;
		}
		if let Some(count) = unpaired
			.get_mut(&candidate.hunk)
			.filter(|count| **count > 0)
		{
			*count -= 1;
			continue;
		}
		lines
			.entry(candidate.path)
			.or_default()
			.insert(candidate.line);
	}
	lines
}

// ── Git and filesystem ─────────────────────────────────────────────────────

fn git_command(root: &Path, args: &[&str]) -> Command {
	let mut command = Command::new("git");
	command
		.args([
			"-c",
			"core.fsmonitor=false",
			"-c",
			"core.untrackedCache=false",
			"-c",
			"fetch.pruneTags=false",
		])
		.args(args)
		.current_dir(root)
		.stdin(Stdio::null());
	command
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
	let output = crate::process::capture(git_command(root, args), GIT_OUTPUT_LIMIT)
		.with_context(|| format!("git {} failed", args.join(" ")))?;
	Ok(String::from_utf8_lossy(&output).into_owned())
}

fn git_maybe(root: &Path, args: &[&str]) -> Option<String> {
	git(root, args).ok()
}

fn resolve_root(root: Option<&Path>) -> Result<PathBuf> {
	match root {
		Some(root) => Ok(std::path::absolute(root)?),
		None => {
			Ok(PathBuf::from(git(&env::current_dir()?, &["rev-parse", "--show-toplevel"])?.trim()))
		},
	}
}

fn baseline_commit(root: &Path) -> Option<String> {
	git_maybe(root, &["rev-parse", "--verify", "--quiet", BASELINE_REF])
		.map(|text| text.trim().to_owned())
		.filter(|text| !text.is_empty())
}

/// Prefer the `clog` baseline over the latest version tag when it is newer.
fn resolve_since(root: &Path, since: Option<&str>) -> String {
	if let Some(since) = since {
		return since.to_owned();
	}
	let tag = git_maybe(root, &["describe", "--tags", "--abbrev=0", "--match", "v*"])
		.unwrap_or_default()
		.trim()
		.to_owned();
	let Some(baseline) = baseline_commit(root) else {
		return tag;
	};
	if tag.is_empty() {
		return BASELINE_REF.to_owned();
	}
	let tag_is_newer = git_maybe(root, &["merge-base", "--is-ancestor", &baseline, &tag]).is_some();
	if tag_is_newer {
		tag
	} else {
		BASELINE_REF.to_owned()
	}
}

fn recovery_tags(root: &Path) -> Result<Vec<String>> {
	let output = match baseline_commit(root) {
		Some(baseline) => git(root, &["tag", "--contains", &baseline, "--sort=v:refname"])?,
		None => git(root, &["tag", "--sort=v:refname"])?,
	};
	Ok(output
		.lines()
		.map(str::trim)
		.filter(|tag| !tag.is_empty())
		.map(str::to_owned)
		.collect())
}

struct Recovery {
	keys:     HashSet<String>,
	sections: Vec<(String, Release)>,
}

fn collect_recovery(root: &Path, paths: &[String]) -> Result<BTreeMap<String, Recovery>> {
	let mut recovery: BTreeMap<String, Recovery> = BTreeMap::new();
	for tag in recovery_tags(root)? {
		for path in paths {
			let Some(content) = git_maybe(root, &["show", &format!("{tag}:{path}")]) else {
				continue;
			};
			for section in parse(&content).sections {
				if section.title == UNRELEASED || !has_content(&section) {
					continue;
				}
				let entry = recovery
					.entry(path.clone())
					.or_insert_with(|| Recovery { keys: HashSet::new(), sections: Vec::new() });
				if !entry
					.sections
					.iter()
					.any(|(title, _)| *title == section.title)
				{
					entry
						.sections
						.push((section.title.clone(), clone_release(&section)));
				}
				for subsection in &section.subsections {
					for item in parse_items(&subsection.lines) {
						entry.keys.insert(item_key(&item.lines));
					}
				}
			}
		}
	}
	Ok(recovery)
}

/// `packages/*/CHANGELOG.md`, relative, sorted, skipping dot directories.
pub fn changelog_paths(root: &Path) -> Result<Vec<String>> {
	let mut paths = Vec::new();
	let Ok(entries) = fs::read_dir(root.join("packages")) else {
		return Ok(paths);
	};
	for entry in entries {
		let name = entry?.file_name().to_string_lossy().into_owned();
		if name.starts_with('.') {
			continue;
		}
		let relative = format!("packages/{name}/CHANGELOG.md");
		if root.join(&relative).is_file() {
			paths.push(relative);
		}
	}
	paths.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
	Ok(paths)
}

/// Per-file report for a changelog the fixer rewrote (or would rewrite).
#[derive(Debug, PartialEq, Eq)]
pub struct Changed {
	pub path: String,
	pub promoted_items: usize,
	pub merged_duplicate_headings: usize,
	pub dropped_released_duplicates: usize,
	pub removed_empty_headings: usize,
	pub collapsed_releases: usize,
}

pub struct Options<'a> {
	pub root:      Option<&'a Path>,
	pub since:     Option<&'a str>,
	pub write:     bool,
	pub recover:   bool,
	pub max_bytes: usize,
}

/// Run the fixer over every package changelog.
pub fn run(options: &Options<'_>) -> Result<(String, Vec<Changed>)> {
	let root = resolve_root(options.root)?;
	let since = resolve_since(&root, options.since);
	let paths = changelog_paths(&root)?;
	let added = if options.recover || paths.is_empty() {
		BTreeMap::new()
	} else {
		let mut args =
			vec!["diff", "--unified=0", "--no-color", "--no-ext-diff", since.as_str(), "--"];
		args.extend(paths.iter().map(String::as_str));
		promotable_lines(&git(&root, &args)?)
	};
	let recovery = if options.recover {
		collect_recovery(&root, &paths)?
	} else {
		BTreeMap::new()
	};
	let mut changed = Vec::new();
	for path in &paths {
		let absolute = root.join(path);
		let raw = fs::read_to_string(&absolute)?;
		let (body, existing_link) = split_archive_footer(&raw);
		let history = recovery.get(path);
		let recovered = history
			.map_or_else(|| body.clone(), |history| rebuild_from_history(&body, &history.sections));
		let empty_lines = HashSet::new();
		let empty_keys = HashSet::new();
		let promotable: HashSet<usize> = added
			.get(path)
			.map_or(empty_lines, |lines| lines.iter().copied().collect());
		let fixed =
			fix_content(&recovered, &promotable, history.map_or(&empty_keys, |history| &history.keys));

		let mut body = fixed.content.clone();
		let mut link = existing_link;
		let mut collapsed = 0;
		let projected = body.len() + link.as_ref().map_or(0, |link| link.len() + 2);
		if projected > options.max_bytes
			&& let Some(commit) = git_maybe(&root, &["rev-list", "-1", "HEAD", "--", path])
				.map(|text| text.trim().to_owned())
				.filter(|text| !text.is_empty())
		{
			let candidate = archive_link(path, &commit);
			let (content, releases) = collapse_tail(&body, &candidate, options.max_bytes);
			if releases > 0 {
				body = content;
				collapsed = releases;
				link = Some(candidate);
			}
		}
		let final_content = link.map_or_else(|| body.clone(), |link| format!("{body}\n{link}\n"));
		if final_content == raw {
			continue;
		}
		changed.push(Changed {
			path: path.clone(),
			promoted_items: fixed.promoted_items,
			merged_duplicate_headings: fixed.merged_duplicate_headings,
			dropped_released_duplicates: fixed.dropped_released_duplicates,
			removed_empty_headings: fixed.removed_empty_headings,
			collapsed_releases: collapsed,
		});
		if options.write {
			fs::write(&absolute, final_content)?;
		}
	}
	Ok((since, changed))
}

// ── CLI ────────────────────────────────────────────────────────────────────

fn usage() -> String {
	format!(
		"Usage: cargo xtask fix-changelogs [--dry-run|--check] [--since <tag>] [--recover] [--pin]

Moves changelog items added since the baseline from released sections into [Unreleased],
drops [Unreleased] items that already appear verbatim in a released section, removes
blank separators between adjacent bullet items, then removes duplicate or empty
### category headings.

Changelogs over {} KiB are collapsed: the oldest release sections are removed
and replaced with a footer linking the last commit that still contains them. Repeated
collapses chain — each footer's commit carries the previous footer.

The baseline defaults to the '{BASELINE_NAME}' ref (the last authoritative rewrite)
when it is newer than the latest version tag, otherwise the latest version tag — so a
--recover is not undone by a later plain run.

With --recover, the fixer scans every tagged changelog snapshot from the baseline forward
and treats every historically released bullet as authoritative, so stale [Unreleased]
items copied forward by past bad releases are pruned even if the current file no longer
contains a matching released copy. After committing a recovery, run --pin to mark it.

With --pin, move the '{BASELINE_NAME}' baseline ref to HEAD and exit without fixing.

Options:
  --dry-run          Print what would change without writing files.
  --check            Exit 1 if any changelog would change.
  --since <tag>      Compare changelog additions against this tag/commit instead of the baseline.
  --recover          Rebuild against historically released bullets from the baseline forward.
  --pin              Move the '{BASELINE_NAME}' baseline ref to HEAD, then exit.
  --repo-root <dir>  Run against an explicit repository root.",
		MAX_CHANGELOG_BYTES / 1024
	)
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Mode {
	Write,
	DryRun,
	Check,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let (mut mode, mut since, mut root, mut recover, mut pin) =
		(Mode::Write, None, None, false, false);
	let mut arguments = arguments.into_iter();
	while let Some(argument) = arguments.next() {
		match argument.as_str() {
			"--dry-run" => mode = Mode::DryRun,
			"--check" => mode = Mode::Check,
			"--recover" => recover = true,
			"--pin" => pin = true,
			"--since" => {
				since = Some(
					arguments
						.next()
						.context("--since requires a tag or commit")?,
				);
			},
			"--repo-root" => {
				root = Some(PathBuf::from(arguments.next().context("--repo-root requires a path")?));
			},
			"-h" | "--help" => {
				println!("{}", usage());
				return Ok(());
			},
			other => bail!("Unknown argument: {other}"),
		}
	}

	if pin {
		let root = resolve_root(root.as_deref())?;
		let paths = changelog_paths(&root)?;
		if !paths.is_empty() {
			let mut args = vec!["status", "--porcelain", "--"];
			args.extend(paths.iter().map(String::as_str));
			let dirty = git(&root, &args)?
				.lines()
				.filter(|line| !line.trim().is_empty())
				.count();
			if dirty > 0 {
				eprintln!(
					"Warning: {dirty} changelog file(s) have uncommitted changes; the pinned commit \
					 will not include them. Commit first, then re-run --pin."
				);
			}
		}
		let head = git(&root, &["rev-parse", "HEAD"])?.trim().to_owned();
		git(&root, &["update-ref", BASELINE_REF, &head])?;
		println!(
			"Pinned changelog baseline '{BASELINE_NAME}' ({BASELINE_REF}) to {}.",
			&head[..head.len().min(12)]
		);
		return Ok(());
	}

	let (since, changed) = run(&Options {
		root: root.as_deref(),
		since: since.as_deref(),
		write: mode == Mode::Write,
		recover,
		max_bytes: MAX_CHANGELOG_BYTES,
	})?;
	if changed.is_empty() {
		println!("Changelogs already clean since {since}.");
	} else {
		let suffix = match mode {
			Mode::Write => "",
			Mode::DryRun => " (dry-run, not written)",
			Mode::Check => " (check, not written)",
		};
		println!("Fixed {} changelog(s) since {since}{suffix}:", changed.len());
		for file in &changed {
			let mut parts = vec![
				format!("{} promoted item(s)", file.promoted_items),
				format!("{} merged duplicate heading(s)", file.merged_duplicate_headings),
				format!("{} dropped released duplicate(s)", file.dropped_released_duplicates),
				format!("{} removed empty heading(s)", file.removed_empty_headings),
			];
			if file.collapsed_releases > 0 {
				parts.push(format!("{} collapsed release(s)", file.collapsed_releases));
			}
			println!("  {}: {}", file.path, parts.join(", "));
		}
	}
	if recover && mode == Mode::Write && !changed.is_empty() {
		println!(
			"\nAuthoritative rewrite written. Commit the changelog changes, then run 'cargo xtask \
			 fix-changelogs --pin' to move the '{BASELINE_NAME}' baseline ref."
		);
	}
	if mode == Mode::Check && !changed.is_empty() {
		// The summary above is the report; exit 1 without an extra message.
		std::process::exit(1);
	}
	Ok(())
}

#[cfg(test)]
mod tests;
