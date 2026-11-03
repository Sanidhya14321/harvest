//! Native, local-only staging of a release formula in a checked-out tap.
use std::{
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;

const FORMULA: &str = "Formula/omp.rb";
const LIMIT: usize = 1024 * 1024;

#[derive(Parser)]
#[command(name = "cargo xtask tap-update")]
struct Arguments {
	#[arg(long)]
	formula: PathBuf,
	#[arg(long)]
	tap:     PathBuf,
	#[arg(long)]
	tag:     String,
}

fn git(root: &Path, hooks: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
	let mut command = Command::new("git");
	// A caller's ambient Git worktree/index overrides must not redirect a
	// mutation away from the explicitly named tap checkout.
	for variable in [
		"GIT_DIR",
		"GIT_WORK_TREE",
		"GIT_COMMON_DIR",
		"GIT_INDEX_FILE",
		"GIT_OBJECT_DIRECTORY",
		"GIT_ALTERNATE_OBJECT_DIRECTORIES",
	] {
		command.env_remove(variable);
	}
	command
		.current_dir(root)
		.env("GIT_TERMINAL_PROMPT", "0")
		.args([
			"-c",
			"commit.gpgsign=false",
			"-c",
			"user.name=pukbot[bot]",
			"-c",
			"user.email=320458784+pukbot[bot]@users.noreply.github.com",
			"-c",
		])
		.arg(format!("core.hooksPath={}", hooks.display()))
		.args(arguments);
	crate::process::capture(command, LIMIT)
		.with_context(|| format!("tap Git operation failed: {}", arguments[0]))
}

pub fn update(formula: &Path, tap: &Path, tag: &str) -> Result<bool> {
	if tag.is_empty() || tag.len() > 256 || tag.chars().any(char::is_control) {
		bail!("tag must contain 1..256 bytes without control characters");
	}
	let formula = std::path::absolute(formula)?;
	let source = JailedFs::open(formula.parent().context("formula has no parent")?)?;
	let contents =
		source.read(Path::new(formula.file_name().context("formula has no filename")?), LIMIT)?;
	let fs = JailedFs::open(tap)?;
	let hooks = tempfile::tempdir()?;
	let top = git(fs.root(), hooks.path(), &["rev-parse", "--show-toplevel"])?;
	let top = std::str::from_utf8(&top)?.trim_end_matches(['\r', '\n']);
	if Path::new(top).canonicalize()? != fs.root() {
		bail!("tap must be the repository root");
	}
	// A linked worktree stores its Git metadata outside the worktree. Git
	// resolves that existing metadata owner; jail the lock to that directory.
	let git_dir = git(fs.root(), hooks.path(), &["rev-parse", "--absolute-git-dir"])?;
	let git_dir = std::str::from_utf8(&git_dir)?.trim_end_matches(['\r', '\n']);
	let metadata = JailedFs::open(Path::new(git_dir))?;
	let _owner = metadata.lock(Path::new("harvest-tap.lock"))?;
	if !git(fs.root(), hooks.path(), &["diff", "--cached", "--name-only", "-z"])?.is_empty() {
		bail!("tap has staged changes; inspect them before retrying");
	}
	if !git(fs.root(), hooks.path(), &[
		"status",
		"--porcelain",
		"--untracked-files=all",
		"--",
		FORMULA,
	])?
	.is_empty()
	{
		bail!("tap formula has uncommitted changes; inspect them before retrying");
	}
	// Read through the jail even on the no-op path; symlinks must not bypass
	// validation merely because their target currently has identical bytes.
	if fs.read_optional(Path::new(FORMULA), LIMIT)?.as_deref() == Some(&contents) {
		println!("the tap already carries {tag}");
		return Ok(false);
	}
	fs.atomic_write(Path::new(FORMULA), &contents)?;
	git(fs.root(), hooks.path(), &["add", "--", FORMULA])?;
	// Git attributes/autocrlf may normalize different worktree bytes to the
	// existing blob. Preserve the old helper's successful staged-diff no-op.
	if git(fs.root(), hooks.path(), &["diff", "--cached", "--name-only", "-z", "--", FORMULA])?
		.is_empty()
	{
		println!("the tap already carries {tag}");
		return Ok(false);
	}
	git(fs.root(), hooks.path(), &[
		"commit",
		"--quiet",
		"--only",
		"-m",
		&format!("Update the Omp formula to {tag}"),
		"--",
		FORMULA,
	])?;
	println!("updated the tap for {tag}");
	Ok(true)
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::try_parse_from(std::iter::once("tap-update".to_owned()).chain(arguments))?;
	update(&args.formula, &args.tap, &args.tag)?;
	Ok(())
}
