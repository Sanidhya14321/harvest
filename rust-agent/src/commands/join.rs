//! `harvest join <link>` (`commands/join.ts`).
use std::io::IsTerminal;

use anyhow::Result;

#[derive(Debug, clap::Args)]
#[command(after_help = "Examples:\n  harvest join \"relay.example.sh/abc123#key\"")]
pub struct JoinArgs {
	/// Collab link shared by the host (/collab)
	pub link: String,
}

pub fn execute(args: JoinArgs) -> Result<u8> {
	if args.link.trim().is_empty() {
		eprintln!("Usage: harvest join <link>");
		return Ok(1);
	}
	if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
		eprintln!("harvest join requires an interactive terminal");
		return Ok(1);
	}
	eprintln!("harvest join: the interactive collab TUI is not ported to the native application yet");
	Ok(1)
}
