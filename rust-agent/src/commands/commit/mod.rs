//! `harvest commit` — generate (and optionally push) a conventional commit.
//! Ported from `commands/commit.ts` and `commit/` (legacy deterministic
//! pipeline: `pipeline.ts`, `execute.ts`, `conventional/*`).

mod generate;

use std::{
	env,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Result, bail};

use crate::config::{self, Overrides};

#[derive(Debug, clap::Args)]
pub struct CommitArgs {
	/// Push after committing
	#[arg(long)]
	push:         bool,
	/// Preview without committing
	#[arg(long)]
	dry_run:      bool,
	/// Skip changelog updates
	#[arg(long)]
	no_changelog: bool,
	/// Use legacy deterministic pipeline
	#[arg(long)]
	legacy:       bool,
	/// Additional context for the model
	#[arg(short = 'c', long)]
	context:      Option<String>,
	/// Override model selection
	#[arg(short = 'm', long)]
	model:        Option<String>,
	/// Authorize configured !commands to retrieve credentials
	#[arg(long)]
	allow_credential_commands: bool,
}

/// A git failure (refusing hook, rejected push) with captured output.
pub(crate) struct GitError {
	pub stdout: String,
	pub stderr: String,
	pub message: String,
}

pub(crate) fn git(cwd: &Path, args: &[&str]) -> std::result::Result<String, GitError> {
	let output = Command::new("git")
		.args(args)
		.current_dir(cwd)
		.env("GIT_TERMINAL_PROMPT", "0")
		.output()
		.map_err(|e| GitError {
			stdout:  String::new(),
			stderr:  String::new(),
			message: format!("failed to run git: {e}"),
		})?;
	let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
	if output.status.success() {
		Ok(stdout)
	} else {
		Err(GitError {
			stdout,
			stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
			message: format!("git {} failed", args.first().copied().unwrap_or("")),
		})
	}
}

/// `abortOnGitFailure`: print an indented failure and exit non-zero.
fn report_git_failure(context: &str, error: &GitError) -> u8 {
	let detail = [error.stderr.trim(), error.stdout.trim(), error.message.as_str()]
		.into_iter()
		.find(|s| !s.is_empty())
		.unwrap_or_default();
	let body: Vec<String> = detail.lines().map(|line| format!("    {line}")).collect();
	eprintln!("✗ {context}:\n{}", body.join("\n"));
	1
}

/// `pushOrAbort`.
fn push(cwd: &Path) -> u8 {
	match git(cwd, &["push"]) {
		Ok(_) => {
			println!("Pushed to remote.");
			0
		},
		Err(error) => report_git_failure("Push failed", &error),
	}
}

pub fn execute(args: CommitArgs) -> Result<u8> {
	let cwd = env::current_dir()?;
	let root = git(&cwd, &["rev-parse", "--show-toplevel"])
		.map(|s| PathBuf::from(s.trim()))
		.map_err(|_| anyhow::anyhow!("not inside a git repository"))?;
	if !args.legacy {
		eprintln!(
			"Note: the agentic commit flow is not yet available natively; using the deterministic \
			 pipeline."
		);
	}
	let settings = crate::settings::Settings::load(
		config::agent_dir_from(None, |key| env::var_os(key)).as_deref(),
		&root,
		&[],
	)?;
	let model_override = args.model.clone();
	let allow = args.allow_credential_commands;
	let model_root = root.clone();
	let factory = move || -> Result<Box<dyn crate::provider::Model>> {
		let environment = config::environment_from(|key| env::var(key).ok())?;
		let resolved = crate::configuration::load(
			&model_root,
			&Overrides { model: model_override.clone(), ..Default::default() },
			&environment,
			&crate::configuration::LoadOptions {
				user_directory: config::agent_dir_from(None, |key| env::var_os(key)),
				environment_profile: env::var("OMP_PROFILE").ok(),
				..Default::default()
			},
		)?;
		Ok(Box::new(crate::provider::HttpModel::from_configuration(
			&model_root,
			&resolved,
			false,
			allow,
		)?))
	};
	let generated = match generate::generate_git_commit(
		&root,
		&settings,
		args.context.as_deref(),
		Box::new(factory),
	) {
		Ok(generated) => generated,
		Err(generate::Failure::Git(error)) => {
			return Ok(report_git_failure("Commit generation failed", &error));
		},
		Err(generate::Failure::NoChanges) => {
			if args.push {
				println!("No changes to commit; pushing existing commits...");
				return Ok(push(&root));
			}
			eprintln!("No changes to commit.");
			return Ok(0);
		},
		Err(generate::Failure::Other(error)) => return Err(error),
	};

	let message = generated.commit.format();
	if args.dry_run {
		println!("\nGenerated commit message:");
		println!("{message}");
		if let Some(error) = &generated.validation_error {
			eprintln!("Warning: generated message requires manual correction: {error}");
		}
		return Ok(0);
	}
	if let Some(error) = generated.validation_error {
		bail!("Generated commit message failed validation: {error}");
	}
	if !args.no_changelog {
		// Changelog updates (`commit/changelog/`) are not yet ported natively.
		eprintln!("Note: changelog updates are not yet available natively; skipping.");
	}
	let result = git_with_stdin(&root, &["commit", "-F", "-"], &message);
	if let Err(error) = result {
		return Ok(report_git_failure("Commit failed", &error));
	}
	println!("Commit created.");
	if args.push {
		return Ok(push(&root));
	}
	Ok(0)
}

/// Run git feeding `input` on stdin (commit message via `-F -`).
fn git_with_stdin(cwd: &Path, args: &[&str], input: &str) -> std::result::Result<String, GitError> {
	use std::io::Write as _;
	let spawn_error = |e: std::io::Error| GitError {
		stdout:  String::new(),
		stderr:  String::new(),
		message: format!("failed to run git: {e}"),
	};
	let mut child = Command::new("git")
		.args(args)
		.current_dir(cwd)
		.stdin(std::process::Stdio::piped())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.map_err(spawn_error)?;
	if let Some(mut stdin) = child.stdin.take() {
		stdin.write_all(input.as_bytes()).map_err(spawn_error)?;
	}
	let output = child.wait_with_output().map_err(spawn_error)?;
	let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
	if output.status.success() {
		Ok(stdout)
	} else {
		Err(GitError {
			stdout,
			stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
			message: "git commit failed".into(),
		})
	}
}
