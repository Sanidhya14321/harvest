//! Single-pass workspace scan: bounded tree entries plus directory-scoped
//! `AGENTS.md` files.
//!
//! `AGENTS.md` files are checked directly in every traversed directory so a
//! file-level gitignore rule cannot hide them, while ignored directories are
//! still pruned by the walker.
use std::{
	collections::HashSet,
	fmt,
	path::{Path, PathBuf},
	sync::LazyLock,
};

use crate::{
	CollectedEntry, DirectoryErrorMode, FileType, FollowLinks, WalkDetail, WalkError, WalkOrder,
	WalkRequest, classify_file_type, normalize_relative_path,
};

const AGENTS_MD_FILENAME: &str = "AGENTS.md";
const AGENTS_MD_MIN_DEPTH: usize = 1;
const AGENTS_MD_MAX_DEPTH: usize = 4;
/// Cap on surfaced `AGENTS.md` files.
pub const AGENTS_MD_LIMIT: usize = 200;
const MAX_ENTRIES: usize = 100_000;

/// Directory names pruned during workspace scans.
pub const EXCLUDED_DIRS: &[&str] = &[
	"node_modules",
	".git",
	".next",
	"dist",
	"build",
	"target",
	".venv",
	".cache",
	".turbo",
	".parcel-cache",
	"coverage",
];

static EXCLUDED_DIR_SET: LazyLock<HashSet<&'static str>> =
	LazyLock::new(|| EXCLUDED_DIRS.iter().copied().collect());

/// One workspace scan.
#[derive(Debug, Clone)]
pub struct WorkspaceQuery {
	pub root:              PathBuf,
	/// Maximum depth for returned tree entries. Root children are depth 1.
	pub max_depth:         usize,
	pub include_hidden:    bool,
	pub gitignore:         bool,
	/// Also surface `AGENTS.md` files at directory depth 1..=4 (walking deeper
	/// than `max_depth` to find them).
	pub collect_agents_md: bool,
}

/// Scan result.
#[derive(Debug, Default)]
pub struct WorkspaceListing {
	/// Entries within `max_depth`, sorted by path, with mtime and file size.
	pub entries:         Vec<CollectedEntry>,
	/// Directory-scoped `AGENTS.md` files (capped at [`AGENTS_MD_LIMIT`]).
	pub agents_md_files: Vec<String>,
	/// True when any output cap was hit.
	pub truncated:       bool,
}

fn entry_at(root: &Path, path: &Path) -> Option<CollectedEntry> {
	let relative = normalize_relative_path(root, path);
	if relative.is_empty() {
		return None;
	}
	let (file_type, mtime, size) = classify_file_type(path)?;
	Some(CollectedEntry {
		path: relative.into_owned(),
		file_type,
		mtime,
		size: size.map(|value| value as f64),
	})
}

fn is_file_or_file_symlink(path: &Path, file_type: FileType) -> bool {
	match file_type {
		FileType::File => true,
		FileType::Symlink => std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()),
		FileType::Dir => false,
	}
}

fn is_excluded(relative: &str, file_type: FileType) -> bool {
	let mut components = relative
		.split('/')
		.filter(|component| !component.is_empty())
		.peekable();
	while let Some(component) = components.next() {
		if component == ".DS_Store" {
			return true;
		}
		let is_final = components.peek().is_none();
		if EXCLUDED_DIR_SET.contains(component) && (!is_final || file_type == FileType::Dir) {
			return true;
		}
	}
	false
}

fn collect_agents_md(
	query: &WorkspaceQuery,
	directory: &Path,
	directory_depth: usize,
	listing: &mut WorkspaceListing,
) {
	if !query.collect_agents_md || listing.agents_md_files.len() >= AGENTS_MD_LIMIT {
		return;
	}
	let candidate = directory.join(AGENTS_MD_FILENAME);
	let Some(entry) = entry_at(&query.root, &candidate) else {
		return;
	};
	if !is_file_or_file_symlink(&candidate, entry.file_type) {
		return;
	}
	if directory_depth < query.max_depth {
		listing.entries.push(entry.clone());
	}
	if (AGENTS_MD_MIN_DEPTH..=AGENTS_MD_MAX_DEPTH).contains(&directory_depth) {
		listing.agents_md_files.push(entry.path);
	}
}

/// Walk the workspace once; `heartbeat` errors abort the walk.
pub fn list_workspace<E, H>(
	query: &WorkspaceQuery,
	heartbeat: H,
) -> Result<WorkspaceListing, WalkError<String>>
where
	H: Fn() -> Result<(), E> + Sync,
	E: fmt::Display,
{
	let walk_max_depth = if query.collect_agents_md {
		query.max_depth.max(AGENTS_MD_MAX_DEPTH)
	} else {
		query.max_depth
	};
	let mut listing = WorkspaceListing::default();
	collect_agents_md(query, &query.root, 0, &mut listing);
	let outcome = WalkRequest::new(query.root.clone())
		.hidden(query.include_hidden)
		.gitignore(query.gitignore)
		.skip_git(true)
		.skip_node_modules(true)
		.follow_links(FollowLinks::Never)
		.detail(WalkDetail::Full)
		.order(WalkOrder::Path)
		.emit_root(false)
		.depth(1, walk_max_depth)
		.directory_errors(DirectoryErrorMode::SkipSkippable)
		.cache(false)
		.limit(MAX_ENTRIES + 1)
		.collect_with_heartbeat(heartbeat)?;

	let mut truncated = outcome.stats.limited_entries > 0;
	for entry in outcome.entries {
		if is_excluded(&entry.path, entry.file_type) {
			continue;
		}
		let depth = entry.depth();
		if entry.file_type == FileType::Dir {
			collect_agents_md(query, &entry.absolute_path(&query.root), depth, &mut listing);
		}
		if depth <= query.max_depth {
			listing.entries.push(entry);
			if listing.entries.len() >= MAX_ENTRIES {
				truncated = true;
				break;
			}
		}
	}
	listing.entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
	listing.entries.dedup_by(|a, b| a.path == b.path);
	listing.agents_md_files.sort_unstable();
	listing.agents_md_files.dedup();
	if listing.entries.len() > MAX_ENTRIES {
		truncated = true;
		listing.entries.truncate(MAX_ENTRIES);
	}
	let agents_truncated = listing.agents_md_files.len() >= AGENTS_MD_LIMIT;
	listing.agents_md_files.truncate(AGENTS_MD_LIMIT);
	listing.truncated = truncated || agents_truncated;
	Ok(listing)
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	fn query(root: &Path, max_depth: usize) -> WorkspaceQuery {
		WorkspaceQuery {
			root: root.to_path_buf(),
			max_depth,
			include_hidden: true,
			gitignore: false,
			collect_agents_md: true,
		}
	}

	#[test]
	fn excluded_dirs_are_pruned_and_depth_is_bounded() {
		let root = tempfile::tempdir().unwrap();
		for path in
			["src/a/deep.rs", "node_modules/x/y.js", "target/debug/z", "README.md", ".DS_Store"]
		{
			let path = root.path().join(path);
			fs::create_dir_all(path.parent().unwrap()).unwrap();
			fs::write(path, "x").unwrap();
		}
		let listing =
			list_workspace(&query(root.path(), 2), || Ok::<(), std::convert::Infallible>(())).unwrap();
		let paths: Vec<&str> = listing
			.entries
			.iter()
			.map(|entry| entry.path.as_str())
			.collect();
		assert_eq!(paths, ["README.md", "src", "src/a"]);
		let readme = listing
			.entries
			.iter()
			.find(|entry| entry.path == "README.md")
			.unwrap();
		assert_eq!(readme.size, Some(1.0));
	}

	#[test]
	fn agents_md_is_found_below_the_tree_depth() {
		let root = tempfile::tempdir().unwrap();
		fs::create_dir_all(root.path().join("a/b/c")).unwrap();
		fs::write(root.path().join("AGENTS.md"), "root").unwrap();
		fs::write(root.path().join("a/b/c/AGENTS.md"), "deep").unwrap();
		let listing =
			list_workspace(&query(root.path(), 1), || Ok::<(), std::convert::Infallible>(())).unwrap();
		assert_eq!(listing.agents_md_files, ["a/b/c/AGENTS.md"]);
		assert!(
			listing
				.entries
				.iter()
				.any(|entry| entry.path == "AGENTS.md")
		);
		assert!(
			!listing
				.entries
				.iter()
				.any(|entry| entry.path.starts_with("a/b"))
		);
	}
}
