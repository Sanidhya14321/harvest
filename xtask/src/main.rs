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
				"{}/{} capabilities have Rust implementations (partial or complete)",
				progress.implemented, progress.total
			);
			println!(
				"{}/{} legacy capabilities verified for Rust parity",
				progress.verified, progress.total
			);
			if selection
				.iter()
				.any(|argument| argument == "--require-complete")
				&& progress.verified != progress.total
			{
				bail!(
					"distribution cutover blocked: legacy capabilities still lack verified Rust parity"
				);
			}
			Ok(())
		},
		"help" | "--help" | "-h" => {
			println!(
				"cargo xtask check [Cargo package selectors]\ncargo xtask test [Cargo package \
				 selectors]\ncargo xtask capabilities [--require-complete]\nTests require \
				 cargo-nextest and include doctests.\ncargo xtask package <homebrew|winget> \
				 --help\ncargo xtask secrets --help"
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
