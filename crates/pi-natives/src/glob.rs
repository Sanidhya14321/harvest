//! Filesystem discovery with glob patterns, ignore semantics, and shared scan
//! caching.
//!
//! # Overview
//! Resolves a search root, scans entries via `pi-walker`, applies glob matching
//! plus optional file-type filtering, and optionally streams each accepted
//! match through a callback.
//!
//! The walker always skips `.git`, and skips `node_modules` unless explicitly
//! requested.
//!
//! # Example
//! ```ignore
//! // JS: await native.glob({ pattern: "*.rs", path: "." })
//! ```

use napi::{
	bindgen_prelude::*,
	threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
};
use napi_derive::napi;

// Re-export entry types so existing `glob::FileType` / `glob::GlobMatch` paths still work.
pub use crate::iofs::{FileType, GlobMatch};
use crate::{iofs, task};

/// Input options for `glob`, including traversal, filtering, and cancellation.
#[napi(object)]
pub struct GlobOptions<'env> {
	/// Glob pattern to match (e.g., "*.ts").
	pub pattern:              String,
	/// Directory to search.
	pub path:                 String,
	/// Filter by file type: "file", "dir", or "symlink". Symlinks are
	/// matched for file/dir filters based on their target type.
	pub file_type:            Option<FileType>,
	/// Match simple patterns recursively by default (`*.ts` -> recursive).
	pub recursive:            Option<bool>,
	/// Include hidden files (default: false).
	pub hidden:               Option<bool>,
	/// Maximum number of results to return.
	pub max_results:          Option<u32>,
	/// Respect .gitignore files (default: true).
	pub gitignore:            Option<bool>,
	/// Enable walker scan caching (default: false).
	pub cache:                Option<bool>,
	/// Sort results by mtime (most recent first) before applying limit.
	pub sort_by_mtime:        Option<bool>,
	/// Include `node_modules` entries when the pattern does not explicitly
	/// mention them.
	pub include_node_modules: Option<bool>,
	/// Abort signal for cancelling the operation.
	pub signal:               Option<Unknown<'env>>,
	/// Timeout in milliseconds for the operation.
	pub timeout_ms:           Option<u32>,
}

/// Result payload returned by a glob operation.
#[napi(object)]
pub struct GlobResult {
	/// Matched filesystem entries.
	pub matches:       Vec<GlobMatch>,
	/// Number of returned matches (`matches.len()`), clamped to `u32::MAX`.
	pub total_matches: u32,
}

fn emit(on_match: Option<&ThreadsafeFunction<GlobMatch>>, matches: &[GlobMatch]) {
	if let Some(callback) = on_match {
		for matched_entry in matches {
			callback.call(Ok(matched_entry.clone()), ThreadsafeFunctionCallMode::NonBlocking);
		}
	}
}

/// Find filesystem entries matching a glob pattern.
///
/// Resolves the search root, scans entries, applies glob and optional file-type
/// filters, and optionally streams each accepted match through `on_match`.
///
/// When `sortByMtime` is enabled, the walker ranks matches by mtime before the
/// native layer applies final symlink-aware file-type filtering and callback
/// emission.
///
/// # Errors
/// Returns an error when the search path cannot be resolved, the path is not a
/// directory, the glob pattern is invalid, or cancellation/timeout is
/// triggered.
#[napi]
pub fn glob(
	options: GlobOptions<'_>,
	#[napi(ts_arg_type = "((error: Error | null, match: GlobMatch) => void) | undefined | null")]
	on_match: Option<ThreadsafeFunction<GlobMatch>>,
) -> task::Promise<GlobResult> {
	let GlobOptions {
		pattern,
		path,
		file_type,
		recursive,
		hidden,
		max_results,
		gitignore,
		sort_by_mtime,
		cache,
		include_node_modules,
		timeout_ms,
		signal,
	} = options;

	let pattern = pattern.trim();
	let pattern = if pattern.is_empty() { "*" } else { pattern };
	let pattern = pattern.to_string();

	let ct = task::CancelToken::new(timeout_ms, signal);

	task::blocking("glob", ct, move |ct| {
		let root = pi_walker::resolve_search_path(&path).map_err(iofs::map_walker_error)?;
		let mut query = pi_walker::glob::GlobQuery::new(root, &pattern);
		query.include_hidden = hidden.unwrap_or(false);
		query.file_type = file_type.map(to_walker_file_type);
		query.recursive = recursive.unwrap_or(true);
		query.max_results = max_results.map_or(usize::MAX, |value| value as usize);
		query.gitignore = gitignore.unwrap_or(true);
		query.include_node_modules =
			include_node_modules.unwrap_or_else(|| pattern.contains("node_modules"));
		query.sort_by_mtime = sort_by_mtime.unwrap_or(false);
		query.cache = cache.unwrap_or(false);
		let matches: Vec<GlobMatch> = pi_walker::glob::glob(&query, || ct.heartbeat())
			.map_err(|error| Error::from_reason(error.to_string()))?
			.into_iter()
			.map(GlobMatch::from)
			.collect();
		emit(on_match.as_ref(), &matches);
		let total_matches = matches.len().min(u32::MAX as usize) as u32;
		Ok(GlobResult { matches, total_matches })
	})
}

const fn to_walker_file_type(file_type: FileType) -> pi_walker::FileType {
	match file_type {
		FileType::File => pi_walker::FileType::File,
		FileType::Dir => pi_walker::FileType::Dir,
		FileType::Symlink => pi_walker::FileType::Symlink,
	}
}
