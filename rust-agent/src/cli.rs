use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use clap_mangen::Man;

use crate::config::{self, Overrides};
use crate::provider::{HttpModel, Model};
use crate::{agent, shell};

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
    #[command(about = "Print or install shell completions")]
    Completions(CompletionArgs),
    #[command(about = "Print the manual page")]
    Man,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    #[arg(short, long)]
    prompt: String,
    #[arg(short, long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    max_steps: Option<u32>,
}

#[derive(Debug, clap::Args)]
struct CompletionArgs {
    shell: Option<String>,
    #[arg(long)]
    install: bool,
    #[arg(long)]
    automatic: bool,
}

pub(crate) fn execute() -> Result<u8> {
    let parsed = Cli::try_parse();
    let installing = parsed
        .as_ref()
        .is_ok_and(|cli| matches!(&cli.command, Commands::Completions(args) if args.install));
    if !installing {
        shell::auto_install();
    }
    match parsed {
        Ok(cli) => dispatch(cli),
        Err(error) => {
            error.print().context("failed to print the command line")?;
            Ok(u8::try_from(error.exit_code()).unwrap_or(1))
        }
    }
}

fn dispatch(cli: Cli) -> Result<u8> {
    match cli.command {
        Commands::Run(args) => run(args),
        Commands::Tools => tools(),
        Commands::Completions(args) => completions(args),
        Commands::Man => manual(),
    }
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
            model: args.model,
            base_url: args.base_url,
            api_key: args.api_key,
            max_steps: args.max_steps,
        },
        &environment,
    )?;
    let max_steps = settings.max_steps;
    let model = HttpModel::new(settings);
    let model_ref: &dyn Model = &model;
    let answer = agent::run(&workspace, &args.prompt, max_steps, model_ref)?;
    writeln!(io::stdout(), "{answer}")?;
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
    let data =
        env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), PathBuf::from);
    let state = env::var_os("XDG_STATE_HOME")
        .map_or_else(|| home.join(".local/state"), PathBuf::from)
        .join("omp");
    // nosemgrep: rust.lang.security.current-exe.current-exe
    let executable = env::current_exe().context("failed to locate the omp executable")?;
    Ok(shell::Layout {
        home,
        data,
        state,
        executable,
    })
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
