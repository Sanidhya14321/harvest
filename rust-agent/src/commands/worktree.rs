//! `harvest worktree` — list and clean up agent-managed git worktrees
//! (port of `packages/coding-agent/src/cli/worktree-cli.ts`).

use std::{
	fs,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use harvest_runtime::JailedFs;
use serde::Serialize;

use super::paths::{self, Paint};

const ISOLATION_OWNER_FILE: &str = ".harvest-isolation-owner.json";
const LEGACY_ISOLATION_OWNER_FILE: &str = ".omp-isolation-owner.json";
const TASK_ISOLATION_MOUNT_DIRS: [&str; 2] = ["m", "merged"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WorktreeAction {
	List,
	Clear,
	Add,
}

#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  harvest worktree\n  harvest worktree list --json\n  harvest \
                        worktree add ../feature\n  harvest worktree add -b feature ../feature \
                        origin/main\n  harvest worktree add --detach ../review HEAD~2\n  harvest \
                        worktree clear\n  harvest worktree clear --dry-run\n  harvest worktree \
                        clear --all")]
pub struct WorktreeArgs {
	/// list (default), clear, or add
	#[arg(value_enum, default_value = "list")]
	action:       WorktreeAction,
	/// Worktree path (add)
	path:         Option<String>,
	/// Commit-ish to check out (add)
	commit:       Option<String>,
	/// Repository/base directory (add)
	#[arg(short = 'C', long)]
	cwd:          Option<String>,
	/// Create a new branch (add)
	#[arg(short = 'b', long, conflicts_with = "force_branch")]
	branch:       Option<String>,
	/// Create or reset a branch (add)
	#[arg(short = 'B', long = "force-branch")]
	force_branch: Option<String>,
	/// Detach HEAD (add)
	#[arg(short = 'd', long)]
	detach:       bool,
	/// Suppress add output
	#[arg(short = 'q', long)]
	quiet:        bool,
	/// Clear every entry, including live PR-checkout worktrees (clear)
	#[arg(long)]
	all:          bool,
	/// Print what would be removed without touching the filesystem (clear)
	#[arg(short = 'n', long = "dry-run")]
	dry_run:      bool,
	/// Emit machine-readable JSON
	#[arg(short = 'j', long)]
	json:         bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum WorktreeKind {
	PrCheckout,
	TaskIsolation,
	Empty,
	Stray,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeEntry {
	path:          String,
	kind:          WorktreeKind,
	#[serde(skip_serializing_if = "Option::is_none")]
	parent_repo:   Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	branch:        Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	orphan_reason: Option<String>,
}

fn settings_for(cwd: &Path) -> Option<crate::settings::Settings> {
	let agent_dir = crate::config::agent_dir_from(None, |key| std::env::var_os(key));
	crate::settings::Settings::load(agent_dir.as_deref(), cwd, &[]).ok()
}

pub fn execute(args: WorktreeArgs) -> Result<u8> {
	if args.action == WorktreeAction::Add {
		let Some(path) = args.path.clone() else {
			eprintln!("Error: Missing required argument: path");
			return Ok(1);
		};
		return match add_worktree(&args, &path) {
			Ok(()) => Ok(0),
			Err(error) => {
				eprintln!("{error:#}");
				Ok(1)
			},
		};
	}
	let cwd = std::env::current_dir()?;
	let base = settings_for(&cwd).and_then(|settings| settings.string("worktree.base"));
	let root = paths::worktrees_dir(base.as_deref());
	if args.action == WorktreeAction::Clear {
		return clear_worktrees(&root, &args);
	}
	list_worktrees(&root, args.json)
}

// ───────────────────────────── add ─────────────────────────────

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
	let output = Command::new("git")
		.args(args)
		.current_dir(cwd)
		.env("GIT_TERMINAL_PROMPT", "0")
		.output()
		.context("failed to run git")?;
	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		bail!("{}", stderr.trim());
	}
	Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn ref_exists(repo: &Path, name: &str) -> bool {
	git(repo, &["show-ref", "--verify", "--quiet", name]).is_ok()
}

fn add_worktree(args: &WorktreeArgs, path_arg: &str) -> Result<()> {
	if args.branch.is_some() && args.force_branch.is_some() {
		bail!("fatal: options '-b' and '-B' cannot be used together");
	}
	let base = std::env::current_dir()?;
	let cwd = args.cwd.as_ref().map_or(base.clone(), |dir| base.join(dir));
	let repo_root = git(&cwd, &["rev-parse", "--show-toplevel"])
		.map_err(|_| anyhow::anyhow!("fatal: not a git repository: {}", cwd.display()))?;
	let repo_root = PathBuf::from(repo_root.trim());
	let worktree_path = cwd.join(path_arg);
	match fs::metadata(&worktree_path) {
		Ok(meta) => {
			let non_empty = !meta.is_dir() || fs::read_dir(&worktree_path)?.next().is_some();
			if non_empty {
				bail!("fatal: '{path_arg}' already exists");
			}
		},
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
		Err(error) => return Err(error.into()),
	}

	let (reference, detach, created_branch) =
		if let Some(branch) = args.branch.as_ref().or(args.force_branch.as_ref()) {
			let start = args.commit.as_deref().unwrap_or("HEAD");
			let mut command = vec!["branch"];
			if args.force_branch.is_some() {
				command.push("--force");
			}
			command.extend([branch.as_str(), start]);
			git(&cwd, &command)?;
			(branch.clone(), false, Some(branch.clone()))
		} else if args.detach {
			(args.commit.clone().unwrap_or_else(|| "HEAD".to_owned()), true, None)
		} else if let Some(commit) = &args.commit {
			(commit.clone(), !ref_exists(&cwd, &format!("refs/heads/{commit}")), None)
		} else {
			let branch = worktree_path
				.file_name()
				.map(|name| name.to_string_lossy().into_owned())
				.context("worktree path has no name")?;
			let mut created = None;
			if !ref_exists(&cwd, &format!("refs/heads/{branch}")) {
				git(&cwd, &["branch", &branch, "HEAD"])?;
				created = Some(branch.clone());
			}
			(branch, false, created)
		};

	let details = git(&cwd, &["log", "-1", "--format=%H%n%s", &reference, "--"])?;
	let mut lines = details.lines();
	let sha = lines.next().unwrap_or_default().to_owned();
	let subject = lines.next().unwrap_or_default().to_owned();
	let short_sha = &sha[..sha.len().min(7)];
	if !args.quiet {
		let preparation = if let Some(branch) = &created_branch {
			format!("new branch '{branch}'")
		} else if detach {
			format!("detached HEAD {short_sha}")
		} else {
			format!("checking out '{reference}'")
		};
		println!("Preparing worktree ({preparation})");
	}
	let target = worktree_path.to_string_lossy().into_owned();
	let mut command = vec!["worktree", "add", "--quiet"];
	if detach {
		command.push("--detach");
	}
	command.extend([target.as_str(), reference.as_str()]);
	git(&repo_root, &command)?;
	if !args.quiet {
		println!("HEAD is now at {short_sha} {subject}");
	}
	Ok(())
}

// ───────────────────────────── list / clear ─────────────────────────────

fn list_worktrees(root: &Path, json: bool) -> Result<u8> {
	let entries = scan_worktrees(root)?;
	if json {
		println!("{}", serde_json::to_string_pretty(&entries)?);
		return Ok(0);
	}
	let paint = Paint::new();
	if entries.is_empty() {
		println!(
			"{}",
			paint.dim(&format!("No agent-managed worktrees found under {}.", root.display()))
		);
		return Ok(0);
	}
	let (mut live, mut orphaned) = (0, 0);
	for entry in &entries {
		let tag = if entry.orphan_reason.is_some() {
			paint.yellow("orphaned")
		} else {
			paint.green("live    ")
		};
		println!("{tag}  {}", entry.path);
		let detail = format_entry_detail(entry);
		if !detail.is_empty() {
			println!("          {}", paint.dim(&detail));
		}
		if entry.orphan_reason.is_some() {
			orphaned += 1;
		} else {
			live += 1;
		}
	}
	println!(
		"{}",
		paint.dim(&format!("\n{live} live · {orphaned} orphaned · {} total", entries.len()))
	);
	Ok(0)
}

#[derive(Serialize)]
struct ClearResult {
	path:  String,
	ok:    bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	error: Option<String>,
}

fn remove_tree(jail: &JailedFs, target: &Path) -> Result<()> {
	// Jail check before touching the filesystem; reuse the resolved path.
	let resolved = jail.resource_path(target)?;
	match fs::symlink_metadata(&resolved) {
		Ok(meta) if meta.is_dir() => fs::remove_dir_all(&resolved)?,
		Ok(_) => fs::remove_file(&resolved)?,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
		Err(error) => return Err(error.into()),
	}
	Ok(())
}

fn clear_worktrees(root: &Path, args: &WorktreeArgs) -> Result<u8> {
	let paint = Paint::new();
	let entries = scan_worktrees(root)?;
	let targets: Vec<&WorktreeEntry> = entries
		.iter()
		.filter(|entry| args.all || entry.orphan_reason.is_some())
		.collect();

	if targets.is_empty() {
		if args.json {
			println!("{}", serde_json::json!({ "removed": 0, "kept": entries.len() }));
		} else {
			let text = if args.all {
				"No worktrees to remove."
			} else {
				"No orphaned worktrees to remove."
			};
			println!("{}", paint.dim(text));
		}
		return Ok(0);
	}

	if args.dry_run {
		if args.json {
			let paths: Vec<&str> = targets.iter().map(|target| target.path.as_str()).collect();
			println!(
				"{}",
				serde_json::to_string_pretty(&serde_json::json!({ "wouldRemove": paths }))?
			);
		} else {
			for target in &targets {
				println!("{}  {}", paint.yellow("would remove"), target.path);
			}
			let plural = if targets.len() == 1 { "" } else { "s" };
			println!("{}", paint.dim(&format!("\n{} dir{plural} would be removed.", targets.len())));
		}
		return Ok(0);
	}

	let jail = JailedFs::open(root)?;
	let mut results = Vec::new();
	let mut parents_to_prune: Vec<String> = Vec::new();
	for target in targets {
		let outcome = (|| -> Result<()> {
			let path = Path::new(&target.path);
			if target.kind == WorktreeKind::PrCheckout
				&& target.orphan_reason.is_none()
				&& let Some(parent) = &target.parent_repo
			{
				jail.resource_path(path)?;
				let removed =
					git(Path::new(parent), &["worktree", "remove", "--force", &target.path]).is_ok();
				if !removed {
					remove_tree(&jail, path)?;
					parents_to_prune.push(parent.clone());
				}
			} else {
				remove_tree(&jail, path)?;
				if let Some(parent) = &target.parent_repo {
					parents_to_prune.push(parent.clone());
				}
			}
			Ok(())
		})();
		results.push(match outcome {
			Ok(()) => ClearResult { path: target.path.clone(), ok: true, error: None },
			Err(error) => ClearResult {
				path:  target.path.clone(),
				ok:    false,
				error: Some(format!("{error:#}")),
			},
		});
	}
	parents_to_prune.sort();
	parents_to_prune.dedup();
	for parent in parents_to_prune {
		// Best effort: the parent repo may already be gone.
		let _ = git(Path::new(&parent), &["worktree", "prune"]);
	}

	let succeeded = results.iter().filter(|result| result.ok).count();
	let failed = results.len() - succeeded;
	if args.json {
		println!(
			"{}",
			serde_json::to_string_pretty(
				&serde_json::json!({ "removed": succeeded, "failed": failed, "results": results })
			)?
		);
		return Ok(u8::from(failed > 0));
	}
	for result in &results {
		if result.ok {
			println!("{}  {}", paint.green("removed"), result.path);
		} else {
			println!("{}  {}", paint.red("failed "), result.path);
			if let Some(error) = &result.error {
				println!("          {}", paint.dim(error));
			}
		}
	}
	let failures = if failed > 0 {
		format!(" · {}", paint.red(&format!("{failed} failed")))
	} else {
		String::new()
	};
	println!("{}", paint.dim(&format!("\n{succeeded} removed{failures}")));
	Ok(u8::from(failed > 0))
}

// ───────────────────────────── scanner ─────────────────────────────

fn is_dir(path: &Path) -> bool {
	fs::metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn sorted_names(dir: &Path) -> std::io::Result<Vec<String>> {
	let mut names: Vec<String> = fs::read_dir(dir)?
		.filter_map(|entry| entry.ok())
		.map(|entry| entry.file_name().to_string_lossy().into_owned())
		.collect();
	names.sort();
	Ok(names)
}

fn scan_worktrees(root: &Path) -> Result<Vec<WorktreeEntry>> {
	let top_level = match sorted_names(root) {
		Ok(names) => names,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(error) => return Err(error.into()),
	};
	let mut entries = Vec::new();
	for name in top_level {
		let dir = root.join(&name);
		if !is_dir(&dir) {
			continue;
		}
		if let Some(direct) = classify_dir(&dir) {
			entries.push(direct);
			continue;
		}
		// Legacy nesting: <wt>/<encoded-project>/<branch-or-id>
		let Ok(children) = sorted_names(&dir) else {
			continue;
		};
		let mut nested = 0;
		for child in &children {
			let child_dir = dir.join(child);
			if !is_dir(&child_dir) {
				continue;
			}
			if let Some(classified) = classify_dir(&child_dir) {
				entries.push(classified);
				nested += 1;
			}
		}
		if nested == 0 {
			let empty = children.is_empty();
			entries.push(WorktreeEntry {
				path:          dir.to_string_lossy().into_owned(),
				kind:          if empty {
					WorktreeKind::Empty
				} else {
					WorktreeKind::Stray
				},
				parent_repo:   None,
				branch:        None,
				orphan_reason: Some(
					if empty {
						"empty directory"
					} else {
						"no recognizable worktree contents"
					}
					.to_owned(),
				),
			});
		}
	}
	Ok(entries)
}

fn classify_dir(dir: &Path) -> Option<WorktreeEntry> {
	let git_entry = dir.join(".git");
	if fs::metadata(&git_entry).is_ok_and(|meta| meta.is_file()) {
		return Some(classify_pr_checkout(dir, &git_entry));
	}
	let is_isolation = dir.join(ISOLATION_OWNER_FILE).is_file()
		|| dir.join(LEGACY_ISOLATION_OWNER_FILE).is_file()
		|| TASK_ISOLATION_MOUNT_DIRS
			.iter()
			.any(|mount| is_dir(&dir.join(mount)));
	if !is_isolation {
		return None;
	}
	let live = has_live_isolation_owner(dir);
	Some(WorktreeEntry {
		path:          dir.to_string_lossy().into_owned(),
		kind:          WorktreeKind::TaskIsolation,
		parent_repo:   None,
		branch:        None,
		orphan_reason: (!live).then(|| "task-isolation leftover (no live task owns it)".to_owned()),
	})
}

fn process_start_token(pid: i64) -> Option<String> {
	if cfg!(target_os = "linux") {
		let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
		let comm_end = stat.rfind(')')?;
		let token = stat.get(comm_end + 2..)?.split(' ').nth(19)?;
		return (!token.is_empty()).then(|| token.to_owned());
	}
	let output = Command::new("ps")
		.args(["-o", "lstart=", "-p", &pid.to_string()])
		.output()
		.ok()?;
	if !output.status.success() {
		return None;
	}
	let started = String::from_utf8_lossy(&output.stdout).trim().to_owned();
	(!started.is_empty()).then_some(started)
}

fn has_live_isolation_owner(dir: &Path) -> bool {
	let decoded: Option<serde_json::Value> = [ISOLATION_OWNER_FILE, LEGACY_ISOLATION_OWNER_FILE]
		.iter()
		.find_map(|file| {
			fs::read_to_string(dir.join(file))
				.ok()
				.and_then(|text| serde_json::from_str(&text).ok())
		});
	let Some(decoded) = decoded else { return false };
	let Some(pid) = decoded
		.get("pid")
		.and_then(serde_json::Value::as_i64)
		.filter(|pid| *pid > 0)
	else {
		return false;
	};
	if paths::probe_pid(pid) == Err(libc::ESRCH) {
		return false;
	}
	if let Some(token) = decoded
		.get("startToken")
		.and_then(|value| value.as_str())
		.filter(|t| !t.is_empty())
		&& let Some(current) = process_start_token(pid)
		&& current != token
	{
		return false;
	}
	true
}

fn classify_pr_checkout(dir: &Path, git_entry: &Path) -> WorktreeEntry {
	let mut entry = WorktreeEntry {
		path:          dir.to_string_lossy().into_owned(),
		kind:          WorktreeKind::PrCheckout,
		parent_repo:   None,
		branch:        None,
		orphan_reason: None,
	};
	let contents = match fs::read_to_string(git_entry) {
		Ok(contents) => contents,
		Err(error) => {
			entry.orphan_reason = Some(format!("cannot read .git file: {error}"));
			return entry;
		},
	};
	let parent_git_dir = contents
		.lines()
		.find_map(|line| line.strip_prefix("gitdir:"))
		.map(str::trim)
		.filter(|value| !value.is_empty());
	let Some(parent_git_dir) = parent_git_dir.map(PathBuf::from) else {
		entry.orphan_reason = Some("malformed .git file (no gitdir line)".to_owned());
		return entry;
	};
	// <parent-repo>/.git/worktrees/<name> → <parent-repo>
	let parent_repo = parent_git_dir
		.parent()
		.and_then(Path::parent)
		.and_then(Path::parent)
		.map(Path::to_path_buf)
		.unwrap_or_default();
	entry.parent_repo = Some(parent_repo.to_string_lossy().into_owned());
	entry.branch = fs::read_to_string(parent_git_dir.join("HEAD"))
		.ok()
		.and_then(|head| {
			head
				.trim()
				.strip_prefix("ref:")
				.map(str::trim)
				.and_then(|r| r.strip_prefix("refs/heads/"))
				.map(str::to_owned)
		});
	if !is_dir(&parent_git_dir) {
		entry.orphan_reason = Some("parent repo no longer tracks this worktree".to_owned());
	} else if !is_dir(&parent_repo) {
		entry.orphan_reason = Some("parent repo missing".to_owned());
	}
	entry
}

fn format_entry_detail(entry: &WorktreeEntry) -> String {
	let mut parts = vec![match entry.kind {
		WorktreeKind::PrCheckout => {
			let repo = entry
				.parent_repo
				.as_deref()
				.and_then(|repo| Path::new(repo).file_name())
				.map_or("unknown repo".to_owned(), |name| name.to_string_lossy().into_owned());
			format!("{repo} · {}", entry.branch.as_deref().unwrap_or("unknown branch"))
		},
		WorktreeKind::TaskIsolation => "task-isolation sandbox".to_owned(),
		WorktreeKind::Empty => "legacy project shell".to_owned(),
		WorktreeKind::Stray => "unrecognized contents".to_owned(),
	}];
	if let Some(reason) = &entry.orphan_reason {
		parts.push(reason.clone());
	}
	parts.join(" — ")
}
