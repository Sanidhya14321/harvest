use std::{
	io::{self, BufRead, IsTerminal, Read, Write},
	path::PathBuf,
};

use anyhow::{Result, ensure};

use crate::{
	chat::{FunctionCall, ToolCall},
	config, configuration,
	tools::TerminalApproval,
};

#[derive(Debug, clap::Args)]
pub(crate) struct Arguments {
	#[arg(short, long, default_value = ".")]
	workspace:   PathBuf,
	#[arg(long)]
	agent_dir:   Option<PathBuf>,
	#[arg(long)]
	profile:     Option<String>,
	#[arg(long = "config")]
	overlays:    Vec<PathBuf>,
	#[arg(long, help = "JSON tool arguments, including action and server or adapter selection")]
	arguments:   String,
	#[arg(
		long,
		help = "Keep service ownership while entering additional JSON requests in this terminal"
	)]
	interactive: bool,
}
pub(crate) fn execute(tool: &str, arguments: Arguments) -> Result<u8> {
	ensure!(
		arguments.arguments.len() <= crate::process_transport::MAX_FRAME_BYTES,
		"language service arguments exceed limit"
	);
	let root = arguments.workspace.canonicalize()?;
	let environment = config::environment_from(|key| std::env::var(key).ok())?;
	let resolved =
		configuration::load(&root, &Default::default(), &environment, &configuration::LoadOptions {
			user_directory: config::agent_dir_from(arguments.agent_dir, |key| std::env::var_os(key)),
			profile: arguments.profile,
			environment_profile: std::env::var("OMP_PROFILE").ok(),
			overlays: arguments.overlays,
			..Default::default()
		})?;
	let mut runtime = super::Runtime::default();
	runtime.configure(&root, resolved.lsp, resolved.dap)?;
	let mut request = arguments.arguments;
	if arguments.interactive {
		ensure!(io::stdin().is_terminal(), "interactive service mode requires a terminal");
	}
	loop {
		let call = ToolCall {
			id:       "cli".into(),
			kind:     "function".into(),
			function: FunctionCall { name: tool.into(), arguments: request },
		};
		let output = runtime.invoke(&call, &TerminalApproval, &Default::default());
		match output {
			Ok(output) => writeln!(io::stdout().lock(), "{output}")?,
			Err(error) if arguments.interactive => writeln!(io::stderr().lock(), "{error:#}")?,
			Err(error) => return Err(error),
		}
		if !arguments.interactive {
			break;
		}
		write!(io::stderr().lock(), "{tool}> ")?;
		io::stderr().flush()?;
		let mut line = Vec::new();
		let count = io::stdin()
			.lock()
			.take(crate::process_transport::MAX_FRAME_BYTES as u64 + 1)
			.read_until(b'\n', &mut line)?;
		ensure!(
			count <= crate::process_transport::MAX_FRAME_BYTES,
			"language-service input exceeds frame limit"
		);
		if count == 0 {
			break;
		}
		request = String::from_utf8(line)
			.map_err(|_| anyhow::anyhow!("language-service input is not UTF-8"))?;
		if matches!(request.trim(), "quit" | "exit") {
			break;
		}
	}
	Ok(0)
}
