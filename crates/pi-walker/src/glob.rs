//! Glob search over the walker: pattern normalization, depth bounding,
//! mtime ranking and symlink-aware file-type filtering.
//!
//! This is the engine behind the `glob` tool. It is host-neutral: callers
//! supply cancellation through the heartbeat closure.
use std::{cmp::Ordering, fmt, path::Path};

use crate::{
	CollectedEntry, CompiledWalkGlob, DirectoryErrorMode, EmptyRecheck, FileType, FollowLinks,
	WalkDetail, WalkError, WalkFilter, WalkOrder, WalkRank, WalkRequest,
};

/// Normalize a raw glob: fix separators, prepend `**/` for recursive simple
/// patterns, and close unclosed `{` alternations.
pub fn build_glob_pattern(glob: &str, recursive: bool) -> String {
	let normalized = glob.replace('\\', "/");
	let pattern = if !recursive
		|| normalized.contains('/')
		|| normalized.starts_with("**")
		|| is_exact_brace_union(&normalized)
	{
		normalized
	} else {
		format!("**/{normalized}")
	};
	fix_unclosed_braces(pattern)
}

/// Maximum walk depth a normalized pattern can match (`usize::MAX` when
/// unbounded).
///
/// Walk globs use literal separators, so a pattern with N literal segments can
/// only match entries at most N components deep. `**` and `{...}` (which may
/// contain `/`) disable the bound.
pub fn walk_depth_bound(pattern: &str) -> usize {
	if pattern.contains("**") || pattern.contains('{') {
		return usize::MAX;
	}
	pattern
		.split('/')
		.filter(|segment| !segment.is_empty())
		.count()
		.max(1)
}

/// LLMs occasionally emit `*.{ts,js` without the closing `}`; append it.
fn fix_unclosed_braces(pattern: String) -> String {
	let opens = pattern.chars().filter(|&c| c == '{').count();
	let closes = pattern.chars().filter(|&c| c == '}').count();
	if opens > closes {
		let mut fixed = pattern;
		fixed.extend(std::iter::repeat_n('}', opens - closes));
		fixed
	} else {
		pattern
	}
}

fn is_exact_brace_union(pattern: &str) -> bool {
	if !(pattern.starts_with('{') && pattern.ends_with('}')) {
		return false;
	}
	let inner = &pattern[1..pattern.len() - 1];
	!inner.is_empty()
		&& !inner
			.chars()
			.any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

/// One glob search.
#[derive(Debug, Clone)]
pub struct GlobQuery {
	/// Resolved search root.
	pub root:                 std::path::PathBuf,
	/// Raw pattern (empty means `*`).
	pub pattern:              String,
	/// Prefix simple patterns with `**/`.
	pub recursive:            bool,
	pub include_hidden:       bool,
	/// Keep only this kind; symlinks match by their target's kind.
	pub file_type:            Option<FileType>,
	pub max_results:          usize,
	pub gitignore:            bool,
	/// Do not skip `node_modules` (defaults to whether the pattern names it).
	pub include_node_modules: bool,
	/// Rank by mtime descending (then path) before truncating.
	pub sort_by_mtime:        bool,
	pub cache:                bool,
}

impl GlobQuery {
	/// A query with the native defaults: recursive, gitignore-respecting,
	/// hidden excluded, unlimited, unsorted.
	pub fn new(root: impl Into<std::path::PathBuf>, pattern: &str) -> Self {
		let pattern = match pattern.trim() {
			"" => "*",
			trimmed => trimmed,
		};
		Self {
			root:                 root.into(),
			include_node_modules: pattern.contains("node_modules"),
			pattern:              pattern.to_owned(),
			recursive:            true,
			include_hidden:       false,
			file_type:            None,
			max_results:          usize::MAX,
			gitignore:            true,
			sort_by_mtime:        false,
			cache:                false,
		}
	}
}

/// Glob failure.
#[derive(Debug)]
pub enum GlobError {
	/// The pattern does not compile.
	InvalidPattern(String),
	/// Traversal failed or the heartbeat interrupted it.
	Walk(WalkError<String>),
}

impl fmt::Display for GlobError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidPattern(message) => write!(f, "Invalid glob pattern: {message}"),
			Self::Walk(WalkError::Interrupted(message)) => f.write_str(message),
			Self::Walk(WalkError::InvalidData { path, message }) => {
				write!(f, "Native directory scan failed for {}: {message}", path.display())
			},
		}
	}
}

impl std::error::Error for GlobError {}

fn mtime(entry: &CollectedEntry) -> f64 {
	entry.mtime.unwrap_or(0.0)
}

fn rank(a: &CollectedEntry, b: &CollectedEntry) -> Ordering {
	mtime(b)
		.total_cmp(&mtime(a))
		.then_with(|| a.path.cmp(&b.path))
}

fn target_type(root: &Path, relative: &str) -> Option<FileType> {
	let metadata = std::fs::metadata(root.join(relative)).ok()?;
	if metadata.is_dir() {
		Some(FileType::Dir)
	} else if metadata.is_file() {
		Some(FileType::File)
	} else {
		None
	}
}

fn effective_type(entry: &CollectedEntry, query: &GlobQuery) -> Option<FileType> {
	let Some(filter) = query.file_type else {
		return Some(entry.file_type);
	};
	if entry.file_type == filter {
		return Some(entry.file_type);
	}
	match (entry.file_type, filter) {
		(FileType::Symlink, FileType::File | FileType::Dir) => {
			target_type(&query.root, &entry.path).filter(|resolved| *resolved == filter)
		},
		_ => None,
	}
}

/// Run a glob search; `heartbeat` is polled during traversal and its error
/// aborts the walk.
pub fn glob<E, H>(query: &GlobQuery, heartbeat: H) -> Result<Vec<CollectedEntry>, GlobError>
where
	H: Fn() -> Result<(), E> + Sync,
	E: fmt::Display,
{
	let pattern = build_glob_pattern(&query.pattern, query.recursive);
	let depth = walk_depth_bound(&pattern);
	let compiled = CompiledWalkGlob::new([pattern])
		.map_err(|error| GlobError::InvalidPattern(error.to_string()))?;
	if query.max_results == 0 {
		return Ok(Vec::new());
	}
	let detail = if query.sort_by_mtime {
		WalkDetail::Full
	} else {
		WalkDetail::Minimal
	};
	let request = WalkRequest::new(query.root.clone())
		.hidden(query.include_hidden)
		.gitignore(query.gitignore)
		.skip_git(true)
		.skip_node_modules(!query.include_node_modules)
		.follow_links(FollowLinks::Never)
		.detail(detail)
		.order(WalkOrder::Path)
		.emit_root(false)
		.depth(1, depth)
		.directory_errors(DirectoryErrorMode::SkipSkippable)
		.cache(query.cache)
		.empty_recheck(EmptyRecheck::Configured)
		.filter(
			WalkFilter::all()
				.glob(compiled)
				.node_modules_unless_mentioned(query.include_node_modules),
		);

	let mut matches = if query.sort_by_mtime && query.file_type.is_none() {
		request
			.collect_ranked_with_heartbeat(WalkRank::MtimeDescPathAsc, query.max_results, &heartbeat)
			.map_err(GlobError::Walk)?
			.entries
	} else {
		let request = if !query.sort_by_mtime && query.file_type.is_none() {
			request.limit(query.max_results)
		} else {
			request
		};
		let entries = request
			.collect_with_heartbeat(&heartbeat)
			.map_err(GlobError::Walk)?
			.entries;
		let mut collected = Vec::new();
		for mut entry in entries {
			heartbeat().map_err(|error| GlobError::Walk(WalkError::Interrupted(error.to_string())))?;
			let Some(kind) = effective_type(&entry, query) else {
				continue;
			};
			entry.file_type = kind;
			collected.push(entry);
			if !query.sort_by_mtime && collected.len() >= query.max_results {
				break;
			}
		}
		collected
	};
	if query.sort_by_mtime {
		matches.sort_by(rank);
		matches.truncate(query.max_results);
	}
	Ok(matches)
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	fn paths(entries: &[CollectedEntry]) -> Vec<&str> {
		entries.iter().map(|entry| entry.path.as_str()).collect()
	}

	fn run(query: &GlobQuery) -> Vec<CollectedEntry> {
		glob(query, || Ok::<(), std::convert::Infallible>(())).expect("glob succeeds")
	}

	#[test]
	fn gitignore_prunes_ignored_directory_but_keeps_matching_sibling() {
		let root = tempfile::tempdir().unwrap();
		fs::create_dir_all(root.path().join(".git")).unwrap();
		fs::write(root.path().join(".gitignore"), "ignored/\n").unwrap();
		fs::create_dir_all(root.path().join("ignored")).unwrap();
		fs::write(root.path().join("ignored/drop.rs"), "fn ignored() {}\n").unwrap();
		fs::write(root.path().join("kept.rs"), "fn kept() {}\n").unwrap();
		let mut query = GlobQuery::new(root.path(), "*.rs");
		query.file_type = Some(FileType::File);
		assert_eq!(paths(&run(&query)), ["kept.rs"]);
	}

	#[test]
	fn depth_bounded_patterns_match_at_exact_depth_in_ranked_mode() {
		let root = tempfile::tempdir().unwrap();
		fs::write(root.path().join("top.txt"), "top").unwrap();
		fs::create_dir_all(root.path().join("deep/nested")).unwrap();
		fs::write(root.path().join("deep/child.txt"), "mid").unwrap();
		fs::write(root.path().join("deep/nested/leaf.txt"), "leaf").unwrap();
		let query = |pattern: &str| {
			let mut query = GlobQuery::new(root.path(), pattern);
			query.recursive = false;
			query.include_hidden = true;
			query.max_results = 100;
			query.sort_by_mtime = true;
			run(&query)
		};
		assert_eq!(paths(&query("*.txt")), ["top.txt"]);
		assert_eq!(paths(&query("deep/*.txt")), ["deep/child.txt"]);
		assert_eq!(paths(&query("*/nested/leaf.txt")), ["deep/nested/leaf.txt"]);
		let all = query("**/*.txt");
		let mut all = paths(&all);
		all.sort_unstable();
		assert_eq!(all, ["deep/child.txt", "deep/nested/leaf.txt", "top.txt"]);
	}

	#[test]
	fn file_type_filter_is_applied_before_the_result_limit() {
		let root = tempfile::tempdir().unwrap();
		fs::create_dir_all(root.path().join("a_dir")).unwrap();
		fs::write(root.path().join("b_file"), "").unwrap();
		let mut query = GlobQuery::new(root.path(), "*");
		query.recursive = false;
		query.max_results = 1;
		query.file_type = Some(FileType::File);
		assert_eq!(paths(&run(&query)), ["b_file"]);
	}

	#[test]
	fn heartbeat_errors_abort_the_walk() {
		let root = tempfile::tempdir().unwrap();
		fs::write(root.path().join("a.txt"), "").unwrap();
		let error =
			glob(&GlobQuery::new(root.path(), "*.txt"), || Err("Aborted: Timeout")).unwrap_err();
		assert!(error.to_string().contains("Aborted: Timeout"), "{error}");
	}

	#[test]
	fn patterns_normalize_and_bound_depth() {
		assert_eq!(build_glob_pattern("*.ts", true), "**/*.ts");
		assert_eq!(build_glob_pattern("src\\*.ts", true), "src/*.ts");
		assert_eq!(build_glob_pattern("*.{ts,js", false), "*.{ts,js}");
		assert_eq!(build_glob_pattern("{a,b}", true), "{a,b}");
		assert_eq!(walk_depth_bound("dir/*.json"), 2);
		assert_eq!(walk_depth_bound("**/x"), usize::MAX);
	}
}
