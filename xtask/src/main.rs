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
		"dev" => xtask::development::execute(selection),
		"path-versions" => xtask::versions::execute(selection),
		"npm-versions" => xtask::npm_versions::execute(selection),
		"registry" => xtask::registry::execute(selection),
		"source-audit" => xtask::source_audit::execute(selection),
		"terminal-wasm" => xtask::terminal_wasm::execute(selection),
		"web-replay" => xtask::web_replay::execute(selection),
		"checksums" => xtask::checksums::execute(selection),
		"clippy-config" => xtask::clippy_config::execute(selection),
		"fix-elf-verdef" => xtask::elf::execute(selection),
		"emit-extensions" => xtask::emit_extensions::execute(selection),
		"theme-index" => xtask::theme_index::execute(selection),
		"release-notes" => xtask::release_notes::execute(selection),
		"version" => xtask::version::execute(selection),
		"package" => packaging::execute(selection),
		"migration" => xtask::migration::execute(selection),
		"playbook" => xtask::playbook::execute(selection),
		"acceptance" => xtask::acceptance::execute(selection),
		"spoofed-versions" => xtask::spoofed_versions::execute(selection),
		"secrets" => xtask::secrets::execute(selection),
		"test" => {
			let doctest_selection = xtask::testing::doctest_selection(&selection)?;
			let mut nextest = vec!["nextest".into(), "run".into(), "--locked".into()];
			nextest.extend(selection.iter().cloned());
			cargo(&nextest)?;
			let mut docs = vec!["test".into(), "--doc".into(), "--locked".into()];
			docs.extend(doctest_selection);
			cargo(&docs)
		},
		"check" => {
			let mut check = vec!["check".into(), "--locked".into(), "--all-targets".into()];
			check.extend(selection);
			cargo(&check)
		},
		"capabilities" => xtask::capabilities::execute(selection),
		"help" | "--help" | "-h" => {
			println!(
				"cargo xtask npm-versions [--check] [--release VERSION|--workspace]\ncargo xtask \
				 path-versions [VERSION] [--check]\ncargo xtask registry --help\ncargo xtask \
				 source-audit --help\ncargo xtask terminal-wasm [--check]\ncargo xtask web-replay \
				 --help\ncargo xtask dev <check|fmt|lint|fix|test> [--always]\ncargo xtask checksums \
				 OUTPUT ASSET...\ncargo xtask emit-extensions --help\ncargo xtask theme-index \
				 --help\ncargo xtask clippy-config --help\ncargo xtask fix-elf-verdef --help\ncargo \
				 xtask release-notes --help\ncargo xtask version [--set VERSION --expect \
				 CURRENT]\ncargo xtask check [Cargo package selectors]\ncargo xtask test [Cargo \
				 package and target selectors]\ncargo xtask capabilities [--require-complete]\nTests \
				 require cargo-nextest and include doctests.\ncargo xtask package <homebrew|winget> \
				 --help\ncargo xtask secrets --help\ncargo xtask migration \
				 [--require-complete]\ncargo xtask playbook <check|status|next> [--json]"
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
			if let Some(arguments) = error.downcast_ref::<clap::Error>() {
				let _ = arguments.print();
				return ExitCode::from(u8::try_from(arguments.exit_code()).unwrap_or(1));
			}
			eprintln!("{error:#}");
			ExitCode::FAILURE
		},
	}
}
