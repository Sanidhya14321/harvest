use std::{
	env,
	io::{self, Write},
	path::{Path, PathBuf},
	sync::Arc,
};

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use clap_mangen::Man;

use crate::{
	agent,
	config::{self, Overrides},
	provider::{HttpModel, Model},
	session_engine::{SessionEngine, SessionEngineOptions},
	shell,
};

#[derive(Debug, Parser)]
#[command(name = "omp", version = env!("HARVEST_VERSION"), about = "Rust coding agent for Harvest")]
struct Cli {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
	#[command(about = "Query or refactor with a configured native language server")]
	Lsp(crate::language_services::cli::Arguments),
	#[command(about = "Inspect or control a program with a configured native debug adapter")]
	Debug(crate::language_services::cli::Arguments),
	#[command(about = "Inspect or call an explicitly configured native MCP server")]
	Mcp(crate::mcp::cli::Arguments),
	#[command(about = "Show redacted resolved native configuration and profile selection")]
	Config(ConfigArgs),
	#[command(about = "List fixtures or score edits using the separate rust-v1 baseline")]
	EditBenchmark(crate::edit_benchmark::cli::Arguments),
	#[command(name = "setup", hide = true, disable_help_flag = true)]
	RetiredSetup {
		component: String,
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		arguments: Vec<std::ffi::OsString>,
	},
	#[command(about = "Convert a DOCX document to HTML, Markdown or JSON using Rust")]
	Document(DocumentArgs),
	#[command(name = "browser-relay", hide = true, disable_help_flag = true)]
	RetiredBrowserRelay {
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		arguments: Vec<std::ffi::OsString>,
	},
	#[command(about = "Control a dedicated Chrome profile using native CDP JSONL commands")]
	Browser(crate::browser_cli::BrowserArgs),
	#[command(about = "Run one prompt against the workspace")]
	Run(RunArgs),
	#[command(about = "Serve the experimental Rust JSONL v1 RPC subset")]
	Rpc(RpcArgs),
	#[command(about = "Render a persisted branch with the portable Rust terminal layout")]
	Replay(ReplayArgs),
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
struct DocumentArgs {
	#[arg(short, long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long, default_value = "markdown", value_parser = ["html", "markdown", "json"])]
	format:    String,
	file:      PathBuf,
}

#[derive(Debug, clap::Args)]
struct ReplayArgs {
	#[arg(short, long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	leaf:      String,
	#[arg(long, default_value_t = 80, value_parser = clap::value_parser!(u16).range(1..))]
	columns:   u16,
	file:      PathBuf,
}

#[derive(Debug, clap::Args)]
struct ConfigArgs {
	#[arg(short, long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	agent_dir: Option<PathBuf>,
	#[arg(long)]
	profile:   Option<String>,
	#[arg(long = "config")]
	overlays:  Vec<PathBuf>,
}
fn inspect_configuration(args: ConfigArgs) -> Result<u8> {
	let root = args.workspace.canonicalize()?;
	let environment = config::environment_from(|key| env::var(key).ok())?;
	let resolved = crate::configuration::load(
		&root,
		&Overrides::default(),
		&environment,
		&crate::configuration::LoadOptions {
			user_directory: config::agent_dir_from(args.agent_dir, |key| env::var_os(key)),
			profile: args.profile,
			environment_profile: env::var("OMP_PROFILE").ok(),
			overlays: args.overlays,
			..Default::default()
		},
	)?;
	let output = serde_json::json!({"model":resolved.settings.model,"maxSteps":resolved.settings.max_steps,"api":match resolved.api { crate::configuration::Api::OpenaiChat=>"openai-chat",crate::configuration::Api::AnthropicMessages=>"anthropic-messages" },"maxOutputTokens":resolved.max_output_tokens,"apiKeyConfigured":!resolved.settings.api_key.is_empty(),"endpointConfigured":!resolved.settings.base_url.is_empty(),"configuredPlugins":resolved.plugins.len(),"configuredMcpServers":resolved.mcp.len(),"configuredLanguageServers":resolved.lsp.len(),"configuredDebugAdapters":resolved.dap.len()});
	serde_json::to_writer(io::stdout().lock(), &output)?;
	writeln!(io::stdout().lock())?;
	Ok(0)
}

#[derive(Debug, Default, clap::Args)]
struct ConfigurationArgs {
	#[arg(long, help = "Enable configured native language-server tools")]
	enable_lsp:     bool,
	#[arg(long, help = "Enable configured native debugger tools")]
	enable_debug:   bool,
	#[arg(long, help = "Connect MCP servers declared in native configuration")]
	enable_mcp:     bool,
	#[arg(
		long = "mcp-config",
		help = "Connect enabled servers from an explicitly selected MCP JSON file"
	)]
	mcp_configs:    Vec<PathBuf>,
	#[arg(long)]
	profile:        Option<String>,
	#[arg(long = "config")]
	overlays:       Vec<PathBuf>,
	#[arg(long, help = "Allow native executables declared in the selected configuration")]
	enable_plugins: bool,
	#[arg(long = "plugin", help = "Explicit native executable plugin; never a TS/JS module")]
	plugins:        Vec<PathBuf>,
}
impl ConfigurationArgs {
	fn load(
		&self,
		root: &Path,
		flags: &Overrides,
		environment: &config::Environment,
		api: Option<&str>,
		max_output_tokens: Option<u32>,
		agent_dir: Option<PathBuf>,
	) -> Result<crate::configuration::Resolved> {
		let api = api.map(|api| {
			if api == "anthropic-messages" {
				crate::configuration::Api::AnthropicMessages
			} else {
				crate::configuration::Api::OpenaiChat
			}
		});
		let mut resolved = crate::configuration::load(
			root,
			flags,
			environment,
			&crate::configuration::LoadOptions {
				user_directory: config::agent_dir_from(agent_dir, |key| env::var_os(key)),
				profile: self.profile.clone(),
				environment_profile: env::var("OMP_PROFILE").ok(),
				overlays: self.overlays.clone(),
				api,
				max_output_tokens,
			},
		)?;
		// Configurations remain inspectable without launching programs. Runtime
		// exposure is an explicit host choice; adapters start lazily on invocation.
		if !self.enable_lsp {
			resolved.lsp.clear();
		}
		if !self.enable_debug {
			resolved.dap.clear();
		}

		if !resolved.mcp.is_empty() && !self.enable_mcp {
			bail!("MCP servers in native configuration require --enable-mcp");
		}
		for path in &self.mcp_configs {
			resolved.mcp.extend(crate::mcp::load_file(root, path)?);
		}
		if resolved.mcp.len() > 32 {
			bail!("too many MCP servers");
		}
		let mut mcp_names = std::collections::BTreeSet::new();
		for server in &resolved.mcp {
			if !mcp_names.insert(&server.name) {
				bail!("duplicate MCP server name across selected configurations");
			}
		}

		if !resolved.plugins.is_empty() && !self.enable_plugins {
			bail!("native plugins in configuration require --enable-plugins");
		}
		for executable in &self.plugins {
			resolved.plugins.push(crate::plugins::Configuration {
				executable:  executable.clone(),
				arguments:   Vec::new(),
				environment: Default::default(),
				timeout_ms:  30_000,
			});
		}
		if resolved.plugins.len() > 16 {
			bail!("too many configured plugins");
		}
		Ok(resolved)
	}
}

#[derive(Debug, clap::Args)]
struct RpcArgs {
	#[command(flatten)]
	configuration:     ConfigurationArgs,
	#[arg(long, default_value = ".omp/session.jsonl")]
	session:           PathBuf,
	#[arg(long)]
	leaf:              Option<String>,
	#[arg(short, long, default_value = ".")]
	workspace:         PathBuf,
	#[arg(long)]
	agent_dir:         Option<PathBuf>,
	#[arg(long)]
	tag:               Vec<String>,
	#[arg(long)]
	model:             Option<String>,
	#[arg(long, value_parser = ["openai-chat", "anthropic-messages"])]
	api:               Option<String>,
	#[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
	max_output_tokens: Option<u32>,

	#[arg(long)]
	base_url:  Option<String>,
	#[arg(long)]
	api_key:   Option<String>,
	#[arg(long)]
	max_steps: Option<u32>,
}

#[derive(Debug, clap::Args)]
struct SessionArgs {
	#[arg(short, long, default_value = ".")]
	workspace:  PathBuf,
	#[arg(long)]
	leaf:       Option<String>,
	#[arg(
		long,
		value_name = "FILE",
		help = "Import every branch to a separate v3 file without overwriting existing files"
	)]
	import_v3:  Option<PathBuf>,
	#[arg(
		long,
		value_name = "FILE",
		conflicts_with = "import_v3",
		help = "Recover an interrupted final v3 record into a new file; preserve the original"
	)]
	recover_to: Option<PathBuf>,
	#[arg(long, value_name = "TEXT", conflicts_with_all = ["import_v3", "recover_to"], help = "Update only the title slot of an intact v3 session")]
	title:      Option<String>,
	file:       PathBuf,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
	#[command(flatten)]
	configuration:     ConfigurationArgs,
	#[arg(
		long,
		requires = "browser_profile",
		help = "Enable approved native browser tools with this Chrome executable"
	)]
	browser_chrome:    Option<PathBuf>,
	#[arg(
		long,
		requires = "browser_chrome",
		help = "Dedicated persistent profile for native browser tools"
	)]
	browser_profile:   Option<PathBuf>,
	#[arg(long, requires = "browser_chrome")]
	browser_headless:  bool,
	#[arg(long, help = "Use the experimental asynchronous session owner")]
	async_session:     bool,
	#[arg(long, help = "Explicitly request OpenAI-style streamed usage; requires host support")]
	request_usage:     bool,
	#[arg(long, default_value = ".omp/session.jsonl")]
	session:           PathBuf,
	#[arg(long)]
	leaf:              Option<String>,
	#[arg(long, help = "Emit ordered JSON events, including provisional streamed text")]
	events:            bool,
	#[arg(short, long)]
	prompt:            String,
	#[arg(short, long, default_value = ".")]
	workspace:         PathBuf,
	#[arg(long)]
	agent_dir:         Option<PathBuf>,
	#[arg(long)]
	tag:               Vec<String>,
	#[arg(long)]
	model:             Option<String>,
	#[arg(long, value_parser = ["openai-chat", "anthropic-messages"])]
	api:               Option<String>,
	#[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
	max_output_tokens: Option<u32>,

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
		Commands::Lsp(arguments) => crate::language_services::cli::execute("lsp", arguments),
		Commands::Debug(arguments) => crate::language_services::cli::execute("debug", arguments),
		Commands::Mcp(arguments) => crate::mcp::cli::execute(arguments),
		Commands::Config(args) => inspect_configuration(args),
		Commands::EditBenchmark(arguments) => crate::edit_benchmark::cli::execute(arguments),
		Commands::Document(args) => {
			let fs = harvest_runtime::JailedFs::open(&args.workspace)?;
			let input = fs.read(&args.file, harvest_media::docx::INPUT_LIMIT)?;
			let cancel = Default::default();
			let text = match args.format.as_str() {
				"markdown" => harvest_media::docx::markdown(&input, &cancel)?,
				format => {
					let result = harvest_media::docx::convert(&input, &Default::default(), &cancel)?;
					if format == "json" {
						serde_json::to_string(&result)?
					} else {
						result.value
					}
				},
			};
			io::stdout().lock().write_all(text.as_bytes())?;
			Ok(0)
		},
		Commands::RetiredSetup { component, arguments } => {
			if component != "python" {
				bail!("setup component {component} is not implemented in the native application");
			}
			let reason = crate::tools::PYTHON_EVAL_RETIRED.trim();
			if arguments.iter().any(|arg| arg == "--json") {
				println!("{}", serde_json::json!({"available":false,"retired":true,"reason":reason}));
				return Ok(1);
			}
			bail!("{reason}");
		},
		Commands::RetiredBrowserRelay { .. } => bail!(
			"The Chrome extension relay has been removed. Use the native harvest browser --chrome \
			 PATH --profile NEW_DIRECTORY controller. Sign in separately in that dedicated profile; \
			 existing Chrome profiles are never attached. See docs/browser-migration.md."
		),
		Commands::Browser(args) => crate::browser_cli::execute(args),
		Commands::Run(args) => run(args),
		Commands::Rpc(args) => rpc(args),
		Commands::Replay(args) => {
			crate::replay_cli::execute(&args.workspace, &args.file, &args.leaf, args.columns)?;
			Ok(0)
		},
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
	let _source_ownership =
		if args.import_v3.is_some() || args.recover_to.is_some() || args.title.is_some() {
			Some(crate::durable_session::lock_session(&fs, &args.file)?)
		} else {
			None
		};
	let bytes = fs.read(&args.file, crate::session_format::SESSION_LIMIT)?;
	let mut document = crate::session_format::SessionDocument::decode(&bytes)?;
	let branch_entries = args
		.leaf
		.as_deref()
		.map(|leaf| document.branch(leaf).map(|branch| branch.len()))
		.transpose()?;
	if let Some(title) = &args.title {
		let bytes = document.retitled(title, Some("user"), &chrono::Utc::now().to_rfc3339())?;
		fs.atomic_write(&args.file, &bytes)?;
		document = crate::session_format::SessionDocument::decode(&bytes)?;
	}
	if let Some(destination) = &args.import_v3 {
		document.import_to(&fs, destination)?;
	}
	let recovery = args
		.recover_to
		.as_ref()
		.map(|destination| document.recover_to(&fs, destination))
		.transpose()?;
	#[derive(serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	struct SessionInspection<'a> {
		header:             &'a crate::session_format::SessionObject,
		entries:            usize,
		branch_entries:     Option<usize>,
		malformed_records:  usize,
		migration_required: bool,
		#[serde(skip_serializing_if = "Option::is_none")]
		imported_to:        Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		recovered_to:       Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		recovery:           Option<crate::session_format::RecoveryReport>,
	}
	let report = SessionInspection {
		header: document.header(),
		entries: document.entries().len(),
		branch_entries,
		malformed_records: document.malformed_records(),
		migration_required: document.migrated(),
		imported_to: args
			.import_v3
			.as_ref()
			.map(|path| path.display().to_string()),
		recovered_to: args
			.recover_to
			.as_ref()
			.map(|path| path.display().to_string()),
		recovery,
	};
	serde_json::to_writer(io::stdout().lock(), &report)?;
	writeln!(io::stdout().lock())?;
	Ok(0)
}

fn configured_model(
	settings: config::Settings,
	api: crate::configuration::Api,
	max_tokens: u32,
	usage: bool,
) -> Result<HttpModel> {
	let model = HttpModel::new(settings).request_usage(usage);
	if api == crate::configuration::Api::AnthropicMessages {
		model.anthropic_messages(max_tokens)
	} else {
		Ok(model)
	}
}

fn rpc(args: RpcArgs) -> Result<u8> {
	let workspace = args
		.workspace
		.canonicalize()
		.context("failed to resolve RPC workspace")?;
	let environment = config::environment_from(|key| env::var(key).ok())?;
	let resolved = args.configuration.load(
		&workspace,
		&Overrides {
			model:     args.model,
			base_url:  args.base_url,
			api_key:   args.api_key,
			max_steps: args.max_steps,
		},
		&environment,
		args.api.as_deref(),
		args.max_output_tokens,
		args.agent_dir.clone(),
	)?;
	let settings = resolved.settings;
	let user_agent_dir =
		config::agent_dir_from(args.agent_dir, |key| env::var_os(key)).filter(|path| path.is_dir());
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(2)
		.enable_all()
		.build()?;
	let result = runtime.block_on(async {
		let options = crate::rpc::RpcOptions {
			session_file: workspace.join(&args.session),
			model:        settings.model.clone(),
		};
		let max_model_turns = Some(settings.max_steps);
		let session = SessionEngine::open(
			&workspace,
			&args.session,
			args.leaf.as_deref(),
			Arc::new(configured_model(settings, resolved.api, resolved.max_output_tokens, false)?),
			Arc::new(crate::tools::DenyMutations),
			SessionEngineOptions {
				max_model_turns,
				plugins: resolved.plugins,
				mcp: resolved.mcp,
				lsp: resolved.lsp,
				dap: resolved.dap,
				user_agent_dir,
				tags: args.tag.into_iter().collect(),
				event_capacity: 256,
				..Default::default()
			},
		)
		.await?;
		crate::rpc::serve(session, tokio::io::stdin(), tokio::io::stdout(), options).await?;
		Ok(0)
	});
	// Tokio's stdin adapter can retain a blocking read after its async task
	// is cancelled. The session has already drained and released ownership;
	// don't let a disconnected client keep the CLI alive through that read.
	runtime.shutdown_timeout(std::time::Duration::from_secs(1));
	result
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
	let resolved = args.configuration.load(
		&workspace,
		&Overrides {
			model:     args.model.clone(),
			base_url:  args.base_url.clone(),
			api_key:   args.api_key.clone(),
			max_steps: args.max_steps,
		},
		&environment,
		args.api.as_deref(),
		args.max_output_tokens,
		args.agent_dir.clone(),
	)?;
	let settings = resolved.settings;
	if args.async_session {
		let agent_dir = config::agent_dir_from(args.agent_dir.clone(), |key| env::var_os(key))
			.filter(|p| p.is_dir());
		return run_async_session(
			&args,
			&workspace,
			settings,
			agent_dir,
			resolved.api,
			resolved.max_output_tokens,
			resolved.plugins,
			resolved.mcp,
			resolved.lsp,
			resolved.dap,
		);
	}
	let configured_browser = browser_options(&args);
	let max_steps = settings.max_steps;
	let model =
		configured_model(settings, resolved.api, resolved.max_output_tokens, args.request_usage)?;
	let model_ref: &dyn Model = &model;
	let user_agent_dir =
		config::agent_dir_from(args.agent_dir, |key| env::var_os(key)).filter(|path| path.is_dir());
	let mut session =
		crate::agent_session::AgentSession::open(&workspace, &args.session, args.leaf.as_deref())?;
	session.configure_plugins(&resolved.plugins, &Default::default())?;
	session.configure_mcp(&resolved.mcp, &Default::default())?;
	session.configure_language_services(resolved.lsp, resolved.dap)?;
	if let Some(options) = configured_browser {
		session.configure_browser(options);
	}
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

fn browser_options(args: &RunArgs) -> Option<harvest_browser::Options> {
	Some(harvest_browser::Options {
		chrome:     args.browser_chrome.clone()?,
		profile:    args.browser_profile.clone()?,
		headless:   args.browser_headless,
		no_sandbox: false,
	})
}

fn run_async_session(
	args: &RunArgs,
	workspace: &Path,
	settings: config::Settings,
	user_agent_dir: Option<PathBuf>,
	api: crate::configuration::Api,
	max_output_tokens: u32,
	plugins: Vec<crate::plugins::Configuration>,
	mcp: Vec<crate::mcp::Configuration>,
	lsp: Vec<crate::language_services::LspConfiguration>,
	dap: Vec<crate::language_services::DapConfiguration>,
) -> Result<u8> {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(2)
		.enable_all()
		.build()?;
	runtime.block_on(async {
		let max_model_turns = Some(settings.max_steps);
		let session = SessionEngine::open(
			workspace,
			&args.session,
			args.leaf.as_deref(),
			Arc::new(configured_model(settings, api, max_output_tokens, args.request_usage)?),
			Arc::new(crate::tools::TerminalApproval),
			SessionEngineOptions {
				browser: browser_options(args),
				max_model_turns,
				plugins,
				mcp,
				lsp,
				dap,
				user_agent_dir,
				tags: args.tag.iter().cloned().collect(),
				..Default::default()
			},
		)
		.await?;
		let mut subscription = session.subscribe().await?;
		let ticket = session.submit(args.prompt.clone()).await?;
		let abort = ticket.handle.clone();
		let completion = ticket.wait();
		let interrupt = tokio::signal::ctrl_c();
		tokio::pin!(completion, interrupt);
		let mut interrupted = false;
		let mut output = io::stdout().lock();
		let outcome: Result<_> = loop {
			tokio::select! {
				result = &mut completion => break Ok(result),
				event = subscription.events.recv(), if args.events => {
					let written = match event {
						Ok(event) => serde_json::to_writer(&mut output, event.as_ref())
							.map_err(anyhow::Error::from)
							.and_then(|()| { writeln!(output)?; output.flush()?; Ok(()) }),
						Err(error) => Err(anyhow::anyhow!("async session event stream interrupted: {error}")),
					};
					if let Err(error) = written { break Err(error); }
				},
				signal = &mut interrupt, if !interrupted => {
					if let Err(error) = signal { break Err(error.into()); }
					interrupted = true;
					abort.abort();
				},
			}
		};
		if outcome.is_err() {
			abort.abort();
			let _ = completion.await;
		}
		session.shutdown().await?;
		if args.events {
			loop {
				match subscription.events.try_recv() {
					Ok(event) => {
						serde_json::to_writer(&mut output, event.as_ref())?;
						writeln!(output)?;
					},
					Err(
						tokio::sync::broadcast::error::TryRecvError::Empty
						| tokio::sync::broadcast::error::TryRecvError::Closed,
					) => break,
					Err(error) => bail!("async session event stream interrupted: {error}"),
				}
			}
			output.flush()?;
		}
		let answer = outcome??;
		if !args.events {
			writeln!(output, "{answer}")?;
		}
		Ok(0)
	})
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
