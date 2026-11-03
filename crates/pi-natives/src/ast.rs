//! N-API adapter for the `pi_ast::engine` ast-grep search and rewrite.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use pi_ast::engine;

use crate::task;

/// ast-grep pattern strictness (controls how patterns match syntax).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[napi(string_enum)]
pub enum AstMatchStrictness {
	/// Match at the concrete syntax tree level.
	#[napi(value = "cst")]
	Cst,
	/// Balanced default suitable for most searches.
	#[napi(value = "smart")]
	Smart,
	/// Match at the AST level.
	#[napi(value = "ast")]
	Ast,
	/// More permissive matching.
	#[napi(value = "relaxed")]
	Relaxed,
	/// Match structural signatures.
	#[napi(value = "signature")]
	Signature,
	/// Template-style pattern matching.
	#[napi(value = "template")]
	Template,
}

/// Options for `astGrep`: patterns, scan scope, and match limits.
#[napi(object)]
pub struct AstFindOptions<'env> {
	/// ast-grep patterns to search for (OR across patterns).
	pub patterns:     Option<Vec<String>>,
	/// Language override; otherwise inferred from file extension per candidate.
	pub lang:         Option<String>,
	/// Single file or directory to scan (combined with `glob` when set).
	pub path:         Option<String>,
	/// Optional glob filter relative to the search root.
	pub glob:         Option<String>,
	/// Rule selector for multi-rule ast-grep configurations.
	pub selector:     Option<String>,
	/// Pattern strictness; defaults to smart matching when omitted.
	pub strictness:   Option<AstMatchStrictness>,
	/// Maximum matches to return after `offset` (default applies when omitted).
	pub limit:        Option<u32>,
	/// Number of leading matches to skip before applying `limit`.
	pub offset:       Option<u32>,
	/// When true, include meta-variable bindings per match.
	pub include_meta: Option<bool>,
	/// Reserved for contextual snippets; not used by the current native find
	/// path.
	pub context:      Option<u32>,
	/// Optional cancellation handle (library-specific).
	pub signal:       Option<Unknown<'env>>,
	/// Wall-clock timeout for the worker task in milliseconds.
	pub timeout_ms:   Option<u32>,
}

/// One ast-grep match with source range and optional meta-variables.
#[napi(object)]
pub struct AstFindMatch {
	/// Display path of the matching file.
	pub path:           String,
	/// Matched source text.
	pub text:           String,
	/// Start byte offset in the file (UTF-8 byte index).
	pub byte_start:     u32,
	/// End byte offset in the file (exclusive UTF-8 byte index).
	pub byte_end:       u32,
	/// 1-based start line.
	pub start_line:     u32,
	/// 1-based start column.
	pub start_column:   u32,
	/// 1-based end line.
	pub end_line:       u32,
	/// 1-based end column.
	pub end_column:     u32,
	/// Meta-variable name to captured text, when `includeMeta` was enabled.
	pub meta_variables: Option<HashMap<String, String>>,
}

/// Aggregated search statistics and any parse or compile diagnostics.
#[napi(object)]
pub struct AstFindResult {
	/// Page of matches after sort, offset, and limit.
	pub matches:            Vec<AstFindMatch>,
	/// Total matches found before paging (can exceed `matches.length`).
	pub total_matches:      u32,
	/// Distinct files that contained at least one match.
	pub files_with_matches: u32,
	/// Files examined for the query.
	pub files_searched:     u32,
	/// True when results were truncated by `limit`.
	pub limit_reached:      bool,
	/// Non-fatal parse or pattern errors collected during the run.
	pub parse_errors:       Option<Vec<String>>,
}

/// Options for `astMatch`: run ast-grep patterns against an in-memory source
/// string instead of files on disk.
#[napi(object)]
pub struct AstMatchOptions<'env> {
	/// Source code to match against (parsed in memory, never read from disk).
	pub source:       String,
	/// Language of `source` (required; e.g. "ts", "tsx", "rust", "python").
	pub lang:         String,
	/// ast-grep patterns to search for (OR across patterns).
	pub patterns:     Vec<String>,
	/// Rule selector for multi-rule ast-grep configurations.
	pub selector:     Option<String>,
	/// Pattern strictness; defaults to smart matching when omitted.
	pub strictness:   Option<AstMatchStrictness>,
	/// Maximum matches to return after `offset` (default applies when omitted).
	pub limit:        Option<u32>,
	/// Number of leading matches to skip before applying `limit`.
	pub offset:       Option<u32>,
	/// When true, include meta-variable bindings per match.
	pub include_meta: Option<bool>,
	/// Optional cancellation handle (library-specific).
	pub signal:       Option<Unknown<'env>>,
	/// Wall-clock timeout for the worker task in milliseconds.
	pub timeout_ms:   Option<u32>,
}

/// Result of an in-memory `astMatch` run.
#[napi(object)]
pub struct AstMatchResult {
	/// Page of matches after sort, offset, and limit.
	pub matches:       Vec<AstFindMatch>,
	/// Total matches found before paging (can exceed `matches.length`).
	pub total_matches: u32,
	/// True when results were truncated by `limit`.
	pub limit_reached: bool,
	/// Non-fatal parse or pattern-compile errors collected during the run.
	pub parse_errors:  Option<Vec<String>>,
}

/// Options for `astEdit`: rewrite rules, scan scope, safety limits, and
/// dry-run.
#[napi(object)]
pub struct AstReplaceOptions<'env> {
	/// Map of pattern string to replacement template.
	pub rewrites:            Option<HashMap<String, String>>,
	/// Language override applied to every file; otherwise inferred per file, so
	/// mixed-language paths rewrite each file in its own language.
	pub lang:                Option<String>,
	/// Single file or directory to rewrite.
	pub path:                Option<String>,
	/// Optional glob filter within the search root.
	pub glob:                Option<String>,
	/// Rule selector for multi-rule configurations.
	pub selector:            Option<String>,
	/// Pattern strictness for rewrites.
	pub strictness:          Option<AstMatchStrictness>,
	/// When true (default), compute changes without writing files.
	pub dry_run:             Option<bool>,
	/// Cap on replacement applications across all files.
	pub max_replacements:    Option<u32>,
	/// Cap on distinct files that may be modified.
	pub max_files:           Option<u32>,
	/// Fail the operation when a file cannot be parsed for rewriting.
	pub fail_on_parse_error: Option<bool>,
	/// Optional cancellation handle.
	pub signal:              Option<Unknown<'env>>,
	/// Wall-clock timeout for the worker task in milliseconds.
	pub timeout_ms:          Option<u32>,
}

/// One textual replacement applied to a file (before/after slice and
/// coordinates).
#[napi(object)]
pub struct AstReplaceChange {
	/// File path for this change.
	pub path:           String,
	/// Original matched text.
	pub before:         String,
	/// Replacement text.
	pub after:          String,
	/// Start byte offset of the replaced span.
	pub byte_start:     u32,
	/// End byte offset of the replaced span (exclusive).
	pub byte_end:       u32,
	/// Length of deleted text in bytes (may differ from `byteEnd - byteStart`
	/// for edge cases).
	pub deleted_length: u32,
	/// 1-based start line of the match.
	pub start_line:     u32,
	/// 1-based start column.
	pub start_column:   u32,
	/// 1-based end line.
	pub end_line:       u32,
	/// 1-based end column.
	pub end_column:     u32,
}

/// Per-file replacement count after an `astEdit` run.
#[napi(object)]
pub struct AstReplaceFileChange {
	/// File that had replacements.
	pub path:  String,
	/// Number of replacements in that file.
	pub count: u32,
}

/// Summary of an ast-grep rewrite pass, including whether disk writes occurred.
#[napi(object)]
pub struct AstReplaceResult {
	/// Individual replacement records (may be large).
	pub changes:            Vec<AstReplaceChange>,
	/// Replacement counts grouped by file.
	pub file_changes:       Vec<AstReplaceFileChange>,
	/// Total replacements applied or previewed.
	pub total_replacements: u32,
	/// Files that had at least one replacement.
	pub files_touched:      u32,
	/// Files considered for rewriting.
	pub files_searched:     u32,
	/// False when `dryRun` prevented writing.
	pub applied:            bool,
	/// True when limits stopped further replacements.
	pub limit_reached:      bool,
	/// Parse or pattern errors when not failing the whole operation.
	pub parse_errors:       Option<Vec<String>>,
}

impl From<AstMatchStrictness> for engine::AstMatchStrictness {
	fn from(value: AstMatchStrictness) -> Self {
		match value {
			AstMatchStrictness::Cst => Self::Cst,
			AstMatchStrictness::Smart => Self::Smart,
			AstMatchStrictness::Ast => Self::Ast,
			AstMatchStrictness::Relaxed => Self::Relaxed,
			AstMatchStrictness::Signature => Self::Signature,
			AstMatchStrictness::Template => Self::Template,
		}
	}
}

impl From<engine::AstFindMatch> for AstFindMatch {
	fn from(value: engine::AstFindMatch) -> Self {
		Self {
			path:           value.path,
			text:           value.text,
			byte_start:     value.byte_start,
			byte_end:       value.byte_end,
			start_line:     value.start_line,
			start_column:   value.start_column,
			end_line:       value.end_line,
			end_column:     value.end_column,
			meta_variables: value.meta_variables,
		}
	}
}

impl From<engine::AstFindResult> for AstFindResult {
	fn from(value: engine::AstFindResult) -> Self {
		Self {
			matches:            value.matches.into_iter().map(Into::into).collect(),
			total_matches:      value.total_matches,
			files_with_matches: value.files_with_matches,
			files_searched:     value.files_searched,
			limit_reached:      value.limit_reached,
			parse_errors:       value.parse_errors,
		}
	}
}

impl From<engine::AstMatchResult> for AstMatchResult {
	fn from(value: engine::AstMatchResult) -> Self {
		Self {
			matches:       value.matches.into_iter().map(Into::into).collect(),
			total_matches: value.total_matches,
			limit_reached: value.limit_reached,
			parse_errors:  value.parse_errors,
		}
	}
}

impl From<engine::AstReplaceChange> for AstReplaceChange {
	fn from(value: engine::AstReplaceChange) -> Self {
		Self {
			path:           value.path,
			before:         value.before,
			after:          value.after,
			byte_start:     value.byte_start,
			byte_end:       value.byte_end,
			deleted_length: value.deleted_length,
			start_line:     value.start_line,
			start_column:   value.start_column,
			end_line:       value.end_line,
			end_column:     value.end_column,
		}
	}
}

impl From<engine::AstReplaceFileChange> for AstReplaceFileChange {
	fn from(value: engine::AstReplaceFileChange) -> Self {
		Self { path: value.path, count: value.count }
	}
}

impl From<engine::AstReplaceResult> for AstReplaceResult {
	fn from(value: engine::AstReplaceResult) -> Self {
		Self {
			changes:            value.changes.into_iter().map(Into::into).collect(),
			file_changes:       value.file_changes.into_iter().map(Into::into).collect(),
			total_replacements: value.total_replacements,
			files_touched:      value.files_touched,
			files_searched:     value.files_searched,
			applied:            value.applied,
			limit_reached:      value.limit_reached,
			parse_errors:       value.parse_errors,
		}
	}
}

fn to_napi_error(err: engine::Error) -> Error {
	Error::from_reason(err.to_string())
}

/// Search source files with ast-grep patterns; returns a promise resolved on a
/// worker thread.
#[napi]
pub fn ast_grep(options: AstFindOptions<'_>) -> task::Promise<AstFindResult> {
	let config = engine::AstFindOptions {
		patterns:     options.patterns,
		lang:         options.lang,
		path:         options.path,
		glob:         options.glob,
		selector:     options.selector,
		strictness:   options.strictness.map(Into::into),
		limit:        options.limit,
		offset:       options.offset,
		include_meta: options.include_meta,
		context:      options.context,
	};
	let ct = task::CancelToken::new(options.timeout_ms, options.signal);
	task::blocking("ast_grep", ct, move |ct| {
		engine::ast_grep(config, ct.into_core())
			.map(Into::into)
			.map_err(to_napi_error)
	})
}

/// Match ast-grep patterns against an in-memory source string; returns a
/// promise resolved on a worker thread.
///
/// This is the file-free counterpart to [`ast_grep`]: callers that already hold
/// the source (streaming buffers, generated code, editor contents) avoid a
/// temp-file round trip. `lang` is required since there is no path to infer it
/// from.
#[napi]
pub fn ast_match(options: AstMatchOptions<'_>) -> task::Promise<AstMatchResult> {
	let config = engine::AstMatchOptions {
		source:       options.source,
		lang:         options.lang,
		patterns:     options.patterns,
		selector:     options.selector,
		strictness:   options.strictness.map(Into::into),
		limit:        options.limit,
		offset:       options.offset,
		include_meta: options.include_meta,
	};
	let ct = task::CancelToken::new(options.timeout_ms, options.signal);
	task::blocking("ast_match", ct, move |ct| {
		engine::ast_match(config, ct.into_core())
			.map(Into::into)
			.map_err(to_napi_error)
	})
}

/// Apply ast-grep rewrite rules to matching files; honors `dryRun` and returns
/// a promise.
#[napi]
pub fn ast_edit(options: AstReplaceOptions<'_>) -> task::Promise<AstReplaceResult> {
	let config = engine::AstReplaceOptions {
		rewrites:            options.rewrites,
		lang:                options.lang,
		path:                options.path,
		glob:                options.glob,
		selector:            options.selector,
		strictness:          options.strictness.map(Into::into),
		dry_run:             options.dry_run,
		max_replacements:    options.max_replacements,
		max_files:           options.max_files,
		fail_on_parse_error: options.fail_on_parse_error,
	};
	let ct = task::CancelToken::new(options.timeout_ms, options.signal);
	task::blocking("ast_edit", ct, move |ct| {
		engine::ast_edit(config, ct.into_core())
			.map(Into::into)
			.map_err(to_napi_error)
	})
}
