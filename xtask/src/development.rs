//! Rust-owned development gates, preserving conservative local change
//! detection.
use std::{
	env,
	ffi::OsString,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Task {
	Check,
	Fix,
	Fmt,
	Lint,
	Test,
}

#[derive(Parser)]
#[command(name = "cargo xtask dev")]
struct Arguments {
	task:      Task,
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	always:    bool,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::try_parse_from(std::iter::once("dev".to_owned()).chain(arguments))?;
	let root = args
		.workspace
		.canonicalize()
		.context("could not resolve development workspace")?;
	let ci = env::var("CI").is_ok_and(|value| ci_enabled(&value));
	if args.task != Task::Fmt && !args.always && !ci {
		match rust_changes(&root) {
			Ok(false) => {
				println!(
					"Skipping {:?} (not in CI and no Rust-affecting changes were found).",
					args.task
				);
				return Ok(());
			},
			Err(error) => {
				eprintln!("Warning: could not inspect Git changes; running conservatively: {error:#}");
			},
			Ok(true) => {},
		}
	}
	let cargo = cargo_binary(&root);
	let commands: Vec<Vec<String>> = match args.task {
		Task::Fmt => vec![strings(&["fmt", "--all"])],
		Task::Lint => vec![lint(false)],
		Task::Check => vec![strings(&["fmt", "--all", "--", "--check"]), lint(false)],
		Task::Fix => vec![strings(&["fmt", "--all"]), lint(true)],
		Task::Test => vec![
			strings(&[
				"nextest",
				"run",
				"--workspace",
				"--exclude",
				"brush-core",
				"--status-level=fail",
				"--final-status-level=fail",
			]),
			strings(&["test", "--doc", "--workspace", "--exclude", "brush-core"]),
		],
	};
	for arguments in commands {
		let mut command = Command::new(&cargo);
		command.current_dir(&root).args(&arguments);
		if let Some(parent) = Path::new(&cargo)
			.parent()
			.filter(|path| !path.as_os_str().is_empty())
		{
			let mut paths = vec![parent.to_path_buf()];
			if let Some(path) = env::var_os("PATH") {
				paths.extend(env::split_paths(&path));
			}
			command.env("PATH", env::join_paths(paths)?);
		}
		let status = command
			.status()
			.with_context(|| format!("could not start Cargo {}", arguments[0]))?;
		if !status.success() {
			bail!("Cargo {} failed with {status}", arguments[0]);
		}
	}
	Ok(())
}

fn strings(values: &[&str]) -> Vec<String> {
	values.iter().map(|value| (*value).to_owned()).collect()
}

fn lint(fix: bool) -> Vec<String> {
	let mut arguments = strings(&["clippy", "--workspace", "--exclude", "brush-core"]);
	if fix {
		arguments.extend(strings(&[
			"--fix",
			"--allow-dirty",
			"--no-deps",
			"--allow-staged",
			"--allow-no-vcs",
		]));
	} else {
		arguments.extend(strings(&["--no-deps", "--", "-D", "warnings"]));
	}
	arguments
}

fn cargo_binary(root: &Path) -> OsString {
	let mut command = Command::new("rustup");
	command.args(["which", "cargo"]).current_dir(root);
	if let Ok(bytes) = crate::process::capture(command, 16 * 1024)
		&& let Ok(path) = String::from_utf8(bytes)
		&& !path.trim().is_empty()
	{
		return path.trim().into();
	}
	env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn ci_enabled(value: &str) -> bool {
	!matches!(value.trim().to_ascii_lowercase().as_str(), "" | "0" | "false")
}

fn rust_changes(root: &Path) -> Result<bool> {
	let mut command = Command::new("git");
	command
		.args(["status", "--porcelain", "-z", "--untracked-files=all"])
		.current_dir(root)
		.env("GIT_OPTIONAL_LOCKS", "0");
	changed_paths(&crate::process::capture(command, 8 * 1024 * 1024)?)
}

fn changed_paths(bytes: &[u8]) -> Result<bool> {
	if !bytes.is_empty() && !bytes.ends_with(b"\0") {
		bail!("unterminated Git status");
	}
	let mut entries = bytes
		.split(|byte| *byte == 0)
		.filter(|entry| !entry.is_empty());
	let mut changed = false;
	while let Some(entry) = entries.next() {
		if entry.len() < 4 || entry[2] != b' ' {
			bail!("malformed Git status");
		}
		changed |= rust_path(&entry[3..]);
		if entry[..2].iter().any(|byte| matches!(byte, b'R' | b'C')) {
			changed |= rust_path(entries.next().context("missing rename source")?);
		}
	}
	Ok(changed)
}

fn rust_path(bytes: &[u8]) -> bool {
	let path = String::from_utf8_lossy(bytes).replace('\\', "/");
	let name = path.rsplit('/').next().unwrap_or_default();
	Path::new(&path)
		.extension()
		.is_some_and(|extension| extension == "rs")
		|| path.starts_with(".cargo/")
		|| matches!(
			name,
			"Cargo.toml"
				| "Cargo.lock"
				| "build.rs"
				| "rust-toolchain"
				| "rust-toolchain.toml"
				| "clippy.toml"
				| ".clippy.toml"
				| "rustfmt.toml"
				| ".rustfmt.toml"
		)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn status_paths_include_both_sides_of_renames_and_non_utf8_rust_names() -> Result<()> {
		assert!(changed_paths(b"R  docs/moved.txt\0crates/old.rs\0")?);
		assert!(changed_paths(b"?? new/\xff.rs\0")?);
		assert!(changed_paths(b" M .cargo/config.toml\0")?);
		assert!(!changed_paths(b" M docs/guide.md\0?? web/view.ts\0")?);
		assert!(!changed_paths(b"")?);
		Ok(())
	}

	#[test]
	fn truncated_status_is_not_mistaken_for_a_clean_tree() {
		assert!(changed_paths(b" M file.rs").is_err());
		assert!(changed_paths(b"R  target.txt\0").is_err());
		assert!(changed_paths(b"bad\0").is_err());
	}

	#[test]
	fn ci_false_values_preserve_local_skips_but_other_values_force_gates() {
		for value in ["", "0", " false ", "FALSE"] {
			assert!(!ci_enabled(value));
		}
		for value in ["1", " true ", "yes"] {
			assert!(ci_enabled(value));
		}
	}
}
