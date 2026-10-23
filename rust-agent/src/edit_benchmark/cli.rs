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
	/// Execute isolated native model runs and write rust-v1 artifacts.
	Run(RunArguments),
	/// Calibrate edit shapes from existing session logs, without running models.
	EditShapes {
		#[arg(long)]
		sessions_dir: Option<PathBuf>,
		#[arg(long, default_value = ".")]
		repo:         PathBuf,
		#[arg(long, default_value_t = 2000)]
		max_sessions: usize,
		#[arg(long)]
		json:         bool,
	},
	Validate {
		#[arg(long)]
		fixtures: PathBuf,
	},
	Prepare {
		#[arg(long)]
		input:    PathBuf,
		#[arg(long)]
		expected: PathBuf,
		#[arg(long)]
		output:   PathBuf,
		#[arg(long)]
		task_id:  String,
	},
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
#[derive(Debug, clap::Args)]
struct RunArguments {
	#[arg(long)]
	fixtures:              PathBuf,
	#[arg(long)]
	output:                PathBuf,
	#[arg(long, default_value_t = 1)]
	repetitions:           u32,
	#[arg(long, default_value_t = 1)]
	concurrency:           usize,
	#[arg(long, default_value_t = 300)]
	timeout_seconds:       u64,
	#[arg(long = "task")]
	task_ids:              Vec<String>,
	#[arg(long)]
	model:                 Option<String>,
	#[arg(long)]
	base_url:              Option<String>,
	#[arg(long)]
	max_steps:             Option<u32>,
	#[arg(long)]
	profile:               Option<String>,
	#[arg(long = "config")]
	overlays:              Vec<PathBuf>,
	/// Explicitly authorize only jailed fixture write/edit calls. No shell,
	/// plugins, browser or external service mutations are authorized.
	#[arg(long)]
	approve_fixture_edits: bool,
}
struct FixtureApproval;
impl crate::tools::Approval for FixtureApproval {
	fn approve(&self, call: &crate::chat::ToolCall) -> Result<bool> {
		Ok(matches!(call.function.name.as_str(), "write" | "edit"))
	}
}
fn run(arguments: RunArguments) -> Result<u8> {
	ensure!(
		arguments.approve_fixture_edits || arguments.concurrency == 1,
		"parallel CLI runs require --approve-fixture-edits; terminal approval is serial"
	);
	let root = std::env::current_dir()?;
	let environment = crate::config::environment_from(|key| std::env::var(key).ok())?;
	let resolved = crate::configuration::load(
		&root,
		&crate::config::Overrides {
			model:     arguments.model,
			base_url:  arguments.base_url,
			api_key:   None,
			max_steps: arguments.max_steps,
		},
		&environment,
		&crate::configuration::LoadOptions {
			user_directory: crate::config::agent_dir_from(None, |key| std::env::var_os(key)),
			profile: arguments.profile,
			environment_profile: std::env::var("OMP_PROFILE").ok(),
			overlays: arguments.overlays,
			..Default::default()
		},
	)?;
	let max_steps = resolved.settings.max_steps;
	let model = crate::provider::HttpModel::new(resolved.settings).request_usage(true);
	let model = if resolved.api == crate::configuration::Api::AnthropicMessages {
		model.anthropic_messages(resolved.max_output_tokens)?
	} else {
		model
	};
	let approval: &(dyn crate::tools::Approval + Sync) = if arguments.approve_fixture_edits {
		&FixtureApproval
	} else {
		&crate::tools::TerminalApproval
	};
	let summary = super::run_tasks(
		&arguments.fixtures,
		&arguments.output,
		&super::RunConfiguration {
			repetitions: arguments.repetitions,
			concurrency: arguments.concurrency,
			max_steps,
			timeout: std::time::Duration::from_secs(arguments.timeout_seconds),
			task_ids: arguments.task_ids,
		},
		&Default::default(),
		&|_, _| Ok(model.clone()),
		approval,
	)?;
	serde_json::to_writer(io::stdout().lock(), &summary)?;
	writeln!(io::stdout().lock())?;
	Ok(if summary.unsuccessful == 0 { 0 } else { 1 })
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
		Command::Run(arguments) => return run(arguments),
		Command::EditShapes { sessions_dir, repo, max_sessions, json } => {
			let sessions = match sessions_dir {
				Some(path) => path,
				None => super::shapes::discover_sessions(&repo)?,
			};
			let report = super::analyze_shapes(&sessions, max_sessions, &cancellation)?;
			if json {
				serde_json::to_writer(io::stdout().lock(), &report)?;
				writeln!(io::stdout().lock())?;
			} else {
				report.write_text(&mut io::stdout().lock())?;
			}
			return Ok(
				if report.parsed_diffs == 0
					|| !report.failed_files.is_empty()
					|| report.malformed_records > 0
				{
					1
				} else {
					0
				},
			);
		},
		Command::Validate { fixtures } => {
			let issues = super::validate_fixtures(&fixtures, &cancellation)?;
			serde_json::to_writer(io::stdout().lock(), &issues)?;
			writeln!(io::stdout().lock())?;
			return Ok(if issues.is_empty() { 0 } else { 1 });
		},
		Command::Prepare { input, expected, output, task_id } => {
			let report = super::prepare_task(&input, &expected, &output, &task_id, &cancellation)?;
			serde_json::to_writer(io::stdout().lock(), &report)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
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
