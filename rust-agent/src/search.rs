//! The `grep` tool: the legacy tool's scope resolution, per-file caps,
//! pagination, grouped hashline output and notices over the shared `pi-grep`
//! engine.
use std::{
	collections::{HashMap, HashSet},
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, Ordering},
	time::Duration,
};

use anyhow::{Result, anyhow, bail};
use harvest_runtime::{
	JailedFs,
	cancel::{AbortReason, CancelToken},
};
use pi_edit::{
	EditStore, modes::hashline::format::format_hashline_header, path_policy::canonical_key,
};
use pi_grep::{GrepConfig, GrepMatch, GrepOutputMode};
use serde::Deserialize;

use crate::{
	output::ToolOutput,
	read::{Range, parse_ranges, split_path},
};

/// Files surfaced per response; further pages use `skip`.
const DEFAULT_FILE_LIMIT: usize = 20;
/// Per-file match cap when the scope spans several files.
const MULTI_FILE_PER_FILE_MATCHES: usize = 20;
/// Per-file match cap for a single-file scope.
const SINGLE_FILE_MATCHES: usize = 200;
/// Ceiling on engine matches before grouping.
const INTERNAL_TOTAL_CAP: usize = 2000;
/// The engine searches only this prefix of larger files.
const NATIVE_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const TIMEOUT_MS: u32 = 30_000;
/// `DEFAULT_MAX_COLUMN`.
const MAX_COLUMNS: u32 = 512;
/// `grep.contextBefore` / `grep.contextAfter` defaults.
const CONTEXT_BEFORE: u32 = 1;
const CONTEXT_AFTER: u32 = 3;
/// `truncateHead` byte budget.
const OUTPUT_MAX_BYTES: usize = 50 * 1024;
const GLOB_CHARS: [char; 4] = ['*', '?', '[', '{'];

#[derive(Debug, Deserialize)]
pub(crate) struct Args {
	pattern:            String,
	path:               Option<String>,
	#[serde(rename = "case")]
	case_sensitive:     Option<bool>,
	gitignore:          Option<bool>,
	skip:               Option<f64>,
	/// `grep.contextBefore` / `grep.contextAfter`, filled from settings.
	#[serde(skip)]
	pub(crate) context: Option<(u32, u32)>,
}

impl Args {
	fn context(&self) -> (u32, u32) {
		self.context.unwrap_or((CONTEXT_BEFORE, CONTEXT_AFTER))
	}
}

struct PathSpec {
	original: String,
	clean:    String,
	ranges:   Option<Vec<Range>>,
}

/// One engine invocation: a base path and an optional glob under it.
pub(crate) struct Target {
	pub(crate) base: PathBuf,
	pub(crate) glob: Option<String>,
}

pub(crate) struct Scope {
	/// Absolute search root (common ancestor for several entries).
	pub(crate) search_path:  PathBuf,
	pub(crate) is_directory: bool,
	pub(crate) glob:         Option<String>,
	pub(crate) targets:      Option<Vec<Target>>,
	pub(crate) exact_files:  Option<Vec<PathBuf>>,
	pub(crate) missing:      Vec<String>,
}

fn has_glob(text: &str) -> bool {
	text.contains(GLOB_CHARS)
}

pub(crate) fn is_url_like(text: &str) -> bool {
	let Some((scheme, _)) = text.split_once("://") else {
		return false;
	};
	let mut chars = scheme.chars();
	chars
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic())
		&& chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}

/// `normalizePathLikeInput`: trim, then strip one pair of outer double quotes.
fn normalize_input(input: &str) -> String {
	let trimmed = input.trim();
	if trimmed.len() > 1 && trimmed.starts_with('"') && trimmed.ends_with('"') {
		trimmed[1..trimmed.len() - 1].to_owned()
	} else {
		trimmed.to_owned()
	}
}

/// `toPathList`: a JSON-encoded string array is several entries.
pub(crate) fn to_path_list(input: Option<&str>) -> Vec<String> {
	let Some(input) = input else {
		return Vec::new();
	};
	let trimmed = input.trim();
	if trimmed.starts_with('[')
		&& trimmed.ends_with(']')
		&& let Ok(list) = serde_json::from_str::<Vec<String>>(trimmed)
	{
		return list;
	}
	vec![input.to_owned()]
}

/// `parseSearchPath`: split at the first segment holding glob characters.
fn parse_search_path(path: &str) -> (String, Option<String>) {
	let normalized = path.replace('\\', "/");
	let segments: Vec<&str> = normalized.split('/').collect();
	match segments.iter().position(|segment| has_glob(segment)) {
		None => (normalized, None),
		Some(0) => (".".to_owned(), Some(normalized)),
		Some(index) => (segments[..index].join("/"), Some(segments[index..].join("/"))),
	}
}

#[derive(Clone, Copy)]
enum SplitMode {
	Semicolon,
	Comma,
	Whitespace,
	Mixed,
}

const fn is_separator(ch: char, mode: SplitMode) -> bool {
	match mode {
		SplitMode::Semicolon => ch == ';',
		SplitMode::Comma => ch == ',',
		SplitMode::Whitespace => ch.is_whitespace(),
		SplitMode::Mixed => ch == ',' || ch == ';' || ch.is_whitespace(),
	}
}

/// Split on top-level separators (outside `{…}`, honoring `\` escapes).
fn split_top_level(entry: &str, mode: Option<SplitMode>) -> Vec<String> {
	let mut parts = Vec::new();
	let (mut depth, mut current) = (0usize, String::new());
	let mut chars = entry.chars();
	while let Some(ch) = chars.next() {
		match ch {
			'\\' => {
				current.push(ch);
				if let Some(next) = chars.next() {
					current.push(next);
				}
				continue;
			},
			'{' => depth += 1,
			'}' => depth = depth.saturating_sub(1),
			_ if depth == 0 => {
				let split = match mode {
					Some(mode) => is_separator(ch, mode),
					None => is_separator(ch, SplitMode::Mixed),
				};
				if split {
					parts.push(std::mem::take(&mut current));
					continue;
				}
			},
			_ => {},
		}
		current.push(ch);
	}
	parts.push(current);
	parts
}

pub(crate) struct Workspace {
	pub(crate) fs:   JailedFs,
	pub(crate) root: PathBuf,
}

impl Workspace {
	/// Jailed relative path of a user path (lexical; access is checked on use).
	fn relative(&self, path: &str) -> Result<PathBuf> {
		self.fs.relative(Path::new(path))
	}

	pub(crate) fn absolute(&self, path: &str) -> Result<PathBuf> {
		Ok(self.root.join(self.relative(path)?))
	}

	/// `lstat`-style existence: `Some(true)` exists, `Some(false)` missing,
	/// `None` unknown.
	fn probe(&self, path: &str) -> Option<bool> {
		let Ok(absolute) = self.absolute(path) else {
			return Some(false);
		};
		match std::fs::symlink_metadata(absolute) {
			Ok(_) => Some(true),
			Err(error) if is_missing(&error) => Some(false),
			Err(_) => None,
		}
	}

	/// Stat through the capability as `(is_file, is_dir)`; `Ok(None)` when the
	/// path does not exist.
	pub(crate) fn stat(&self, path: &str) -> Result<Option<(bool, bool)>> {
		let relative = self.relative(path)?;
		match self.fs.file_type(&relative) {
			Ok(kind) => Ok(Some((kind.is_file(), kind.is_dir()))),
			Err(_) if !self.fs.exists(&relative).unwrap_or(true) => Ok(None),
			Err(error) => Err(error),
		}
	}

	/// Canonical search root for the engine, refused if it leaves the jail.
	pub(crate) fn engine_path(&self, path: &Path) -> Result<PathBuf> {
		let canonical = std::fs::canonicalize(path)?;
		if !canonical.starts_with(&self.root) {
			bail!("path escapes the authorized root");
		}
		Ok(canonical)
	}

	pub(crate) fn display(&self, absolute: &Path) -> String {
		let relative = absolute
			.strip_prefix(&self.root)
			.map_or_else(|_| absolute.to_path_buf(), Path::to_path_buf);
		let text = relative.to_string_lossy().replace('\\', "/");
		if text.is_empty() {
			".".to_owned()
		} else {
			text
		}
	}
}

fn is_missing(error: &std::io::Error) -> bool {
	matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory)
		|| error.raw_os_error() == Some(63) // ENAMETOOLONG (macOS)
		|| error.raw_os_error() == Some(36) // ENAMETOOLONG (Linux)
}

/// `splitDelimitedPathEntry` + `expandDelimitedPathEntries`.
pub(crate) fn expand_entries(ws: &Workspace, entries: &[String]) -> Result<Vec<String>> {
	let mut expanded = Vec::new();
	for entry in entries {
		let normalized = normalize_input(entry);
		match split_entry(ws, &normalized)? {
			Some(parts) => expanded.extend(parts),
			None => expanded.push(normalized),
		}
	}
	Ok(expanded)
}

fn part_resolves(ws: &Workspace, part: &str) -> Result<bool> {
	if is_url_like(part) {
		return Ok(true);
	}
	let (peeled, _) = split_path(part);
	let (base, _) = parse_search_path(peeled);
	let Ok(absolute) = ws.absolute(&base) else {
		return Ok(false);
	};
	match std::fs::metadata(absolute) {
		Ok(_) => Ok(true),
		Err(error) if is_missing(&error) => Ok(false),
		Err(error) => Err(error.into()),
	}
}

fn split_entry(ws: &Workspace, entry: &str) -> Result<Option<Vec<String>>> {
	if split_top_level(entry, None).len() < 2 || is_url_like(entry) {
		return Ok(None);
	}
	if ws.probe(entry) != Some(false) {
		return Ok(None);
	}
	let (peeled, _) = split_path(entry);
	if !has_glob(peeled) && part_resolves(ws, entry)? {
		return Ok(None);
	}
	#[derive(Clone, Copy)]
	enum Need {
		All,
		Some,
		None,
	}
	for (mode, need) in [
		(SplitMode::Semicolon, Need::None),
		(SplitMode::Comma, Need::Some),
		(SplitMode::Whitespace, Need::All),
		(SplitMode::Mixed, Need::All),
	] {
		let raw = split_top_level(entry, Some(mode));
		if raw.len() < 2 {
			continue;
		}
		let parts: Vec<String> = raw
			.iter()
			.map(|part| normalize_input(part))
			.filter(|part| !part.is_empty())
			.collect();
		if parts.is_empty() || (parts.len() < 2 && raw.len() == parts.len()) {
			continue;
		}
		let accepted = match need {
			Need::None => true,
			Need::Some => {
				let mut any = false;
				for part in &parts {
					any |= part_resolves(ws, part)?;
				}
				any
			},
			Need::All => {
				let mut all = true;
				for part in &parts {
					all &= part_resolves(ws, part)?;
				}
				all
			},
		};
		if accepted {
			return Ok(Some(parts));
		}
	}
	Ok(None)
}

/// `parsePathSpecs` for filesystem entries.
fn parse_specs(ws: &Workspace, entries: &[String]) -> Result<Vec<PathSpec>> {
	let mut specs = Vec::new();
	for entry in entries {
		if is_url_like(entry) {
			bail!("Internal URLs are not supported by this host: {entry}");
		}
		let (path, selector) = split_path(entry);
		let literal = selector.is_some() && ws.probe(entry) != Some(false);
		let (clean, ranges) = match selector {
			Some(selector) if !literal => {
				let Some(ranges) = parse_ranges(&selector)? else {
					bail!(
						"path entry \"{entry}\" — only line-range selectors like \":50-100\" are \
						 supported (no \":raw\"/\":conflicts\")"
					);
				};
				if has_glob(path) {
					bail!("Line-range selector requires a single file, not a glob: {entry}");
				}
				(path.to_owned(), Some(ranges))
			},
			_ => (entry.clone(), None),
		};
		specs.push(PathSpec { original: entry.clone(), clean, ranges });
	}
	Ok(specs)
}

/// `parseSearchPathPreferringLiteral`.
fn parse_preferring_literal(ws: &Workspace, path: &str) -> (String, Option<String>) {
	if has_glob(path) && ws.absolute(path).is_ok_and(|absolute| absolute.exists()) {
		return (path.replace('\\', "/"), None);
	}
	parse_search_path(path)
}

fn common_base(paths: &[PathBuf]) -> PathBuf {
	let mut common: Vec<_> = paths[0].components().collect();
	for path in &paths[1..] {
		let shared = common
			.iter()
			.zip(path.components())
			.take_while(|(a, b)| **a == *b)
			.count();
		common.truncate(shared);
	}
	common.iter().collect()
}

pub(crate) fn posix_relative(base: &Path, path: &Path) -> String {
	let text = path
		.strip_prefix(base)
		.map_or_else(|_| path.to_string_lossy().into_owned(), |p| p.to_string_lossy().into_owned())
		.replace('\\', "/");
	if text.is_empty() {
		".".to_owned()
	} else {
		text
	}
}

fn join_glob(base: &str, glob: &str) -> String {
	let glob = glob.replace('\\', "/");
	let glob = glob.trim_start_matches('/');
	if base.is_empty() || base == "." {
		return glob.to_owned();
	}
	format!("{}/{glob}", base.trim_end_matches('/'))
}

fn brace_union(patterns: &[String]) -> Option<String> {
	let mut unique: Vec<String> = Vec::new();
	for pattern in patterns {
		let pattern = pattern.replace('\\', "/").trim().to_owned();
		if !pattern.is_empty() && !unique.contains(&pattern) {
			unique.push(pattern);
		}
	}
	match unique.len() {
		0 => None,
		1 => unique.pop(),
		_ => Some(format!("{{{}}}", unique.join(","))),
	}
}

/// `resolveToolSearchScope` for local paths (`fanOutFileTargets`,
/// `surfaceExactFilePaths`).
pub(crate) fn resolve_scope(ws: &Workspace, raw: &[String], fan_out: bool) -> Result<Scope> {
	let mut inputs = Vec::new();
	for entry in raw {
		let normalized = normalize_input(entry);
		if normalized.is_empty() {
			bail!("Search scope entries must be non-empty paths or globs");
		}
		inputs.push(normalized);
	}
	let mut missing = Vec::new();
	let effective = if inputs.len() > 1 {
		let mut valid = Vec::new();
		for item in &inputs {
			let (base, _) = parse_search_path(item);
			let exists = match ws.absolute(&base) {
				Ok(absolute) => match std::fs::metadata(absolute) {
					Ok(_) => true,
					Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
					Err(error) => return Err(error.into()),
				},
				Err(_) => false,
			};
			if exists {
				valid.push(item.clone());
			} else {
				missing.push(item.clone());
			}
		}
		if valid.is_empty() {
			bail!("Path not found: {}", missing.join(", "));
		}
		valid
	} else {
		inputs.clone()
	};
	let scope_display = |items: &[String]| -> String {
		items
			.iter()
			.map(|item| {
				let mut text = ws
					.absolute(item)
					.map_or_else(|_| item.clone(), |absolute| ws.display(&absolute));
				if (item.ends_with('/') || item.ends_with('\\')) && text != "." && !text.ends_with('/')
				{
					text.push('/');
				}
				text
			})
			.collect::<Vec<_>>()
			.join(", ")
	};
	let mut scope = if effective.len() == 1 {
		let (base, glob) = parse_preferring_literal(ws, &effective[0]);
		Scope {
			search_path: ws.absolute(&base)?,
			is_directory: false,
			glob,
			targets: None,
			exact_files: None,
			missing,
		}
	} else {
		let mut unique: Vec<String> = Vec::new();
		for item in effective {
			if !unique.contains(&item) {
				unique.push(item);
			}
		}
		struct Item {
			glob:     Option<String>,
			absolute: PathBuf,
			is_file:  bool,
			is_dir:   bool,
		}
		let mut items = Vec::new();
		for item in &unique {
			let (base, glob) = parse_preferring_literal(ws, item);
			let absolute = ws.absolute(&base)?;
			let Some((is_file, is_dir)) = ws.stat(&base)? else {
				bail!("Path not found: {}", scope_display(&unique));
			};
			items.push(Item { glob, absolute, is_file, is_dir });
		}
		let all_exact = items.iter().all(|item| item.glob.is_none() && item.is_file);
		let base = common_base(
			&items
				.iter()
				.map(|item| item.absolute.clone())
				.collect::<Vec<_>>(),
		);
		let patterns: Vec<String> = items
			.iter()
			.map(|item| {
				let relative = posix_relative(&base, &item.absolute);
				if let Some(glob) = &item.glob {
					join_glob(&relative, glob)
				} else if item.is_dir {
					join_glob(&relative, "**/*")
				} else if relative == "." {
					item
						.absolute
						.file_name()
						.map_or_else(String::new, |name| name.to_string_lossy().into_owned())
				} else {
					relative
				}
			})
			.collect();
		let common_requested = items.iter().any(|item| item.absolute == base);
		let demotes_file =
			fan_out && !all_exact && items.iter().any(|item| item.glob.is_none() && item.is_file);
		let targets = (items.len() > 1 && (!common_requested || demotes_file)).then(|| {
			items
				.iter()
				.map(|item| Target { base: item.absolute.clone(), glob: item.glob.clone() })
				.collect::<Vec<_>>()
		});
		let exact_files = (fan_out && all_exact).then(|| {
			items
				.iter()
				.map(|item| item.absolute.clone())
				.collect::<Vec<_>>()
		});
		let glob = if exact_files.is_some() || targets.is_some() {
			None
		} else {
			brace_union(&patterns)
		};
		Scope { search_path: base, is_directory: false, glob, targets, exact_files, missing }
	};
	let display_scope = if inputs.len() == 1 {
		ws.display(&scope.search_path)
	} else {
		scope_display(&inputs)
	};
	let Ok(metadata) = std::fs::metadata(&scope.search_path) else {
		let hint = if raw.len() > 1 {
			" (`path` list entries must each exist relative to cwd)"
		} else {
			""
		};
		bail!("Path not found: {display_scope}{hint}");
	};
	scope.is_directory = metadata.is_dir();
	Ok(scope)
}

/// Marks in-memory archive matches; the rest is the display path.
const VIRTUAL_PREFIX: &str = "\u{0}archive:";

/// An `archive.ext:member` entry's UTF-8 text, or why it cannot be searched;
/// `None` when the entry is not an archive member.
fn archive_member(ws: &Workspace, entry: &str) -> Option<std::result::Result<String, String>> {
	let (archive_path, member) = harvest_archive::path_candidates(entry)
		.into_iter()
		.find(|(archive, sub)| !sub.is_empty() && archive != entry)?;
	let read = || -> std::result::Result<String, String> {
		let relative = ws
			.relative(&archive_path)
			.map_err(|e| format!("cannot open archive: {e}"))?;
		let bytes = ws
			.fs
			.read(&relative, 256 * 1024 * 1024)
			.map_err(|e| format!("cannot open archive: {e}"))?;
		let archive = harvest_archive::Archive::open(Path::new(&archive_path), bytes)
			.map_err(|e| format!("cannot open archive: {e}"))?;
		let bytes = archive.read(&member).map_err(|e| e.to_string())?;
		if bytes.contains(&0) {
			return Err("binary archive entry".to_owned());
		}
		String::from_utf8(bytes).map_err(|_| "non-UTF-8 archive entry".to_owned())
	};
	Some(read())
}

/// Keep a match (and its context) only inside `ranges`.
fn filter_ranges(mut m: GrepMatch, ranges: &[Range]) -> Option<GrepMatch> {
	if !line_in_ranges(m.line_number, ranges) {
		return None;
	}
	let keep = |lines: Option<Vec<pi_grep::ContextLine>>| {
		lines
			.map(|lines| {
				lines
					.into_iter()
					.filter(|line| line_in_ranges(line.line_number, ranges))
					.collect::<Vec<_>>()
			})
			.filter(|lines| !lines.is_empty())
	};
	m.context_before = keep(m.context_before.take());
	m.context_after = keep(m.context_after.take());
	Some(m)
}

fn line_in_ranges(line: u32, ranges: &[Range]) -> bool {
	let line = line as usize;
	ranges
		.iter()
		.any(|range| line >= range.start && range.end.is_none_or(|end| line <= end))
}

/// `lineRangeFetchCap`: enough per-file entries for the range filter to still
/// keep `keep` in-range hits, clamped to the engine's file-size ceiling.
fn range_fetch_cap(specs: &[PathSpec], keep: usize) -> usize {
	let cap = specs
		.iter()
		.filter_map(|spec| spec.ranges.as_ref())
		.flatten()
		.map(|range| range.end.unwrap_or(range.start - 1 + keep))
		.max()
		.unwrap_or(0);
	cap.min(NATIVE_MAX_FILE_BYTES as usize)
}

struct EngineRun {
	matches:           Vec<GrepMatch>,
	limit_reached:     bool,
	skipped_oversized: u32,
}

fn run_engine(
	pattern: &str,
	target: &Path,
	glob: Option<&str>,
	ignore_case: bool,
	gitignore: bool,
	max_count: usize,
	per_file: usize,
	context: (u32, u32),
	cancellation: &CancelToken,
) -> Result<pi_grep::GrepResult> {
	let mut token = CancelToken::new(Some(TIMEOUT_MS));
	let abort = token.emplace_abort_token();
	let done = AtomicBool::new(false);
	let multiline = pattern.contains('\n') || pattern.contains("\\n");
	let config = GrepConfig {
		pattern:            pattern.to_owned(),
		path:               target.to_string_lossy().into_owned(),
		glob:               glob.map(str::to_owned),
		type_filter:        None,
		ignore_case:        Some(ignore_case),
		multiline:          Some(multiline),
		hidden:             Some(true),
		gitignore:          Some(gitignore),
		max_count:          Some(max_count as u32),
		offset:             None,
		context_before:     Some(context.0),
		context_after:      Some(context.1),
		context:            None,
		max_columns:        Some(MAX_COLUMNS),
		mode:               Some(GrepOutputMode::Content),
		max_count_per_file: Some(per_file as u32),
	};
	let result = std::thread::scope(|scope| {
		scope.spawn(|| {
			while !done.load(Ordering::Relaxed) {
				if cancellation.heartbeat().is_err() {
					abort.abort(AbortReason::User);
					return;
				}
				std::thread::sleep(Duration::from_millis(20));
			}
		});
		let result = pi_grep::grep_sync(config, token);
		done.store(true, Ordering::Relaxed);
		result
	});
	result.map_err(|error| {
		let message = error.to_string();
		let lower = message.to_ascii_lowercase();
		if lower.starts_with("regex error") || lower.starts_with("regex parse error") {
			let rest = message
				.split_once("error")
				.map_or("", |(_, rest)| rest)
				.trim_start_matches(':')
				.trim_start();
			anyhow!("Invalid regex: {rest}")
		} else if message.contains("Aborted: Timeout") {
			anyhow!(
				"Grep timed out after {}s; narrow paths or pattern, or scope with `glob` first",
				TIMEOUT_MS / 1000
			)
		} else {
			anyhow!("{message}")
		}
	})
}

/// Run the engine over the resolved scope; match paths come back relative to
/// `scope.search_path` (directory scopes) or absolute (file scopes).
fn search_scope(
	ws: &Workspace,
	scope: &Scope,
	args: &Args,
	max_count: usize,
	per_file: usize,
	cancellation: &CancelToken,
) -> Result<EngineRun> {
	let ignore_case = !args.case_sensitive.unwrap_or(true);
	let gitignore = args.gitignore.unwrap_or(true);
	let targets: Option<Vec<Target>> = match (&scope.exact_files, &scope.targets) {
		(Some(files), _) => Some(
			files
				.iter()
				.map(|file| Target { base: file.clone(), glob: None })
				.collect(),
		),
		(None, Some(targets)) => Some(
			targets
				.iter()
				.map(|target| Target { base: target.base.clone(), glob: target.glob.clone() })
				.collect(),
		),
		(None, None) => None,
	};
	let Some(targets) = targets else {
		let engine_root = ws.engine_path(&scope.search_path)?;
		let result = run_engine(
			&args.pattern,
			&engine_root,
			scope.glob.as_deref(),
			ignore_case,
			gitignore,
			max_count,
			per_file,
			args.context(),
			cancellation,
		)?;
		let matches = result
			.matches
			.into_iter()
			.map(|mut m| {
				if scope.is_directory {
					return m;
				}
				m.path = scope.search_path.to_string_lossy().into_owned();
				m
			})
			.collect();
		return Ok(EngineRun {
			matches,
			limit_reached: result.limit_reached.unwrap_or(false),
			skipped_oversized: result.skipped_oversized.unwrap_or(0),
		});
	};
	let mut run =
		EngineRun { matches: Vec::new(), limit_reached: false, skipped_oversized: 0 };
	let mut seen = HashSet::new();
	for target in targets {
		let engine_root = ws.engine_path(&target.base)?;
		let result = run_engine(
			&args.pattern,
			&engine_root,
			target.glob.as_deref(),
			ignore_case,
			gitignore,
			max_count,
			per_file,
			args.context(),
			cancellation,
		)?;
		run.skipped_oversized += result.skipped_oversized.unwrap_or(0);
		run.limit_reached |= result.limit_reached.unwrap_or(false);
		let target_is_dir = target.base.is_dir();
		for mut m in result.matches {
			let absolute = if target_is_dir {
				target.base.join(&m.path)
			} else {
				target.base.clone()
			};
			if !seen.insert((absolute.clone(), m.line_number)) {
				continue;
			}
			m.path = posix_relative(&scope.search_path, &absolute);
			run.matches.push(m);
		}
	}
	Ok(run)
}

/// `formatGroupedFiles`: a prefix-folded tree with one `#` per level and a
/// blank line before every directory header and root-level file.
pub(crate) fn format_grouped_files(
	files: &[String],
	render: &mut dyn FnMut(&str) -> (Vec<String>, String),
) -> Vec<String> {
	#[derive(Default)]
	struct Node {
		files:   Vec<(String, String)>,
		subdirs: Vec<(String, Self)>,
	}
	fn child<'a>(node: &'a mut Node, name: &str) -> &'a mut Node {
		let index = node
			.subdirs
			.iter()
			.position(|(existing, _)| existing == name)
			.unwrap_or_else(|| {
				node.subdirs.push((name.to_owned(), Node::default()));
				node.subdirs.len() - 1
			});
		&mut node.subdirs[index].1
	}
	let mut sections = HashMap::new();
	let mut root = Node::default();
	for file in files {
		if sections.contains_key(file) {
			continue;
		}
		let (body, suffix) = render(file);
		if body.is_empty() {
			continue;
		}
		sections.insert(file.clone(), (body, suffix));
		let normalized = file.replace('\\', "/");
		if is_url_like(&normalized) {
			root.files.push((normalized, file.clone()));
			continue;
		}
		let segments: Vec<&str> = normalized.split('/').collect();
		let mut node = &mut root;
		for segment in &segments[..segments.len() - 1] {
			node = child(node, segment);
		}
		let name = segments[segments.len() - 1].to_owned();
		if !node.files.iter().any(|(existing, _)| *existing == name) {
			node.files.push((name, file.clone()));
		}
	}
	fn walk(
		node: &Node,
		depth: usize,
		sections: &HashMap<String, (Vec<String>, String)>,
		out: &mut Vec<String>,
		emitted: &mut bool,
	) {
		let hashes = "#".repeat(depth + 1);
		for (name, key) in &node.files {
			if *emitted && depth == 0 {
				out.push(String::new());
			}
			*emitted = true;
			let (body, suffix) = &sections[key];
			out.push(format!("{hashes} {name}{suffix}"));
			out.extend(body.iter().cloned());
		}
		for (name, mut dir) in node.subdirs.iter().map(|(name, dir)| (name, dir)) {
			let mut parts = vec![name.as_str()];
			while dir.files.is_empty() && dir.subdirs.len() == 1 {
				parts.push(&dir.subdirs[0].0);
				dir = &dir.subdirs[0].1;
			}
			if *emitted {
				out.push(String::new());
			}
			*emitted = true;
			out.push(format!("{hashes} {}/", parts.join("/")));
			walk(dir, depth + 1, sections, out, emitted);
		}
	}
	let mut out = Vec::new();
	walk(&root, 0, &sections, &mut out, &mut false);
	out
}

pub(crate) fn grep(
	root: &Path,
	args: &Args,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<ToolOutput> {
	cancellation.heartbeat()?;
	if args.pattern.trim().is_empty() {
		bail!("Pattern must not be empty");
	}
	let skip = match args.skip {
		None => 0,
		Some(skip) if skip.is_finite() && skip.floor() >= 0.0 => skip.floor() as usize,
		Some(_) => bail!("Skip must be a non-negative number"),
	};
	let fs = JailedFs::open(root)?;
	let ws = Workspace { root: fs.root().to_path_buf(), fs };
	let scoped = to_path_list(args.path.as_deref());
	let effective = if scoped.is_empty() {
		vec![".".to_owned()]
	} else {
		scoped
	};
	let entries = expand_entries(&ws, &effective)?;
	let specs = parse_specs(&ws, &entries)?;
	// `resolveArchiveSearchPaths`: text members are searched in memory.
	let mut members: Vec<(usize, String)> = Vec::new();
	let mut unreadable: Vec<String> = Vec::new();
	let mut fs_specs: Vec<&PathSpec> = Vec::new();
	for (index, spec) in specs.iter().enumerate() {
		match archive_member(&ws, &spec.clean) {
			None => fs_specs.push(spec),
			Some(Ok(text)) => members.push((index, text)),
			Some(Err(reason)) => unreadable.push(format!("{} ({reason})", spec.clean)),
		}
	}
	if !unreadable.is_empty() && fs_specs.is_empty() && members.is_empty() {
		bail!(
			"Cannot search archive member(s): {}. Read the member with `read <archive>:<member>` and \
			 inspect the returned text, or pass a UTF-8 text member.",
			unreadable.join(", ")
		);
	}
	let scope = if fs_specs.is_empty() {
		Scope {
			search_path:  ws.root.clone(),
			is_directory: false,
			glob:         None,
			targets:      Some(Vec::new()),
			exact_files:  None,
			missing:      Vec::new(),
		}
	} else {
		let searchable: Vec<String> = fs_specs.iter().map(|spec| spec.clean.clone()).collect();
		resolve_scope(&ws, &searchable, true)?
	};

	let mut ranges_by_path: HashMap<PathBuf, Vec<Range>> = HashMap::new();
	for spec in &fs_specs {
		let Some(ranges) = &spec.ranges else { continue };
		match ws.stat(&spec.clean)? {
			None => bail!("Path not found for line-range selector: {}", spec.original),
			Some((is_file, _)) if !is_file => {
				bail!("Line-range selector requires a single file: {} is a directory", spec.original)
			},
			Some(_) => {},
		}
		ranges_by_path
			.entry(ws.absolute(&spec.clean)?)
			.or_default()
			.extend(ranges.iter().copied());
	}

	let multi_scope = if fs_specs.is_empty() {
		members.len() > 1
	} else {
		scope.is_directory
			|| scope.exact_files.is_some()
			|| scope.targets.is_some()
			|| !members.is_empty()
	};
	let per_file_cap = if multi_scope {
		MULTI_FILE_PER_FILE_MATCHES
	} else {
		SINGLE_FILE_MATCHES
	};
	let has_ranges = specs.iter().any(|spec| spec.ranges.is_some());
	let native_per_file = if has_ranges {
		(per_file_cap + 1).max(range_fetch_cap(&specs, per_file_cap + 1))
	} else {
		per_file_cap + 1
	};
	let native_max = if has_ranges {
		INTERNAL_TOTAL_CAP.div_ceil(per_file_cap + 1) * native_per_file
	} else {
		INTERNAL_TOTAL_CAP
	};
	let run = search_scope(&ws, &scope, args, native_max, native_per_file, cancellation)?;
	cancellation.heartbeat()?;

	let absolute_of = |match_path: &str| -> PathBuf {
		let path = Path::new(match_path);
		if path.is_absolute() {
			path.to_path_buf()
		} else {
			scope.search_path.join(path)
		}
	};
	let mut matches = run.matches;
	if !ranges_by_path.is_empty() {
		matches = matches
			.into_iter()
			.filter_map(|m| match ranges_by_path.get(&absolute_of(&m.path)) {
				None => Some(m),
				Some(ranges) => filter_ranges(m, ranges),
			})
			.collect();
	}

	for (index, text) in &members {
		let spec = &specs[*index];
		let result = pi_grep::search_sync(text.as_bytes(), pi_grep::SearchOptions {
			pattern:        args.pattern.clone(),
			ignore_case:    Some(!args.case_sensitive.unwrap_or(true)),
			multiline:      Some(args.pattern.contains('\n') || args.pattern.contains("\\n")),
			max_count:      Some(native_per_file as u32),
			offset:         None,
			context_before: Some(args.context().0),
			context_after:  Some(args.context().1),
			context:        None,
			max_columns:    Some(MAX_COLUMNS),
			mode:           Some(GrepOutputMode::Content),
		});
		if let Some(error) = result.error {
			let lower = error.to_ascii_lowercase();
			if lower.starts_with("regex error") || lower.starts_with("regex parse error") {
				let rest = error.split_once("error").map_or("", |(_, rest)| rest);
				bail!("Invalid regex: {}", rest.trim_start_matches(':').trim_start());
			}
			bail!("{error}");
		}
		for found in result.matches {
			let m = GrepMatch {
				path:           format!("{VIRTUAL_PREFIX}{}", spec.clean),
				line_number:    found.line_number,
				line:           found.line,
				context_before: found.context_before,
				context_after:  found.context_after,
				truncated:      found.truncated,
				match_count:    None,
			};
			match &spec.ranges {
				None => matches.push(m),
				Some(ranges) => matches.extend(filter_ranges(m, ranges)),
			}
		}
	}

	let mut file_order: Vec<String> = Vec::new();
	let mut by_path: HashMap<String, Vec<GrepMatch>> = HashMap::new();
	for m in matches {
		by_path
			.entry(m.path.clone())
			.or_insert_with(|| {
				file_order.push(m.path.clone());
				Vec::new()
			})
			.push(m);
	}
	for list in by_path.values_mut() {
		list.truncate(per_file_cap);
	}
	let total_files = file_order.len();
	let total_label = if run.limit_reached {
		format!("{total_files}+")
	} else {
		total_files.to_string()
	};
	let skip_files = if multi_scope {
		skip.min(total_files)
	} else {
		0
	};
	let window: Vec<String> = if multi_scope {
		file_order
			.iter()
			.skip(skip_files)
			.take(DEFAULT_FILE_LIMIT)
			.cloned()
			.collect()
	} else {
		file_order
	};
	let file_limited = multi_scope && total_files > skip_files + DEFAULT_FILE_LIMIT;
	let mut selected: Vec<GrepMatch> = Vec::new();
	let mut lists: Vec<std::vec::IntoIter<GrepMatch>> = window
		.iter()
		.map(|file| by_path.remove(file).unwrap_or_default().into_iter())
		.collect();
	loop {
		let mut added = false;
		for list in &mut lists {
			if let Some(m) = list.next() {
				selected.push(m);
				added = true;
			}
		}
		if !added {
			break;
		}
	}
	let next_skip = skip_files + window.len();
	let limit_message = file_limited.then(|| {
		format!(
			"Showing files {}-{next_skip} of {total_label}. Use skip={next_skip} for the next page, \
			 or narrow paths/pattern.",
			skip_files + 1
		)
	});

	let explicit_files: Vec<&PathBuf> = match (&scope.exact_files, &scope.targets) {
		(Some(files), _) => files.iter().collect(),
		(None, None) if !scope.is_directory => vec![&scope.search_path],
		_ => Vec::new(),
	};
	let oversized: Vec<String> = explicit_files
		.into_iter()
		.filter(|path| {
			std::fs::metadata(path)
				.is_ok_and(|metadata| metadata.is_file() && metadata.len() > NATIVE_MAX_FILE_BYTES)
		})
		.map(|path| ws.display(path))
		.collect();
	let limit_mb = NATIVE_MAX_FILE_BYTES / (1024 * 1024);
	let oversized_note = (!oversized.is_empty()).then(|| {
		format!(
			"Searched only the first {limit_mb}MB of large files (matches past the {limit_mb}MB \
			 window are not shown; use `read` for the rest): {}",
			oversized.join(", ")
		)
	});
	let scan_note = (oversized_note.is_none() && run.skipped_oversized > 0).then(|| {
		format!(
			"Skipped {} unreadable large file(s); target them directly with `read`",
			run.skipped_oversized
		)
	});
	let missing_note = (!scope.missing.is_empty())
		.then(|| format!("Skipped missing paths: {}", scope.missing.join(", ")));
	let archive_note = (!unreadable.is_empty()).then(|| {
		format!(
			"Skipped archive entries (search supports text members only): {}",
			unreadable.join(", ")
		)
	});
	let warning: Vec<String> = [missing_note, archive_note, oversized_note, scan_note]
		.into_iter()
		.flatten()
		.collect();
	let warning = (!warning.is_empty()).then(|| warning.join("\n"));

	if selected.is_empty() {
		let past_end = multi_scope && skip > 0 && total_files > 0 && skip_files >= total_files;
		let text = if past_end {
			format!("No more results ({total_label} files total; skip={skip} is past the end)")
		} else {
			"No matches found".to_owned()
		};
		return Ok(ToolOutput::text(match warning {
			Some(warning) => format!("{text}\n{warning}"),
			None => text,
		}));
	}

	// `formatResultPath`: directory scopes resolve under the search path; a
	// file scope always names the file itself.
	let format_path = |match_path: &str| -> String {
		if let Some(display) = match_path.strip_prefix(VIRTUAL_PREFIX) {
			return display.to_owned();
		}
		if scope.is_directory {
			ws.display(&scope.search_path.join(match_path.trim_start_matches('/')))
		} else {
			ws.display(&scope.search_path)
		}
	};
	let mut file_list: Vec<String> = Vec::new();
	let mut by_file: HashMap<String, Vec<GrepMatch>> = HashMap::new();
	let mut lines_truncated = false;
	for m in selected {
		let display = format_path(&m.path);
		lines_truncated |= m.truncated.unwrap_or(false);
		by_file
			.entry(display.clone())
			.or_insert_with(|| {
				file_list.push(display.clone());
				Vec::new()
			})
			.push(m);
	}
	let mut tags: HashMap<String, String> = HashMap::new();
	for file in &file_list {
		if let Some(tag) = store.record_file(&ws.root.join(file), None) {
			tags.insert(file.clone(), tag);
		}
	}
	let render = |file: &str| -> Vec<String> {
		let matches = by_file.get(file).map_or(&[][..], Vec::as_slice);
		let hashed = tags.contains_key(file);
		let mut out = Vec::new();
		let mut last: Option<u32> = None;
		let mut push = |line_number: u32, line: &str, is_match: bool, out: &mut Vec<String>| {
			if last.is_some_and(|last| line_number > last + 1) {
				out.push("...".to_owned());
			}
			// `formatMatchLine`: editable `N:` lines only under a snapshot tag.
			let marker = if is_match { '*' } else { ' ' };
			let separator = if hashed { ':' } else { '|' };
			out.push(format!("{marker}{line_number}{separator}{line}"));
			last = Some(line_number);
		};
		for m in matches {
			for ctx in m.context_before.iter().flatten() {
				push(ctx.line_number, &ctx.line, false, &mut out);
			}
			push(m.line_number, &m.line, true, &mut out);
			for ctx in m.context_after.iter().flatten() {
				push(ctx.line_number, &ctx.line, false, &mut out);
			}
		}
		if let Some(tag) = tags.get(file) {
			let seen = pi_edit::store::seen_lines_from_body(&out.join("\n"));
			store.record_seen_lines(&canonical_key(&ws.root.join(file)), tag, &seen);
		}
		out
	};
	let mut output: Vec<String> = Vec::new();
	if multi_scope {
		output.extend(format_grouped_files(&file_list, &mut |file| {
			let suffix = tags
				.get(file)
				.map_or_else(String::new, |tag| format!("#{tag}"));
			(render(file), suffix)
		}));
	} else {
		for file in &file_list {
			let body = render(file);
			if body.is_empty() {
				continue;
			}
			if !output.is_empty() {
				output.push(String::new());
			}
			if let Some(tag) = tags.get(file) {
				output.push(format_hashline_header(file, tag));
			}
			output.extend(body);
		}
	}
	if let Some(message) = limit_message {
		output.push(String::new());
		output.push(message);
	}
	if let Some(warning) = warning {
		output.push(String::new());
		output.push(warning);
	}
	let (text, truncation) = crate::output::head_window(&output.join("\n"), OUTPUT_MAX_BYTES);
	let mut result = ToolOutput::text(text);
	result.truncation = truncation;
	if lines_truncated {
		result
			.limits
			.push(format!("Some lines truncated to {MAX_COLUMNS} chars"));
	}
	Ok(result)
}

#[cfg(test)]
mod tests;
