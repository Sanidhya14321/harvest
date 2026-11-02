//! The `glob` tool: newest-first file discovery over the shared walker engine,
//! with the legacy tool's argument, error and grouped-output contract.
use std::{
	collections::HashSet,
	fmt::Write as _,
	path::{Path, PathBuf},
	time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use harvest_runtime::{JailedFs, cancel::CancelToken};
use pi_walker::{
	FileType,
	glob::{GlobError, GlobQuery},
};
use serde::Deserialize;

const DEFAULT_LIMIT: f64 = 200.0;
const MAX_LIMIT: usize = 200;
const TIMEOUT: Duration = Duration::from_secs(5);
const GLOB_CHARS: [char; 4] = ['*', '?', '[', '{'];

#[derive(Debug, Deserialize)]
pub(crate) struct Args {
	path:      Option<String>,
	hidden:    Option<bool>,
	gitignore: Option<bool>,
	limit:     Option<f64>,
}

/// A user target split into a literal base directory and a glob.
struct Target {
	display:  String,
	base:     PathBuf,
	pattern:  String,
	has_glob: bool,
}

fn is_url_like(text: &str) -> bool {
	let Some((scheme, _)) = text.split_once("://") else {
		return false;
	};
	let mut chars = scheme.chars();
	chars
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic())
		&& chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}

/// Split at the first segment containing glob characters (`parseFindPattern`).
fn parse_target(pattern: &str) -> (String, String, bool) {
	let normalized = pattern.replace('\\', "/");
	let segments: Vec<&str> = normalized.split('/').collect();
	match segments
		.iter()
		.position(|segment| segment.contains(GLOB_CHARS))
	{
		None => (normalized.clone(), "**/*".to_owned(), false),
		Some(0) => {
			let glob = if normalized.starts_with("**/") {
				normalized.clone()
			} else {
				format!("**/{normalized}")
			};
			(".".to_owned(), glob, true)
		},
		Some(index) => (segments[..index].join("/"), segments[index..].join("/"), true),
	}
}

fn display(path: &Path) -> String {
	let text = path.to_string_lossy().replace('\\', "/");
	if text.is_empty() {
		".".to_owned()
	} else {
		text
	}
}

fn targets(fs: &JailedFs, input: Option<&str>) -> Result<Vec<Target>> {
	let entries: Vec<String> = match input {
		Some(input) => input
			.split(';')
			.map(|entry| entry.trim().to_owned())
			.collect(),
		None => vec![".".to_owned()],
	};
	let entries = if entries.len() == 1 && entries[0].is_empty() && input.is_some_and(str::is_empty)
	{
		vec![".".to_owned()]
	} else {
		entries
	};
	let mut targets = Vec::new();
	for entry in entries {
		let normalized = entry.replace('\\', "/");
		if !normalized.is_empty() && normalized.chars().all(|ch| ch == '/') {
			bail!("Searching from root directory '/' is not allowed");
		}
		if normalized.is_empty() {
			bail!("`path` must contain non-empty globs or paths");
		}
		if is_url_like(&normalized) {
			if normalized.to_ascii_lowercase().starts_with("ssh://") {
				bail!(
					"find cannot operate on a remote ssh:// path: {normalized}. ssh:// has no local \
					 file to glob; use `read {normalized}` to list or inspect the remote path."
				);
			}
			bail!("Internal URLs are not supported by this host: {normalized}");
		}
		let (base, pattern, has_glob) = parse_target(&normalized);
		let base = fs.relative(Path::new(&base))?;
		targets.push(Target { display: display(&base), base, pattern, has_glob });
	}
	Ok(targets)
}

fn mtime_ms(path: &Path) -> f64 {
	std::fs::metadata(path)
		.and_then(|metadata| metadata.modified())
		.ok()
		.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
		.map_or(0.0, |duration| duration.as_secs_f64() * 1000.0)
}

fn join_display(base: &Path, relative: &str, is_dir: bool) -> String {
	let joined = display(&base.join(relative));
	if is_dir { format!("{joined}/") } else { joined }
}

/// Render paths as a prefix-folded directory tree (`formatGroupedPaths`).
pub(crate) fn format_grouped(paths: &[String]) -> String {
	#[derive(Default)]
	struct Node {
		files:   Vec<String>,
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
	fn walk(node: &Node, depth: usize, lines: &mut Vec<String>) {
		lines.extend(node.files.iter().cloned());
		for (name, mut dir) in node.subdirs.iter().map(|(name, dir)| (name, dir)) {
			let mut parts = vec![name.as_str()];
			while dir.files.is_empty() && dir.subdirs.len() == 1 {
				parts.push(&dir.subdirs[0].0);
				dir = &dir.subdirs[0].1;
			}
			lines.push(format!("{} {}/", "#".repeat(depth + 1), parts.join("/")));
			walk(dir, depth + 1, lines);
		}
	}
	let mut root = Node::default();
	for raw in paths {
		let normalized = raw.replace('\\', "/");
		if is_url_like(&normalized) {
			if !root.files.contains(&normalized) {
				root.files.push(normalized);
			}
			continue;
		}
		let is_dir = normalized.ends_with('/');
		let trimmed = normalized.strip_suffix('/').unwrap_or(&normalized);
		if trimmed.is_empty() {
			continue;
		}
		let segments: Vec<&str> = trimmed.split('/').collect();
		let dirs = if is_dir {
			segments.len()
		} else {
			segments.len() - 1
		};
		let mut node = &mut root;
		for segment in &segments[..dirs] {
			node = child(node, segment);
		}
		if !is_dir {
			let name = segments[segments.len() - 1].to_owned();
			if !node.files.contains(&name) {
				node.files.push(name);
			}
		}
	}
	let mut lines = Vec::new();
	walk(&root, 0, &mut lines);
	lines.join("\n")
}

pub(crate) fn glob(root: &Path, args: &Args, cancellation: &CancelToken) -> Result<String> {
	let deadline = Instant::now() + TIMEOUT;
	search(root, args, &|| {
		cancellation.heartbeat()?;
		if Instant::now() > deadline {
			Err(anyhow!("Aborted: Timeout"))
		} else {
			Ok(())
		}
	})
}

fn search(root: &Path, args: &Args, heartbeat: &(dyn Fn() -> Result<()> + Sync)) -> Result<String> {
	heartbeat()?;
	let fs = JailedFs::open(root)?;
	let targets = targets(&fs, args.path.as_deref())?;
	let requested = args.limit.unwrap_or(DEFAULT_LIMIT);
	if !requested.is_finite() || requested <= 0.0 {
		bail!("Limit must be a positive number");
	}
	let limit = (requested.floor() as usize).clamp(1, MAX_LIMIT);
	let single = targets.len() == 1;

	// Multi-path calls skip missing entries and fail only when all are missing.
	let mut missing = Vec::new();
	let mut present = Vec::new();
	for target in targets {
		match fs.file_type(&target.base) {
			Ok(kind) => present.push((target, kind)),
			Err(_) if single => bail!("Path not found: {}", target.display),
			Err(_) => missing.push(target.display),
		}
	}
	if present.is_empty() {
		bail!("Path not found: {}", missing.join(", "));
	}

	let mut found: Vec<(String, f64)> = Vec::new();
	let mut timed_out = false;
	for (target, kind) in &present {
		let absolute = fs.root().join(&target.base);
		if !target.has_glob && kind.is_file() {
			found.push((target.display.clone(), mtime_ms(&absolute)));
			continue;
		}
		if !kind.is_dir() {
			if single {
				bail!("Path is not a directory: {}", target.display);
			}
			continue;
		}
		let mut query = GlobQuery::new(&absolute, &target.pattern);
		query.include_hidden = args.hidden.unwrap_or(true);
		query.gitignore = args.gitignore.unwrap_or(true);
		query.max_results = limit;
		query.sort_by_mtime = true;
		// Patterns arrive with an explicit `**/` when recursion is wanted.
		query.recursive = false;
		match pi_walker::glob::glob(&query, heartbeat) {
			Ok(entries) => found.extend(entries.into_iter().map(|entry| {
				(
					join_display(&target.base, &entry.path, entry.file_type == FileType::Dir),
					entry.mtime.unwrap_or(0.0),
				)
			})),
			Err(GlobError::Walk(pi_walker::WalkError::Interrupted(message)))
				if message.contains("Aborted: Timeout") =>
			{
				heartbeat().or_else(|error| {
					if error.to_string().contains("Aborted: Timeout") {
						Ok(())
					} else {
						Err(error)
					}
				})?;
				timed_out = true;
			},
			Err(error) => return Err(error.into()),
		}
	}

	let mut seen = HashSet::new();
	found.retain(|(path, _)| seen.insert(path.clone()));
	found.sort_by(|a, b| b.1.total_cmp(&a.1));
	let files: Vec<String> = found.into_iter().map(|(path, _)| path).collect();

	let mut notes = Vec::new();
	if timed_out {
		let seconds = TIMEOUT.as_secs();
		notes.push(if files.is_empty() {
			format!(
				"Glob timed out after {seconds}s before finding any matches — the scan is incomplete, \
				 NOT proof of absence. The walk is bounded by directory size, not pattern width; \
				 scope the search to a deeper directory (e.g. `sub/dir/*.ext` instead of `*.ext` at a \
				 huge root)."
			)
		} else {
			format!(
				"glob timed out after {seconds}s; returning {} partial matches — results are \
				 incomplete, scope to a deeper directory instead of retrying blindly",
				files.len()
			)
		});
	}
	if !missing.is_empty() {
		notes.push(format!("Skipped missing paths: {}", missing.join(", ")));
	}
	if files.is_empty() {
		let mut parts = if timed_out {
			Vec::new()
		} else {
			vec!["No files found matching pattern".to_owned()]
		};
		parts.extend(notes);
		return Ok(parts.join("\n"));
	}
	let reached = files.len() >= limit;
	let shown = &files[..files.len().min(limit)];
	let mut output = format_grouped(shown);
	if !notes.is_empty() {
		output = format!("{output}\n\n{}", notes.join("\n"));
	}
	if reached {
		let _ = write!(
			output,
			"

[{limit} results limit reached. Use limit={} for more]",
			limit * 2
		);
	}
	Ok(output)
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	fn args(path: Option<&str>) -> Args {
		Args { path: path.map(str::to_owned), hidden: None, gitignore: None, limit: None }
	}

	fn run(root: &Path, args: &Args) -> Result<String> {
		search(root, args, &|| Ok(()))
	}

	fn touch(root: &Path, path: &str, age_secs: u64) {
		let path = root.join(path);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(&path, "").unwrap();
		let time = std::time::SystemTime::now() - Duration::from_secs(age_secs);
		fs::File::options()
			.write(true)
			.open(&path)
			.unwrap()
			.set_modified(time)
			.unwrap();
	}

	#[test]
	fn grouped_tree_folds_single_child_chains_and_lists_files_first() {
		let paths = ["a/b/c/x.ts", "a/b/c/y.ts", "a/z.ts", "top.md", "d/"].map(String::from);
		assert_eq!(format_grouped(&paths), "top.md\n# a/\nz.ts\n## b/c/\nx.ts\ny.ts\n# d/");
	}

	#[test]
	fn results_are_newest_first_scoped_and_dirs_suffixed() -> Result<()> {
		let root = tempfile::tempdir()?;
		touch(root.path(), "src/old.ts", 300);
		touch(root.path(), "src/new.ts", 10);
		touch(root.path(), "src/deep/mid.ts", 100);
		touch(root.path(), "notes.md", 5);
		fs::create_dir_all(root.path().join("src/tests"))?;
		assert_eq!(run(root.path(), &args(Some("src/*.ts")))?, "# src/\nnew.ts\nold.ts");
		assert_eq!(
			run(root.path(), &args(Some("src/**/*.ts")))?,
			"# src/\nnew.ts\nold.ts\n## deep/\nmid.ts"
		);
		assert!(run(root.path(), &args(Some("**/tests")))?.contains("# src/tests/"));
		assert_eq!(run(root.path(), &args(Some("notes.md")))?, "notes.md");
		Ok(())
	}

	#[test]
	fn limit_notice_and_missing_paths_follow_the_legacy_wording() -> Result<()> {
		let root = tempfile::tempdir()?;
		touch(root.path(), "a.txt", 2);
		touch(root.path(), "b.txt", 1);
		let mut limited = args(Some("*.txt"));
		limited.limit = Some(1.0);
		assert_eq!(
			run(root.path(), &limited)?,
			"b.txt\n\n[1 results limit reached. Use limit=2 for more]"
		);
		assert_eq!(
			run(root.path(), &args(Some("*.txt; gone/*.txt")))?,
			"b.txt\na.txt\n\nSkipped missing paths: gone"
		);
		assert_eq!(run(root.path(), &args(Some("*.none")))?, "No files found matching pattern");
		Ok(())
	}

	#[test]
	fn invalid_targets_fail_with_actionable_errors() {
		let root = tempfile::tempdir().unwrap();
		let error = |path: &str| run(root.path(), &args(Some(path))).unwrap_err().to_string();
		assert_eq!(error("/"), "Searching from root directory '/' is not allowed");
		assert_eq!(error("missing/*.ts"), "Path not found: missing");
		assert!(error("../outside/*.ts").contains("escapes the authorized root"));
		assert!(error("ssh://host/x").starts_with("find cannot operate on a remote ssh:// path"));
		let mut zero = args(None);
		zero.limit = Some(0.0);
		assert_eq!(
			run(root.path(), &zero).unwrap_err().to_string(),
			"Limit must be a positive number"
		);
	}

	#[test]
	fn gitignore_and_hidden_flags_control_visibility() -> Result<()> {
		let root = tempfile::tempdir()?;
		fs::create_dir_all(root.path().join(".git"))?;
		fs::write(root.path().join(".gitignore"), ".env\n")?;
		touch(root.path(), ".env", 1);
		assert_eq!(run(root.path(), &args(Some(".env*")))?, "No files found matching pattern");
		let mut ignored = args(Some(".env*"));
		ignored.gitignore = Some(false);
		assert_eq!(run(root.path(), &ignored)?, ".env");
		ignored.hidden = Some(false);
		assert_eq!(run(root.path(), &ignored)?, "No files found matching pattern");
		Ok(())
	}

	#[test]
	fn timeout_reports_an_incomplete_scan_instead_of_absence() -> Result<()> {
		let root = tempfile::tempdir()?;
		touch(root.path(), "a.txt", 1);
		let output = search(root.path(), &args(Some("*.txt")), &|| Err(anyhow!("Aborted: Timeout")));
		// The first heartbeat runs before the walk; a deadline already passed
		// surfaces as the timeout error itself.
		assert!(output.is_err());
		let calls = std::sync::atomic::AtomicUsize::new(0);
		let output = search(root.path(), &args(Some("*.txt")), &|| {
			if calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
				Ok(())
			} else {
				Err(anyhow!("Aborted: Timeout"))
			}
		})?;
		assert!(output.starts_with("Glob timed out after 5s before finding any matches"), "{output}");
		Ok(())
	}
}
