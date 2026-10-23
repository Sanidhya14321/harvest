use std::{
	io::{self, Write},
	path::PathBuf,
};

use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, clap::Args)]
pub(crate) struct Arguments {
	#[command(subcommand)]
	command: Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
	List {
		#[arg(long)]
		fixtures: PathBuf,
	},
	Compare {
		#[arg(long)]
		expected: PathBuf,
		#[arg(long)]
		actual:   PathBuf,
	},
	Apply {
		#[arg(long)]
		expected: PathBuf,
		#[arg(long)]
		actual:   PathBuf,
		#[arg(long)]
		manifest: PathBuf,
	},
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
	scoring_version: String,
	edits:           Vec<super::Edit>,
	usage:           Option<Box<RawValue>>,
}
pub(crate) fn execute(arguments: Arguments) -> Result<u8> {
	let cancellation = Default::default();
	let report = match arguments.command {
		Command::List { fixtures } => {
			serde_json::to_writer(io::stdout().lock(), &super::load_tasks(&fixtures)?)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
		Command::Compare { expected, actual } => super::compare(&expected, &actual, &cancellation)?,
		Command::Apply { expected, actual, manifest } => {
			let filesystem = harvest_runtime::JailedFs::open(
				manifest
					.parent()
					.filter(|path| !path.as_os_str().is_empty())
					.unwrap_or(std::path::Path::new(".")),
			)?;
			let filename = manifest
				.file_name()
				.ok_or_else(|| anyhow::anyhow!("benchmark manifest has no filename"))?;
			let bytes =
				filesystem.read(std::path::Path::new(filename), harvest_runtime::MAX_FILE_BYTES)?;
			let manifest: Manifest = serde_json::from_slice(&bytes)?;
			ensure!(
				manifest.scoring_version == super::SCORING_VERSION,
				"benchmark scores from different versions cannot be combined"
			);
			super::apply_and_score(&expected, &actual, &manifest.edits, manifest.usage, &cancellation)?
		},
	};
	serde_json::to_writer(io::stdout().lock(), &report)?;
	writeln!(io::stdout().lock())?;
	Ok(if report.success { 0 } else { 1 })
}
