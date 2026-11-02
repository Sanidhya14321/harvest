//! Merge a local PR branch with the repository merge-commit schema:
//! `Merge PR #<number>: <conventional PR subject> (@<author>)`.
//!
//! The PR number and author come from `gh`. The subject is the first (oldest)
//! compliant commit subject in `HEAD..<branch>`; with none, the merge is
//! refused rather than writing a noncompliant message.
use std::{
	process::{Command, Stdio},
	sync::LazyLock,
};

use anyhow::{Result, bail};
use clap::Parser;
use regex::Regex;
use serde::Deserialize;

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
static CONVENTIONAL_PREFIX: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^[a-z]+(\([^)]+\))?!?: (.+)$").expect("valid regex"));
static PR_SEGMENT: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"(?i)(?:^|[/_-])pr[/_-]?(\d+)(?:$|[/_-])").expect("valid regex"));
static LEADING_NUMBER: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^(\d+)[/_-]").expect("valid regex"));

#[derive(Parser)]
#[command(name = "cargo xtask merge-pr")]
/// Merge a PR branch with a schema-compliant merge commit message.
struct Arguments {
	branch:  String,
	/// Explicit PR number instead of resolving it from the branch.
	#[arg(long)]
	pr:      Option<u64>,
	/// Print the message without merging.
	#[arg(long, short = 'n')]
	dry_run: bool,
}

#[derive(Deserialize)]
struct PrMeta {
	number: u64,
	author: Author,
}

#[derive(Deserialize)]
struct Author {
	login: String,
}

/// Mechanically checkable subject policy: conventional `type(scope)!:` prefix,
/// at most 72 UTF-16 units, description not starting uppercase, no trailing
/// period.
pub fn is_compliant_subject(subject: &str) -> bool {
	let Some(captures) = CONVENTIONAL_PREFIX.captures(subject) else {
		return false;
	};
	let description = &captures[2];
	subject.encode_utf16().count() <= 72
		&& !description.starts_with(|ch: char| ch.is_ascii_uppercase())
		&& !description.ends_with('.')
}

/// Infer a PR number from names like `pr/6386`, `pr-6386` or `6386-fix-foo`.
pub fn pr_number_from_branch(branch: &str) -> Option<u64> {
	PR_SEGMENT
		.captures(branch)
		.or_else(|| LEADING_NUMBER.captures(branch))
		.and_then(|captures| captures[1].parse().ok())
}

/// Compose the merge message from the oldest compliant subject.
pub fn merge_message(number: u64, login: &str, subjects: &[&str]) -> Option<String> {
	let subject = subjects
		.iter()
		.find(|subject| is_compliant_subject(subject))?;
	Some(format!("Merge PR #{number}: {subject} (@{login})"))
}

fn succeeds(program: &str, args: &[&str]) -> bool {
	Command::new(program)
		.args(args)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.is_ok_and(|status| status.success())
}

fn pr_view(selector: &str) -> Option<PrMeta> {
	let mut command = Command::new("gh");
	command.args(["pr", "view", selector, "--json", "number,title,author"]);
	let bytes = crate::process::capture(command, OUTPUT_LIMIT).ok()?;
	serde_json::from_slice(&bytes).ok()
}

fn resolve_pr(branch: &str, explicit: Option<u64>) -> Result<PrMeta> {
	if let Some(number) = explicit {
		return pr_view(&number.to_string())
			.ok_or_else(|| anyhow::anyhow!("gh could not find PR #{number}"));
	}
	// gh resolves a branch name when it is a PR head ref.
	if let Some(meta) = pr_view(branch) {
		return Ok(meta);
	}
	if let Some(meta) = pr_number_from_branch(branch).and_then(|number| pr_view(&number.to_string()))
	{
		return Ok(meta);
	}
	bail!("could not resolve a PR for branch '{branch}'; pass --pr <number>")
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("merge-pr", arguments)? else {
		return Ok(());
	};
	let branch = args.branch.as_str();
	// Prefer the local ref; fall back to the remote-tracking ref.
	let merge_ref = if succeeds("git", &["rev-parse", "--verify", "--quiet", branch]) {
		branch.to_owned()
	} else {
		let remote = format!("origin/{branch}");
		if !succeeds("git", &["rev-parse", "--verify", "--quiet", &remote]) {
			bail!("'{branch}' is neither a local nor an origin/ ref");
		}
		eprintln!("note: '{branch}' is not local; merging '{remote}'");
		remote
	};

	let meta = resolve_pr(branch, args.pr)?;
	let range = format!("HEAD..{merge_ref}");
	let mut log = Command::new("git");
	log.args(["log", "--reverse", "--format=%s", &range]);
	let log = crate::process::capture(log, OUTPUT_LIMIT)
		.map_err(|_| anyhow::anyhow!("git log {range} failed"))?;
	let log = String::from_utf8_lossy(&log);
	let subjects: Vec<&str> = log
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty())
		.collect();
	if subjects.is_empty() {
		bail!("'{branch}' has no commits ahead of HEAD");
	}
	let Some(message) = merge_message(meta.number, &meta.author.login, &subjects) else {
		bail!(
			"no compliant commit subject in {range}; the merge message cannot comply with the schema"
		);
	};

	if args.dry_run {
		println!("{message}");
		return Ok(());
	}
	let status = Command::new("git")
		.args(["merge", "--no-ff", "-m", &message, &merge_ref])
		.status()?;
	if !status.success() {
		bail!("git merge failed with {status}");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accepts_conventional_subjects_including_scope_and_bang() {
		assert!(is_compliant_subject("fix: added thing"));
		assert!(is_compliant_subject("feat(catalog): added native meta provider"));
		assert!(is_compliant_subject("refactor(tui)!: dropped legacy renderer"));
	}

	#[test]
	fn rejects_prefix_only_false_positives_and_missing_prefixes() {
		assert!(!is_compliant_subject("fix: Add Thing."));
		assert!(!is_compliant_subject("fix: Added thing"));
		assert!(!is_compliant_subject("fix: added thing."));
		assert!(!is_compliant_subject(&format!("fix: {}", "a".repeat(70))));
		assert!(!is_compliant_subject("added thing"));
		assert!(!is_compliant_subject("Fix: added thing"));
		assert!(!is_compliant_subject("fix:"));
	}

	#[test]
	fn message_uses_the_oldest_compliant_subject() {
		let subjects = ["fix: Add Thing.", "WIP", "fix: added thing", "feat: also fine"];
		assert_eq!(
			merge_message(6386, "octo", &subjects).as_deref(),
			Some("Merge PR #6386: fix: added thing (@octo)")
		);
		assert_eq!(merge_message(1, "octo", &["WIP"]), None);
	}

	#[test]
	fn pr_numbers_are_inferred_from_branch_conventions() {
		assert_eq!(pr_number_from_branch("pr/6386"), Some(6386));
		assert_eq!(pr_number_from_branch("user/PR-12/fix"), Some(12));
		assert_eq!(pr_number_from_branch("6386-fix-foo"), Some(6386));
		assert_eq!(pr_number_from_branch("feature/sprint-42"), None);
	}
}
