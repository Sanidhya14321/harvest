//! Rust-owned validation entrypoint. Test runs always include nextest and
//! doctests.
use std::{
	env,
	process::{Command, ExitCode},
};

use anyhow::{Context, Result, bail};
use xtask::packaging;

fn cargo(arguments: &[String]) -> Result<()> {
	let status = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.args(arguments)
		.status()
		.context("could not start Cargo")?;
	if !status.success() {
		bail!("Cargo failed with {status}");
	}
	Ok(())
}

fn execute() -> Result<()> {
	let mut arguments = env::args().skip(1);
	let task = arguments.next().unwrap_or_else(|| "help".to_owned());
	let selection: Vec<String> = arguments.collect();
	match task.as_str() {
		"release-notes" => xtask::release_notes::execute(selection),
		"web-replay" => xtask::web_replay::execute(selection),
		"migration" => xtask::migration::execute(selection),
		"registry" => xtask::registry::execute(selection),
		"terminal-wasm" => xtask::terminal_wasm::execute(selection),
		"checksums" => xtask::checksums::execute(selection),
		"version" => xtask::versions::execute(selection),
		"npm-versions" => xtask::npm_versions::execute(selection),
		"package" => packaging::execute(selection),
		"secrets" => xtask::secrets::execute(selection),
		"test" => {
			let mut nextest = vec!["nextest".into(), "run".into(), "--locked".into()];
			nextest.extend(selection.iter().cloned());
			cargo(&nextest)?;
			let mut docs = vec!["test".into(), "--doc".into(), "--locked".into()];
			docs.extend(selection);
			cargo(&docs)
		},
		"check" => {
			let mut check = vec!["check".into(), "--locked".into(), "--all-targets".into()];
			check.extend(selection);
			cargo(&check)
		},
		"capabilities" => {
			let manifest: serde_json::Value =
				serde_json::from_str(include_str!("../../docs/rust-migration/capabilities.json"))?;
			let progress = xtask::capabilities::inspect(&manifest)?;
			println!(
				"{}/{} retained capabilities have Rust implementations (partial or complete)",
				progress.implemented, progress.retained
			);
			println!(
				"{}/{} retained capabilities have verified replacement contracts",
				progress.verified, progress.retained
			);
			println!(
				"{}/{} intentionally retired interfaces have verified removal ({} total entries)",
				progress.removed, progress.retired, progress.total
			);
			if selection
				.iter()
				.any(|argument| argument == "--require-complete")
			{
				progress.require_complete()?;
			}
			Ok(())
		},
		"help" | "--help" | "-h" => {
			println!(
				"cargo xtask check [Cargo package selectors]\ncargo xtask test [Cargo package \
				 selectors]\ncargo xtask migration <audit|status|verify> [--help]\ncargo xtask \
				 registry verify --help\ncargo xtask capabilities [--require-complete]\nTests require \
				 cargo-nextest and include doctests.\ncargo xtask package <homebrew|winget|archives> \
				 --help\ncargo xtask secrets --help\ncargo xtask checksums [--root DIR] \
				 [--require-platforms] OUTPUT ASSET...\ncargo xtask web-replay --help\ncargo xtask \
				 release-notes --help\ncargo xtask terminal-wasm [--check]\ncargo xtask version \
				 [--root DIR] [--check] [VERSION]\ncargo xtask npm-versions [--root DIR] [--check] \
				 [--release VERSION|--workspace]"
			);
			Ok(())
		},
		_ => bail!("unknown task {task}"),
	}
}

fn main() -> ExitCode {
	match execute() {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("{error:#}");
			ExitCode::FAILURE
		},
	}
}
