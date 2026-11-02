//! Workspace discovery for startup context.
//!
//! Walks a project tree once and returns the bounded entries needed to render
//! the workspace tree plus directory-scoped AGENTS.md files. AGENTS.md files
//! are checked directly in every traversed directory so a file-level gitignore
//! rule cannot hide them, while ignored directories are still pruned by the
//! walker.

use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::{
	iofs::{self, GlobMatch},
	task,
};

/// Input options for `listWorkspace`, the single-pass workspace startup scan.
#[napi(object)]
pub struct ListWorkspaceOptions<'env> {
	/// Directory to scan.
	pub path:              String,
	/// Maximum depth for returned tree entries. Root children are depth 1.
	pub max_depth:         u32,
	/// Include hidden files and directories. Default: false.
	pub hidden:            Option<bool>,
	/// Respect .gitignore files. Default: true.
	pub gitignore:         Option<bool>,
	/// Also surface AGENTS.md files in directories at depth 1..=4, even when
	/// gitignore would otherwise hide the file. Walks deeper than `maxDepth`
	/// to find them. Default: false.
	pub collect_agents_md: Option<bool>,
	/// Timeout in milliseconds for the operation.
	pub timeout_ms:        Option<u32>,
	/// Abort signal for cancelling the operation.
	pub signal:            Option<Unknown<'env>>,
}

/// Result payload returned by a workspace scan.
#[napi(object)]
pub struct ListWorkspaceResult {
	/// Entries within `maxDepth`, with mtime and regular-file size metadata.
	pub entries:         Vec<GlobMatch>,
	/// Directory-scoped AGENTS.md files within depth 1..=4 (capped at 200).
	/// Always empty when `collectAgentsMd` is false.
	pub agents_md_files: Vec<String>,
	/// True when any output cap was hit.
	pub truncated:       bool,
}

/// Walk the workspace once and return tree entries plus AGENTS.md candidates.
///
/// File-level ignore rules for AGENTS.md are bypassed by checking each
/// traversed directory directly when `collectAgentsMd` is enabled, but ignored
/// directories are still pruned by the walker and are not searched.
#[napi(js_name = "listWorkspace")]
pub fn list_workspace(options: ListWorkspaceOptions<'_>) -> task::Promise<ListWorkspaceResult> {
	let ListWorkspaceOptions {
		path,
		max_depth,
		hidden,
		gitignore,
		collect_agents_md,
		timeout_ms,
		signal,
	} = options;

	let ct = task::CancelToken::new(timeout_ms, signal);
	task::blocking("listWorkspace", ct, move |ct| {
		let query = pi_walker::workspace::WorkspaceQuery {
			root:              pi_walker::resolve_search_path(&path)
				.map_err(iofs::map_walker_error)?,
			max_depth:         max_depth as usize,
			include_hidden:    hidden.unwrap_or(false),
			gitignore:         gitignore.unwrap_or(true),
			collect_agents_md: collect_agents_md.unwrap_or(false),
		};
		let listing = pi_walker::workspace::list_workspace(&query, || ct.heartbeat())
			.map_err(iofs::map_walker_error)?;
		Ok(ListWorkspaceResult {
			entries:         listing.entries.into_iter().map(GlobMatch::from).collect(),
			agents_md_files: listing.agents_md_files,
			truncated:       listing.truncated,
		})
	})
}
