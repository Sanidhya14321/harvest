//! `harvest acp` (`commands/acp.ts`): ACP server over stdio.
use anyhow::{Result, bail};

use super::launch::{LaunchArgs, prepare_acp_terminal_auth_args};

pub fn execute(
	args: LaunchArgs,
	run: impl FnOnce(Vec<std::ffi::OsString>) -> Result<u8>,
) -> Result<u8> {
	let (rest, terminal_auth) = prepare_acp_terminal_auth_args(&args.args);
	if terminal_auth {
		// Terminal auth opens the interactive launch flow instead of ACP mode.
		return super::launch::execute(LaunchArgs { args: rest }, run);
	}
	bail!(
		"ACP (Agent Client Protocol) server mode is not ported to the native application yet; use \
		 harvest rpc for the native JSONL protocol"
	)
}
