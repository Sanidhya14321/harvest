//! `harvest launch` (root command, `commands/launch.ts`).
//!
//! The interactive TUI is not ported; the non-TUI path forwards the remaining
//! arguments to the native one-shot `run` command.
use std::ffi::OsString;

use anyhow::{Result, bail};

pub const ACP_TERMINAL_AUTH_FLAG: &str = "--acp-terminal-auth";

#[derive(Debug, clap::Args)]
pub struct LaunchArgs {
	/// Arguments forwarded to the agent (prompt and run options)
	#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
	pub args: Vec<OsString>,
}

/// `prepareAcpTerminalAuthArgs`: strip the terminal-auth flag and, when it was
/// present, any `--mode` selection.
pub fn prepare_acp_terminal_auth_args(raw: &[OsString]) -> (Vec<OsString>, bool) {
	let terminal_auth = raw.iter().any(|a| a == ACP_TERMINAL_AUTH_FLAG);
	let without: Vec<OsString> =
		raw.iter().filter(|a| *a != ACP_TERMINAL_AUTH_FLAG).cloned().collect();
	if !terminal_auth {
		return (without, false);
	}
	let mut out = Vec::new();
	let mut iter = without.into_iter();
	while let Some(arg) = iter.next() {
		if arg == "--mode" {
			iter.next();
			continue;
		}
		if arg.to_string_lossy().starts_with("--mode=") {
			continue;
		}
		out.push(arg);
	}
	(out, true)
}

/// Run the root launch flow. `run` re-dispatches args through the native
/// `run` subcommand.
pub fn execute(args: LaunchArgs, run: impl FnOnce(Vec<OsString>) -> Result<u8>) -> Result<u8> {
	let (args, _) = prepare_acp_terminal_auth_args(&args.args);
	if args.is_empty() {
		bail!(
			"the interactive TUI is not ported to the native application yet; pass a prompt to run \
			 it non-interactively (harvest launch <prompt>) or use harvest rpc"
		);
	}
	run(args)
}
