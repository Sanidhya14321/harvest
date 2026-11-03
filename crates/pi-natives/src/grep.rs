//! N-API adapter for the `pi-grep` search engine.
//!
//! Provides two layers:
//! - `search()` for in-memory content search.
//! - `grep()` for filesystem search with glob/type filtering.

use napi::{
	JsString,
	bindgen_prelude::*,
	threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
};
use napi_derive::napi;
use pi_grep as engine;

use crate::task;

/// Output mode for [`search`] and [`grep`] (string values match JS callers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[napi(string_enum)]
pub enum GrepOutputMode {
	/// Emit matched lines (and optional context lines).
	#[napi(value = "content")]
	Content,
	/// Emit per-file or total counts instead of line content.
	#[napi(value = "count")]
	Count,
	/// Emit one row per file that matched, without line content.
	#[napi(value = "filesWithMatches")]
	FilesWithMatches,
}

impl From<GrepOutputMode> for engine::GrepOutputMode {
	fn from(mode: GrepOutputMode) -> Self {
		match mode {
			GrepOutputMode::Content => Self::Content,
			GrepOutputMode::Count => Self::Count,
			GrepOutputMode::FilesWithMatches => Self::FilesWithMatches,
		}
	}
}

/// Options for searching file content.
#[napi(object)]
pub struct SearchOptions {
	/// Regex pattern to search for.
	pub pattern:        String,
	/// Case-insensitive search.
	pub ignore_case:    Option<bool>,
	/// Enable multiline matching.
	pub multiline:      Option<bool>,
	/// Maximum number of matches to return.
	pub max_count:      Option<u32>,
	/// Skip first N matches.
	pub offset:         Option<u32>,
	/// Lines of context before matches.
	pub context_before: Option<u32>,
	/// Lines of context after matches.
	pub context_after:  Option<u32>,
	/// Lines of context before/after matches (legacy).
	pub context:        Option<u32>,
	/// Truncate lines longer than this (characters).
	pub max_columns:    Option<u32>,
	/// Output mode (content or count).
	pub mode:           Option<GrepOutputMode>,
}

/// Options for searching files on disk.
#[napi(object)]
pub struct GrepOptions<'env> {
	/// Regex pattern to search for.
	pub pattern:            String,
	/// Directory or file to search.
	pub path:               String,
	/// Glob filter for filenames (e.g., "*.ts").
	pub glob:               Option<String>,
	/// Filter by file type (e.g., "js", "py", "rust").
	pub r#type:             Option<String>,
	/// Case-insensitive search.
	pub ignore_case:        Option<bool>,
	/// Enable multiline matching.
	pub multiline:          Option<bool>,
	/// Include hidden files (default: true).
	pub hidden:             Option<bool>,
	/// Respect .gitignore files (default: true).
	pub gitignore:          Option<bool>,
	/// Maximum number of matches to return.
	pub max_count:          Option<u32>,
	/// Skip first N matches.
	pub offset:             Option<u32>,
	/// Lines of context before matches.
	pub context_before:     Option<u32>,
	/// Lines of context after matches.
	pub context_after:      Option<u32>,
	/// Lines of context before/after matches (legacy).
	pub context:            Option<u32>,
	/// Truncate lines longer than this (characters).
	pub max_columns:        Option<u32>,
	/// Output mode (content, filesWithMatches, or count).
	pub mode:               Option<GrepOutputMode>,
	/// Maximum matches collected per file (content mode). Keeps one hot file
	/// from exhausting the global `max_count` budget before other files are
	/// reached.
	pub max_count_per_file: Option<u32>,
	/// Abort signal for cancelling the operation.
	pub signal:             Option<Unknown<'env>>,
	/// Timeout in milliseconds for the operation.
	pub timeout_ms:         Option<u32>,
}

/// A context line (before or after a match).
#[derive(Clone, Debug)]
#[napi(object)]
pub struct ContextLine {
	/// 1-indexed line number in the source file.
	pub line_number: u32,
	/// Raw line content (trimmed line ending).
	pub line:        String,
}

/// A single match in the content.
#[napi(object)]
pub struct Match {
	/// 1-indexed line number.
	pub line_number:    u32,
	/// The matched line content.
	pub line:           String,
	/// Context lines before the match.
	pub context_before: Option<Vec<ContextLine>>,
	/// Context lines after the match.
	pub context_after:  Option<Vec<ContextLine>>,
	/// Whether the line was truncated.
	pub truncated:      Option<bool>,
}

/// Result of searching content.
#[napi(object)]
pub struct SearchResult {
	/// All matches found.
	pub matches:       Vec<Match>,
	/// Total number of matches (may exceed `matches.len()` due to offset/limit).
	pub match_count:   u32,
	/// Whether the limit was reached.
	pub limit_reached: bool,
	/// Error message, if any.
	pub error:         Option<String>,
}

/// A single match in a grep result.
#[derive(Clone)]
#[napi(object)]
pub struct GrepMatch {
	/// File path for the match (relative for directory searches).
	pub path:           String,
	/// 1-indexed line number (0 for count-only entries).
	pub line_number:    u32,
	/// The matched line content (empty for count-only entries).
	pub line:           String,
	/// Context lines before the match.
	pub context_before: Option<Vec<ContextLine>>,
	/// Context lines after the match.
	pub context_after:  Option<Vec<ContextLine>>,
	/// Whether the line was truncated.
	pub truncated:      Option<bool>,
	/// Per-file match count (count mode only).
	pub match_count:    Option<u32>,
}

/// Result of searching files.
#[napi(object)]
pub struct GrepResult {
	/// Matches or per-file counts, depending on output mode.
	pub matches:            Vec<GrepMatch>,
	/// Total matches across all files, or matched file count in filesWithMatches
	/// mode.
	pub total_matches:      u32,
	/// Number of files with at least one match.
	pub files_with_matches: u32,
	/// Number of files searched.
	pub files_searched:     u32,
	/// Whether the limit/offset stopped the search early.
	pub limit_reached:      Option<bool>,
	/// Number of files skipped because they exceed the size limit.
	pub skipped_oversized:  Option<u32>,
}

fn context(lines: Vec<engine::ContextLine>) -> Vec<ContextLine> {
	lines
		.into_iter()
		.map(|line| ContextLine { line_number: line.line_number, line: line.line })
		.collect()
}

impl From<engine::GrepMatch> for GrepMatch {
	fn from(m: engine::GrepMatch) -> Self {
		Self {
			path:           m.path,
			line_number:    m.line_number,
			line:           m.line,
			context_before: m.context_before.map(context),
			context_after:  m.context_after.map(context),
			truncated:      m.truncated,
			match_count:    m.match_count,
		}
	}
}

fn to_napi_error(err: engine::Error) -> Error {
	Error::from_reason(err.to_string())
}

fn search_sync(content: &[u8], options: SearchOptions) -> SearchResult {
	let result = engine::search_sync(content, engine::SearchOptions {
		pattern:        options.pattern,
		ignore_case:    options.ignore_case,
		multiline:      options.multiline,
		max_count:      options.max_count,
		offset:         options.offset,
		context_before: options.context_before,
		context_after:  options.context_after,
		context:        options.context,
		max_columns:    options.max_columns,
		mode:           options.mode.map(Into::into),
	});
	SearchResult {
		matches:       result
			.matches
			.into_iter()
			.map(|m| Match {
				line_number:    m.line_number,
				line:           m.line,
				context_before: m.context_before.map(context),
				context_after:  m.context_after.map(context),
				truncated:      m.truncated,
			})
			.collect(),
		match_count:   result.match_count,
		limit_reached: result.limit_reached,
		error:         result.error,
	}
}

/// Search content for a pattern (one-shot, compiles pattern each time).
/// For repeated searches with the same pattern, use [`grep`] with file filters.
///
/// # Arguments
/// - `content`: `Uint8Array`/`Buffer` (zero-copy) or `string` (UTF-8).
/// - `options`: Regex settings, context, and output mode.
///
/// # Returns
/// Match list plus counts/limit status; errors are surfaced in `error`.
#[napi]
pub fn search(content: Either<JsString, Uint8Array>, options: SearchOptions) -> SearchResult {
	match &content {
		Either::A(js_str) => match js_str.into_utf8() {
			Ok(utf8) => search_sync(utf8.as_slice(), options),
			Err(err) => SearchResult {
				matches:       Vec::new(),
				match_count:   0,
				limit_reached: false,
				error:         Some(err.to_string()),
			},
		},
		Either::B(buf) => search_sync(buf.as_ref(), options),
	}
}

/// Quick check if content matches a pattern.
///
/// # Arguments
/// - `content`: `Uint8Array`/`Buffer` (zero-copy) or `string` (UTF-8).
/// - `pattern`: `Uint8Array`/`Buffer` (zero-copy) or `string` (UTF-8).
/// - `ignore_case`: Case-insensitive matching.
/// - `multiline`: Enable multiline regex mode.
///
/// # Returns
/// True if any match exists; false on no match.
#[napi]
pub fn has_match(
	content: Either<JsString, Uint8Array>,
	pattern: Either<JsString, Uint8Array>,
	ignore_case: Option<bool>,
	multiline: Option<bool>,
) -> Result<bool> {
	let content_utf8;
	let content_slice: &[u8] = match &content {
		Either::A(js_str) => {
			content_utf8 = js_str.into_utf8()?;
			content_utf8.as_slice()
		},
		Either::B(buf) => buf.as_ref(),
	};

	let pattern_utf8;
	let pattern_string;
	let pattern_ref: &str = match &pattern {
		Either::A(js_str) => {
			pattern_utf8 = js_str.into_utf8()?;
			pattern_utf8.as_str()?
		},
		Either::B(buf) => {
			pattern_string = std::str::from_utf8(buf.as_ref())
				.map_err(|err| Error::from_reason(format!("Invalid UTF-8 in pattern: {err}")))?
				.to_owned();
			&pattern_string
		},
	};

	engine::has_match(
		content_slice,
		pattern_ref,
		ignore_case.unwrap_or(false),
		multiline.unwrap_or(false),
	)
	.map_err(to_napi_error)
}

/// Search files for a regex pattern.
///
/// # Arguments
/// - `options`: Pattern, path, filters, and output mode.
/// - `on_match`: Optional callback invoked per match/result.
///
/// # Returns
/// Aggregated results across matching files.
#[napi]
pub fn grep(
	options: GrepOptions<'_>,
	#[napi(ts_arg_type = "((error: Error | null, match: GrepMatch) => void) | undefined | null")]
	on_match: Option<ThreadsafeFunction<GrepMatch>>,
) -> task::Promise<GrepResult> {
	let GrepOptions {
		pattern,
		path,
		glob,
		r#type,
		ignore_case,
		multiline,
		hidden,
		gitignore,
		max_count,
		offset,
		context_before,
		context_after,
		context,
		max_columns,
		mode,
		max_count_per_file,
		timeout_ms,
		signal,
	} = options;

	let config = engine::GrepConfig {
		pattern,
		path,
		glob,
		type_filter: r#type,
		ignore_case,
		multiline,
		hidden,
		gitignore,
		max_count,
		max_count_per_file,
		offset,
		context_before,
		context_after,
		context,
		max_columns,
		mode: mode.map(Into::into),
	};
	let ct = task::CancelToken::new(timeout_ms, signal);
	task::blocking("grep", ct, move |ct| {
		let result = engine::grep_sync(config, ct.into_core()).map_err(to_napi_error)?;
		let matches: Vec<GrepMatch> = result.matches.into_iter().map(Into::into).collect();
		// Fire callbacks after aggregation so offset/limit semantics match
		// returned results.
		if let Some(callback) = on_match.as_ref() {
			for grep_match in &matches {
				callback.call(Ok(grep_match.clone()), ThreadsafeFunctionCallMode::NonBlocking);
			}
		}
		Ok(GrepResult {
			matches,
			total_matches: result.total_matches,
			files_with_matches: result.files_with_matches,
			files_searched: result.files_searched,
			limit_reached: result.limit_reached,
			skipped_oversized: result.skipped_oversized,
		})
	})
}
