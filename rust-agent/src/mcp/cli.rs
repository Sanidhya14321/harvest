use std::{
	io::{self, Write},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{Client, Configuration};
use crate::{config, configuration};

#[derive(Debug, clap::Args)]
pub(crate) struct Arguments {
	#[arg(short, long, default_value = ".")]
	workspace:  PathBuf,
	#[arg(long)]
	agent_dir:  Option<PathBuf>,
	#[arg(long)]
	profile:    Option<String>,
	#[arg(long = "config")]
	overlays:   Vec<PathBuf>,
	#[arg(long)]
	mcp_config: Option<PathBuf>,
	#[arg(long)]
	server:     String,
	#[command(subcommand)]
	command:    Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
	Status,
	Tools,
	Resources,
	Templates,
	Prompts,
	Read {
		uri: String,
	},
	Prompt {
		name:      String,
		#[arg(long, default_value = "{}")]
		arguments: String,
	},
	Call {
		name:      String,
		#[arg(long, default_value = "{}")]
		arguments: String,
	},
}
fn parse_arguments(arguments: &str) -> Result<Value> {
	ensure!(
		arguments.len() <= crate::process_transport::MAX_FRAME_BYTES,
		"MCP CLI arguments exceed limit"
	);
	let value: Value = serde_json::from_str(arguments)
		.map_err(|_| anyhow::anyhow!("invalid MCP CLI argument JSON"))?;
	ensure!(value.is_object(), "MCP CLI arguments must be an object");
	Ok(value)
}
fn configurations(root: &Path, arguments: &Arguments) -> Result<Vec<Configuration>> {
	if let Some(path) = &arguments.mcp_config {
		return super::load_file(root, path);
	}
	let environment = config::environment_from(|key| std::env::var(key).ok())?;
	Ok(configuration::load(root, &Default::default(), &environment, &configuration::LoadOptions {
		user_directory: config::agent_dir_from(arguments.agent_dir.clone(), |key| {
			std::env::var_os(key)
		}),
		profile: arguments.profile.clone(),
		environment_profile: std::env::var("OMP_PROFILE").ok(),
		overlays: arguments.overlays.clone(),
		..Default::default()
	})?
	.mcp)
}
pub(crate) fn execute(arguments: Arguments) -> Result<u8> {
	let root = arguments.workspace.canonicalize()?;
	let configurations = configurations(&root, &arguments)?;
	let configuration = configurations
		.iter()
		.find(|configuration| configuration.name == arguments.server && configuration.enabled)
		.context("selected MCP server is not configured or is disabled")?;
	let cancellation = Default::default();
	let mut client = Client::connect(&root, configuration, &cancellation)?;
	let result = match arguments.command {
		Command::Status => {
			json!({"server":client.name(),"serverInfo":{"name":client.info().name,"version":client.info().version},"protocolVersion":client.info().protocol_version,"capabilities":client.info().capabilities})
		},
		Command::Tools => Value::Array(
			client
				.tools()
				.iter()
				.map(|tool| json!({"hostName":tool.spec.name(),"definition":tool.definition}))
				.collect(),
		),
		Command::Resources => json!({"resources":client.resources(&cancellation)?}),
		Command::Templates => json!({"resourceTemplates":client.resource_templates(&cancellation)?}),
		Command::Prompts => json!({"prompts":client.prompts(&cancellation)?}),
		Command::Read { uri } => client.read_resource(&uri, &cancellation)?,
		Command::Prompt { name, arguments } => {
			client.get_prompt(&name, parse_arguments(&arguments)?, &cancellation)?
		},
		Command::Call { name, arguments } => {
			client.call_tool(&name, parse_arguments(&arguments)?, &cancellation)?
		},
	};
	// An explicit user CLI call is an explicit host action. Model-originated
	// calls go through Client::invoke and the session's Approval owner.
	let tool_failed = result.get("isError") == Some(&Value::Bool(true));
	serde_json::to_writer(io::stdout().lock(), &result)?;
	writeln!(io::stdout().lock())?;
	Ok(if tool_failed { 1 } else { 0 })
}
