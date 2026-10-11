use std::{
	env,
	io::{self, Write},
	path::PathBuf,
};

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use clap_mangen::Man;

use crate::{
	agent,
	config::{self, Overrides},
	provider::{HttpModel, Model},
	shell,
};

#[derive(Debug, Parser)]
#[command(name = "omp", version, about = "Rust coding agent for Harvest")]
struct Cli {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
	#[command(about = "Run one prompt against the workspace")]
	Run(RunArgs),
	#[command(about = "List the built-in tools")]
	Tools,
	#[command(about = "Inspect compatible session JSONL without changing its source")]
	Session(SessionArgs),
	#[command(about = "Print or install shell completions")]
	Completions(CompletionArgs),
	#[command(about = "Print the manual page")]
	Man,
	#[command(about = "Retrieve and organize project and user brains")]
	Brain(crate::brain_cli::BrainArgs),
	#[command(
		name = "organize-skills",
		visible_alias = "orgnize-skills",
		alias = "/organize-skills",
		alias = "/orgnize-skills"
	)]
	Organize(crate::brain_cli::OrganizationArgs),
}

#[derive(Debug, clap::Args)]
struct SessionArgs {
	#[arg(short, long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	leaf:      Option<String>,
	file:      PathBuf,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
	#[arg(long, default_value = ".omp/session.jsonl")]
	session:   PathBuf,
	#[arg(long)]
	leaf:      Option<String>,
	#[arg(long, help = "Emit ordered JSON events, including provisional streamed text")]
	events:    bool,
	#[arg(short, long)]
	prompt:    String,
	#[arg(short, long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	agent_dir: Option<PathBuf>,
	#[arg(long)]
	tag:       Vec<String>,
	#[arg(long)]
	model:     Option<String>,
	#[arg(long)]
	base_url:  Option<String>,
	#[arg(long)]
	api_key:   Option<String>,
	#[arg(long)]
	max_steps: Option<u32>,
}

#[derive(Debug, clap::Args)]
struct CompletionArgs {
	shell:     Option<String>,
	#[arg(long)]
	install:   bool,
	#[arg(long)]
	automatic: bool,
}

pub(crate) fn execute() -> Result<u8> {
	let parsed = Cli::try_parse();
	match parsed {
		Ok(cli) => dispatch(cli),
		Err(error) => {
			error.print().context("failed to print the command line")?;
			Ok(u8::try_from(error.exit_code()).unwrap_or(1))
		},
	}
}

fn dispatch(cli: Cli) -> Result<u8> {
	match cli.command {
		Commands::Run(args) => run(args),
		Commands::Tools => tools(),
		Commands::Session(args) => inspect_session(args),
		Commands::Completions(args) => completions(args),
		Commands::Man => manual(),
		Commands::Brain(args) => crate::brain_cli::execute(args),
		Commands::Organize(args) => crate::brain_cli::organize_command(args),
	}
}

fn inspect_session(args: SessionArgs) -> Result<u8> {
	let fs = harvest_runtime::JailedFs::open(&args.workspace)?;
	let bytes = fs.read(&args.file, crate::session_format::SESSION_LIMIT)?;
	let document = crate::session_format::SessionDocument::decode(&bytes)?;
	let branch_entries = args
		.leaf
		.as_deref()
		.map(|leaf| document.branch(leaf).map(|branch| branch.len()))
		.transpose()?;
	#[derive(serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	struct SessionInspection<'a> {
		header:             &'a crate::session_format::SessionObject,
		entries:            usize,
		branch_entries:     Option<usize>,
		malformed_records:  usize,
		migration_required: bool,
	}
	let report = SessionInspection {
		header: document.header(),
		entries: document.entries().len(),
		branch_entries,
		malformed_records: document.malformed_records(),
		migration_required: document.migrated(),
	};
	serde_json::to_writer(io::stdout().lock(), &report)?;
	writeln!(io::stdout().lock())?;
	Ok(0)
}

fn run(args: RunArgs) -> Result<u8> {
	if args.prompt.is_empty() {
		bail!("prompt is empty");
	}
	let workspace = args
		.workspace
		.canonicalize()
		.with_context(|| format!("failed to resolve {}", args.workspace.display()))?;
	let environment = config::environment_from(|key| env::var(key).ok())?;
	let settings = config::resolve(
		&workspace,
		&Overrides {
			model:     args.model,
			base_url:  args.base_url,
			api_key:   args.api_key,
			max_steps: args.max_steps,
		},
		&environment,
	)?;
	let max_steps = settings.max_steps;
	let model = HttpModel::new(settings);
	let model_ref: &dyn Model = &model;
	let user_agent_dir =
		config::agent_dir_from(args.agent_dir, |key| env::var_os(key)).filter(|path| path.is_dir());
	let mut session =
		crate::agent_session::AgentSession::open(&workspace, &args.session, args.leaf.as_deref())?;
	let mut output = io::stdout().lock();
	let answer = session.submit(
		&args.prompt,
		model_ref,
		&crate::tools::TerminalApproval,
		&agent::RunOptions {
			max_steps,
			user_agent_dir,
			tags: args.tag.into_iter().collect(),
			cancellation: Default::default(),
		},
		&mut |event| {
			if args.events {
				serde_json::to_writer(&mut output, &event)?;
				writeln!(output)?;
				output.flush()?;
			}
			Ok(())
		},
	)?;
	if !args.events {
		writeln!(output, "{answer}")?;
	}
	Ok(0)
}

fn tools() -> Result<u8> {
	let mut sink = io::stdout().lock();
	for spec in crate::chat::specs() {
		writeln!(sink, "{}", spec.name())?;
	}
	Ok(0)
}

fn completions(args: CompletionArgs) -> Result<u8> {
	let name = args
		.shell
		.or_else(shell::detected_shell)
		.context("could not detect a supported shell; name one explicitly")?;
	let script = completion_script(&name)?;
	if args.install || args.automatic {
		let layout = process_layout()?;
		let path = shell::install(&name, &layout, &script)?;
		writeln!(io::stdout(), "{}", path.display())?;
		return Ok(0);
	}
	write!(io::stdout(), "{script}")?;
	Ok(0)
}

pub(crate) fn completion_script(shell: &str) -> Result<String> {
	let mut command = Cli::command();
	let mut script = Vec::new();
	generate(clap_shell(shell)?, &mut command, "omp", &mut script);
	String::from_utf8(script).context("the completion script was not valid UTF-8")
}

fn process_layout() -> Result<shell::Layout> {
	let home = env::var_os("HOME")
		.or_else(|| env::var_os("USERPROFILE"))
		.map(PathBuf::from)
		.context("HOME is not set")?;
	let data = env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), PathBuf::from);
	let state = env::var_os("XDG_STATE_HOME")
		.map_or_else(|| home.join(".local/state"), PathBuf::from)
		.join("omp");
	// nosemgrep: rust.lang.security.current-exe.current-exe
	let executable = env::current_exe().context("failed to locate the omp executable")?;
	Ok(shell::Layout { home, data, state, executable })
}

fn manual() -> Result<u8> {
	let mut page = Vec::new();
	Man::new(Cli::command())
		.render(&mut page)
		.context("failed to render the manual page")?;
	let text = String::from_utf8(page).context("the manual page was not valid UTF-8")?;
	write!(io::stdout(), "{text}")?;
	Ok(0)
}

fn clap_shell(shell: &str) -> Result<Shell> {
	match shell {
		"bash" => Ok(Shell::Bash),
		"zsh" => Ok(Shell::Zsh),
		"fish" => Ok(Shell::Fish),
		"powershell" | "pwsh" => Ok(Shell::PowerShell),
		"elvish" => Ok(Shell::Elvish),
		_ => bail!("unsupported shell {shell}"),
	}
}
