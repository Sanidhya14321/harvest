//! `harvest shell` (`commands/shell.ts` + `cli/shell-cli.ts`): interactive
//! console over a persistent brush-core shell session.
use std::path::PathBuf;

use anyhow::Result;

#[derive(Debug, clap::Args)]
pub struct ShellArgs {
	/// Set working directory for commands
	#[arg(short = 'C', long)]
	pub cwd:         Option<PathBuf>,
	/// Timeout per command in milliseconds
	#[arg(short, long)]
	pub timeout:     Option<u32>,
	/// Skip sourcing snapshot from user shell
	#[arg(long)]
	pub no_snapshot: bool,
}

#[cfg(not(feature = "full"))]
pub fn execute(_args: ShellArgs) -> Result<u8> {
	anyhow::bail!("shell console requires the full Rust profile")
}

#[cfg(feature = "full")]
fn print_help(p: &super::Paint) {
	print!(
		"{}\n\n{}\n  .help           Show this help\n  .exit, exit     Exit the console\n\n{}\n  \
		 --cwd, -C <path>     Set working directory for commands\n  --timeout, -t <ms>   Timeout \
		 per command in milliseconds\n  --no-snapshot        Skip sourcing snapshot from user \
		 shell\n\n{}\n  Runs in a persistent brush-core shell session.\n  Variables and functions \
		 defined in one command persist for the next.\n\n",
		p.bold("Shell Console Commands"),
		p.bold("Special Commands:"),
		p.bold("Options:"),
		p.bold("Notes:"),
	);
}

#[cfg(feature = "full")]
pub fn execute(args: ShellArgs) -> Result<u8> {
	use std::{
		io::{BufRead, IsTerminal, Write},
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
		},
	};

	if !std::io::stdin().is_terminal() {
		eprintln!("Error: shell console requires an interactive TTY.");
		return Ok(1);
	}
	let cwd = match args.cwd {
		Some(dir) => std::path::absolute(dir)?,
		None => std::env::current_dir()?,
	};
	// Snapshot sourcing (`getOrCreateSnapshot`) and the settings-driven minimizer
	// are not ported; the session always runs as if `--no-snapshot` were set.
	let _ = args.no_snapshot;
	let out = super::Paint::stdout();
	let err = super::Paint::stderr();
	let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
	let shell = Arc::new(pi_shell::Shell::new(None));
	let active = Arc::new(AtomicBool::new(false));
	{
		let shell = shell.clone();
		let active = active.clone();
		runtime.spawn(async move {
			while tokio::signal::ctrl_c().await.is_ok() {
				if active.load(Ordering::SeqCst) {
					shell.abort().await;
				} else {
					std::process::exit(0);
				}
			}
		});
	}
	println!("{}", out.dim("Type .help for commands."));
	let prompt = out.cyan("harvest shell> ");
	let stdin = std::io::stdin();
	let mut line = String::new();
	loop {
		print!("{prompt}");
		std::io::stdout().flush()?;
		line.clear();
		if stdin.lock().read_line(&mut line)? == 0 {
			break;
		}
		let command = line.trim();
		if command.is_empty() {
			continue;
		}
		if command == ".help" {
			print_help(&out);
			continue;
		}
		if matches!(command, ".exit" | "exit" | "quit") {
			break;
		}
		active.store(true, Ordering::SeqCst);
		let (result, last) = runtime.block_on(async {
			let (sender, receiver) = flume::bounded::<String>(64);
			let printer = tokio::spawn(async move {
				let mut last = None;
				while let Ok(chunk) = receiver.recv_async().await {
					if let Some(c) = chunk.chars().last() {
						last = Some(c);
					}
					let mut stdout = std::io::stdout().lock();
					let _ = stdout.write_all(chunk.as_bytes());
					let _ = stdout.flush();
				}
				last
			});
			let result = shell
				.run(
					pi_shell::ShellRunOptions {
						command:    command.to_owned(),
						cwd:        Some(cwd.to_string_lossy().into_owned()),
						env:        None,
						timeout_ms: args.timeout,
					},
					Some(sender),
					harvest_runtime::cancel::CancelToken::new(args.timeout),
				)
				.await;
			(result, printer.await.ok().flatten())
		});
		active.store(false, Ordering::SeqCst);
		if last.is_some_and(|c| c != '\n') {
			println!();
		}
		match result {
			Ok(r) if r.timed_out => eprint!("{}", err.yellow("Command timed out.\n")),
			Ok(r) if r.cancelled => eprint!("{}", err.yellow("Command cancelled.\n")),
			Ok(r) => {
				if let Some(code) = r.exit_code.filter(|c| *c != 0) {
					eprint!("{}", err.yellow(&format!("Exit code: {code}\n")));
				}
			},
			Err(e) => eprint!("{}", err.red(&format!("Error: {e}\n"))),
		}
	}
	Ok(0)
}
