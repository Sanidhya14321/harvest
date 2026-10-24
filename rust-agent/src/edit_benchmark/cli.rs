use std::{
	io::{self, Write},
	path::PathBuf,
};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, clap::Args)]
pub(crate) struct Arguments {
	#[command(subcommand)]
	command: Command,
}
#[derive(Debug, clap::Args)]
struct GenerationSelectionArgs {
	/// JSON shape constraint: min/max_changed_lines and min/max_hunks.
	#[arg(long)]
	shape:           Option<PathBuf>,
	/// Read-only original/mutated SHA-256 pair history.
	#[arg(long)]
	exclude_content: Option<PathBuf>,
}
impl GenerationSelectionArgs {
	fn options(
		self,
		source_policy: super::SourcePolicy,
		difficulty: Option<super::Difficulty>,
		min_score: Option<u32>,
		excluded_regions: Vec<super::MutationRegion>,
	) -> Result<super::GenerationOptions> {
		Ok(super::GenerationOptions {
			source_policy,
			difficulty,
			min_score,
			excluded_regions,
			shape: self
				.shape
				.map(|p| -> Result<super::ShapeConstraint> {
					Ok(serde_json::from_slice(&read_document(&p, 65536)?)?)
				})
				.transpose()?,
			excluded_content: self
				.exclude_content
				.map(|p| -> Result<Vec<super::ContentIdentity>> {
					Ok(serde_json::from_slice(&read_document(&p, 1024 * 1024)?)?)
				})
				.transpose()?
				.unwrap_or_default(),
			deduplicate_content: false,
		})
	}
}
#[derive(Debug, clap::Subcommand)]
enum Command {
	/// Expand explicit per-mutation/shape quotas into a versioned case plan.
	Plan {
		#[arg(long)]
		quotas:      PathBuf,
		/// Native edit-shapes JSON; apportion unshaped quota cases by per-call
		/// size.
		#[arg(long)]
		calibration: Option<PathBuf>,
		#[arg(long, default_value_t = 42)]
		seed:        u64,
	},
	/// Execute isolated native model runs and write rust-v1 artifacts.
	Run(RunArguments),
	/// Inspect a completed native summary without model/network access.
	View {
		#[arg(long)]
		output: PathBuf,
		#[arg(long)]
		json:   bool,
	},
	Mutations,
	/// Inspect candidate source data and exclusions without mutation/model
	/// access.
	Corpus {
		#[arg(long, value_enum)]
		difficulty: Option<super::Difficulty>,
		#[arg(long)]
		source:     PathBuf,
		#[arg(long, value_enum, default_value_t = super::SourcePolicy::ReviewedTree)]
		policy:     super::SourcePolicy,
	},
	GenerateBatch {
		/// Carry successful target-content identities across this collection.
		#[arg(long)]
		deduplicate_content: bool,
		#[command(flatten)]
		selection:           GenerationSelectionArgs,
		#[arg(long)]
		source:              PathBuf,
		#[arg(long)]
		output:              PathBuf,
		/// Ordered case array or rust-calibration-v1 plan document, max 64 cases.
		#[arg(long)]
		plan:                PathBuf,
		#[arg(long, value_enum, default_value_t = super::SourcePolicy::ReviewedTree)]
		source_policy:       super::SourcePolicy,
		#[arg(long, value_enum)]
		difficulty:          Option<super::Difficulty>,
		#[arg(long, requires = "difficulty")]
		min_score:           Option<u32>,
		#[arg(long)]
		exclude_regions:     Option<PathBuf>,
	},
	Generate {
		#[command(flatten)]
		selection:       GenerationSelectionArgs,
		/// Read-only JSON array of native source-hash-bound region exclusions.
		#[arg(long)]
		exclude_regions: Option<PathBuf>,
		#[arg(long, value_enum)]
		difficulty:      Option<super::Difficulty>,
		#[arg(long, requires = "difficulty")]
		min_score:       Option<u32>,
		#[arg(long, value_enum, default_value_t = super::SourcePolicy::ReviewedTree)]
		source_policy:   super::SourcePolicy,
		#[arg(long)]
		source:          PathBuf,
		#[arg(long)]
		output:          PathBuf,
		#[arg(long)]
		task_id:         String,
		#[arg(long)]
		mutation:        String,
		#[arg(long, default_value_t = 42)]
		seed:            u64,
	},
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
	/// Export validated inert fixtures as a native tar.gz archive.
	Pack {
		#[arg(long)]
		fixtures: PathBuf,
		#[arg(long)]
		output:   PathBuf,
	},
	Unpack {
		#[arg(long)]
		archive: PathBuf,
		#[arg(long)]
		output:  PathBuf,
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
	/// Bound each model/session submit inside the original run deadline.
	#[arg(long)]
	attempt_timeout_seconds:   Option<u64>,
	#[arg(long, default_value_t = 3)]
	timeout_retries:           u32,
	/// Keep requesting model responses after a matching committed tool batch.
	#[arg(long)]
	no_early_stop_on_match:    bool,
	/// Scored attempts, excluding no-edit retries; all share one run deadline.
	#[arg(long, default_value_t = 1)]
	max_attempts:              u32,
	#[arg(long, default_value_t = 2)]
	no_edit_retries:           u32,
	#[arg(long, default_value_t = 3)]
	provider_retries:          u32,
	/// Authorize configured !credential commands in the invocation workspace.
	#[arg(long)]
	allow_credential_commands: bool,
	#[arg(long)]
	require_read_tool:         bool,
	#[arg(long)]
	require_edit_tool:         bool,
	#[arg(long)]
	export_messages:           bool,
	#[arg(long)]
	fixtures:                  PathBuf,
	#[arg(long)]
	output:                    PathBuf,
	#[arg(long, default_value_t = 1)]
	repetitions:               u32,
	#[arg(long, default_value_t = 1)]
	concurrency:               usize,
	#[arg(long, default_value_t = 300)]
	timeout_seconds:           u64,
	#[arg(long = "task")]
	task_ids:                  Vec<String>,
	#[arg(long)]
	model:                     Option<String>,
	#[arg(long)]
	base_url:                  Option<String>,
	#[arg(long)]
	max_steps:                 Option<u32>,
	#[arg(long)]
	profile:                   Option<String>,
	/// Ordered native configuration overlay (TOML, JSONC/JSON or YAML).
	#[arg(long = "config")]
	overlays:                  Vec<PathBuf>,
	/// Explicitly authorize only jailed fixture write/edit calls. No shell,
	/// plugins, browser or external service mutations are authorized.
	#[arg(long)]
	approve_fixture_edits:     bool,
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
	let model = crate::provider::HttpModel::from_configuration(
		&root,
		&resolved,
		true,
		arguments.allow_credential_commands,
	)?;
	let approval: &(dyn crate::tools::Approval + Sync) = if arguments.approve_fixture_edits {
		&FixtureApproval
	} else {
		&crate::tools::TerminalApproval
	};
	let summary = super::run_tasks(
		&arguments.fixtures,
		&arguments.output,
		&super::RunConfiguration {
			attempt_timeout: arguments
				.attempt_timeout_seconds
				.map(std::time::Duration::from_secs),
			timeout_retries: arguments.timeout_retries,
			early_stop_on_match: !arguments.no_early_stop_on_match,
			max_attempts: arguments.max_attempts,
			no_edit_retries: arguments.no_edit_retries,
			provider_retries: arguments.provider_retries,
			repetitions: arguments.repetitions,
			require_read_tool: arguments.require_read_tool,
			require_edit_tool: arguments.require_edit_tool,
			export_messages: arguments.export_messages,
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
#[derive(Serialize)]
struct ListedTask<'a> {
	id:                 &'a str,
	prompt:             &'a str,
	files:              &'a [PathBuf],
	#[serde(skip_serializing_if = "Option::is_none")]
	metadata:           Option<&'a RawValue>,
	#[serde(skip_serializing_if = "Option::is_none")]
	input_directory:    Option<&'a std::path::Path>,
	#[serde(skip_serializing_if = "Option::is_none")]
	expected_directory: Option<&'a std::path::Path>,
	#[serde(skip_serializing_if = "Option::is_none")]
	archive:            Option<&'a std::path::Path>,
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
		Command::Plan { quotas, seed, calibration } => {
			let quotas: Vec<super::GenerationQuota> =
				serde_json::from_slice(&read_document(&quotas, 1024 * 1024)?)?;
			let plan = if let Some(path) = calibration {
				super::plan_calibrated_quotas(
					&quotas,
					seed,
					&read_document(&path, 1024 * 1024)?,
					&cancellation,
				)?
			} else {
				super::plan_quotas(&quotas, seed, &cancellation)?
			};
			serde_json::to_writer(io::stdout().lock(), &plan)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
		Command::Run(arguments) => return run(arguments),
		Command::View { output, json } => {
			let summary = super::SummaryView::load(&output)?;
			if json {
				serde_json::to_writer(io::stdout().lock(), &summary)?;
				writeln!(io::stdout().lock())?;
			} else {
				summary.write_text(&mut io::stdout().lock())?;
			}
			return Ok(0);
		},
		Command::Mutations => {
			serde_json::to_writer(io::stdout().lock(), &super::MutationKind::ALL)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
		Command::Corpus { source, policy, difficulty } => {
			let report = match difficulty {
				Some(level) => {
					super::inspect_corpus_with_difficulty(&source, policy, level, &cancellation)?
				},
				None => super::inspect_corpus(&source, policy, &cancellation)?,
			};
			serde_json::to_writer(io::stdout().lock(), &report)?;
			writeln!(io::stdout().lock())?;
			return Ok(if report.eligible > 0 { 0 } else { 1 });
		},
		Command::GenerateBatch {
			deduplicate_content,
			selection,
			source,
			output,
			plan,
			source_policy,
			difficulty,
			min_score,
			exclude_regions,
		} => {
			let cases = super::calibration::decode_plan(&read_document(&plan, 1024 * 1024)?)?;
			let excluded_regions = exclude_regions
				.map(|path| -> Result<Vec<super::MutationRegion>> {
					Ok(serde_json::from_slice(&read_document(&path, 1024 * 1024)?)?)
				})
				.transpose()?
				.unwrap_or_default();
			let mut options =
				selection.options(source_policy, difficulty, min_score, excluded_regions)?;
			options.deduplicate_content = deduplicate_content;
			let report = match cases {
				super::calibration::PlanInput::Array(cases) => {
					super::generate_batch(&source, &output, &cases, &options, &cancellation)?
				},
				super::calibration::PlanInput::Document(plan) => {
					let quota_plan = super::QuotaPlan {
						version:     super::CALIBRATION_VERSION,
						seed:        plan.seed,
						cases:       plan.cases,
						calibration: plan.calibration,
					};
					super::generate_quota_batch(&source, &output, &quota_plan, &options, &cancellation)?
				},
			};

			serde_json::to_writer(io::stdout().lock(), &report)?;
			writeln!(io::stdout().lock())?;
			return Ok(if report.complete { 0 } else { 1 });
		},
		Command::Generate {
			selection,
			source,
			output,
			task_id,
			mutation,
			seed,
			source_policy,
			difficulty,
			min_score,
			exclude_regions,
		} => {
			let kind = super::MutationKind::from_name(&mutation).ok_or_else(|| {
				anyhow::anyhow!("unsupported native mutation; use edit-benchmark mutations")
			})?;
			let excluded_regions = exclude_regions
				.map(|path| -> Result<Vec<super::MutationRegion>> {
					Ok(serde_json::from_slice(&read_document(&path, 1024 * 1024)?)?)
				})
				.transpose()?
				.unwrap_or_default();
			let report = super::generate_task_with_options(
				&source,
				&output,
				&task_id,
				kind,
				seed,
				&selection.options(source_policy, difficulty, min_score, excluded_regions)?,
				&cancellation,
			)?;
			serde_json::to_writer(io::stdout().lock(), &report)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
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
			let source = super::FixtureSource::open(&fixtures, &cancellation)?;
			let issues = super::validate_fixtures(source.directory(), &cancellation)?;
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
		Command::Pack { fixtures, output } => {
			let report = super::pack_fixtures(&fixtures, &output, &cancellation)?;
			serde_json::to_writer(io::stdout().lock(), &report)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
		Command::Unpack { archive, output } => {
			let report = super::unpack_fixtures(&archive, &output, &cancellation)?;
			serde_json::to_writer(io::stdout().lock(), &report)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
		Command::List { fixtures } => {
			let source = super::FixtureSource::open(&fixtures, &cancellation)?;
			let tasks = super::load_tasks(source.directory())?;
			let listed: Vec<_> = tasks
				.iter()
				.map(|task| ListedTask {
					id:                 &task.id,
					prompt:             &task.prompt,
					files:              &task.files,
					metadata:           task.metadata.as_deref(),
					input_directory:    (!source.is_temporary())
						.then_some(task.input_directory.as_path()),
					expected_directory: (!source.is_temporary())
						.then_some(task.expected_directory.as_path()),
					archive:            source.is_temporary().then_some(fixtures.as_path()),
				})
				.collect();
			serde_json::to_writer(io::stdout().lock(), &listed)?;
			writeln!(io::stdout().lock())?;
			return Ok(0);
		},
		Command::Compare { expected, actual } => super::compare(&expected, &actual, &cancellation)?,
		Command::Apply { expected, actual, manifest } => {
			let bytes = read_document(&manifest, harvest_runtime::MAX_FILE_BYTES)?;
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

fn read_document(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
	let filesystem = harvest_runtime::JailedFs::open(
		path
			.parent()
			.filter(|p| !p.as_os_str().is_empty())
			.unwrap_or(std::path::Path::new(".")),
	)?;
	let name = path
		.file_name()
		.ok_or_else(|| anyhow::anyhow!("benchmark document has no filename"))?;
	filesystem.read(std::path::Path::new(name), limit)
}
