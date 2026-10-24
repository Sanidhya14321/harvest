use std::{
	io::{self, Write},
	path::{Path, PathBuf},
	time::Duration,
};

use anyhow::{Result, ensure};
use harvest_runtime::{BoundedWriter, JailedFs, cancel::CancelToken};

use super::{Options, Run};
#[derive(Debug, clap::Args)]
pub(crate) struct Arguments {
	/// Run and trace, separately or one run|trace / run/trace argument.
	#[arg(num_args=1..=2)]
	identifiers: Vec<String>,
	#[arg(long, default_value = ".")]
	workspace:   PathBuf,
	#[arg(long, default_value = "http://localhost:4700")]
	base:        String,
	/// Read a normalized trace instead of fetching the metaharness server.
	#[arg(long, conflicts_with = "session")]
	input:       Option<PathBuf>,
	/// Read an actual persisted JSONL session through the shared branch codec.
	#[arg(long, requires = "leaf", conflicts_with = "input")]
	session:     Option<PathBuf>,
	#[arg(long, requires = "session")]
	leaf:        Option<String>,

	/// Optional run metadata for offline input.
	#[arg(long, requires = "input")]
	metadata: Option<PathBuf>,
	/// Native configuration profile for the map model.
	#[arg(long)]
	tiny_profile: Option<String>,
	/// Native configuration profile for the reduce model (defaults to tiny).
	#[arg(long)]
	synth_profile: Option<String>,
	#[arg(long = "config")]
	overlays: Vec<PathBuf>,
	#[arg(long)]
	allow_credential_commands: bool,
	#[arg(long)]
	focus: Option<String>,
	/// Jailed workspace-relative output; publish only a complete report.
	#[arg(long)]
	out: Option<PathBuf>,
	#[arg(long)]
	json: bool,
	#[arg(long,default_value_t=8,value_parser=clap::value_parser!(u8).range(1..=32))]
	concurrency: u8,
	#[arg(long,default_value_t=600,value_parser=clap::value_parser!(u32).range(1..=3600))]
	timeout_seconds: u32,
}
fn identities(values: &[String]) -> Result<(String, String)> {
	let pair = match values {
		[run, trace] => Some((run.clone(), trace.clone())),
		[combined] => combined
			.split_once('|')
			.or_else(|| combined.split_once('/'))
			.map(|(a, b)| (a.into(), b.into())),
		_ => None,
	};
	let (run, trace) = pair.ok_or_else(|| anyhow::anyhow!("expected run and trace identities"))?;
	super::label(&run)?;
	super::label(&trace)?;
	// Dot segments change HTTP routing even when URL-encoded.
	for id in [&run, &trace] {
		ensure!(
			!matches!(id.as_str(), "." | "..") && !id.contains('/'),
			"invalid run/trace identity"
		);
	}
	Ok((run, trace))
}
fn open_model(
	root: &Path,
	profile: Option<String>,
	arguments: &Arguments,
	tokens: u32,
) -> Result<crate::provider::HttpModel> {
	let resolved = crate::configuration::load(
		root,
		&Default::default(),
		&crate::config::environment_from(|key| std::env::var(key).ok())?,
		&crate::configuration::LoadOptions {
			user_directory: crate::config::agent_dir_from(None, |key| std::env::var_os(key)),
			profile,
			environment_profile: std::env::var("OMP_PROFILE").ok(),
			overlays: arguments.overlays.clone(),
			..Default::default()
		},
	)?;
	crate::provider::HttpModel::from_configuration(
		root,
		&resolved,
		true,
		arguments.allow_credential_commands,
	)?
	.with_output_limit(tokens)
}
pub(crate) fn execute(arguments: Arguments) -> Result<u8> {
	let (run, trace) = identities(&arguments.identifiers)?;
	let root = arguments.workspace.canonicalize()?;
	let filesystem = JailedFs::open(&root)?;
	// Validate the mutation target before any model cost/network activity.
	let output_path = arguments
		.out
		.as_ref()
		.map(|path| filesystem.resource_path(path))
		.transpose()?;
	if let Some(output) = &output_path {
		for input in arguments
			.input
			.iter()
			.chain(arguments.metadata.iter())
			.chain(arguments.session.iter())
			.chain(arguments.overlays.iter())
		{
			ensure!(
				*output != filesystem.resource_path(input)?,
				"report output overlaps an input/configuration file"
			);
		}
	}
	let _ownership = output_path
		.as_ref()
		.map(|path| filesystem.resource_lock(path))
		.transpose()?;

	let cancellation =
		CancelToken::with_timeout(Some(Duration::from_secs(arguments.timeout_seconds.into())));
	let (data, metadata) = if let Some(session) = &arguments.session {
		(
			super::from_session(
				&filesystem.read(session, harvest_protocol::session::SESSION_LIMIT)?,
				arguments.leaf.as_deref().unwrap_or_default(),
				&run,
				&trace,
				&cancellation,
			)?,
			None,
		)
	} else if let Some(input) = &arguments.input {
		let data = super::decode(&filesystem.read(input, super::MAX_INPUT)?)?;
		let metadata: Option<Run> = arguments
			.metadata
			.as_ref()
			.map(|p| -> Result<Run> {
				Ok(serde_json::from_slice(&filesystem.read(p, super::MAX_INPUT)?)?)
			})
			.transpose()?;
		(data, metadata)
	} else {
		super::fetch(&arguments.base, &run, &trace, &cancellation)?
	};
	let tiny = open_model(&root, arguments.tiny_profile.clone(), &arguments, 300)?;
	let synth = open_model(
		&root,
		arguments
			.synth_profile
			.clone()
			.or(arguments.tiny_profile.clone()),
		&arguments,
		3000,
	)?;
	let report = super::generate(
		&data,
		metadata.as_ref(),
		&Options { run, trace, focus: arguments.focus, concurrency: arguments.concurrency.into() },
		&tiny,
		&synth,
		&cancellation,
	)?;
	let mut output = BoundedWriter::new(super::MAX_REPORT + 65536);
	if arguments.json {
		serde_json::to_writer(&mut output, &report)?;
		writeln!(output)?;
	} else {
		output.write_all(report.markdown.as_bytes())?;
	}
	cancellation.heartbeat()?;
	if let Some(path) = &output_path {
		filesystem.atomic_write(path, &output.into_inner())?;
	} else {
		io::stdout().lock().write_all(&output.into_inner())?;
	}
	Ok(0)
}
