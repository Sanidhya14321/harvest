//! `harvest git` (`commands/git.ts`): fullscreen git TUI.
use std::{io::IsTerminal, path::PathBuf};

use anyhow::Result;

#[derive(Debug, clap::Args)]
#[command(
	after_help = "Examples:\n  harvest git\n  harvest git HEAD~2\n  harvest git -C ~/projects/app"
)]
pub struct GitArgs {
	/// Pin the view to one commit (any revision, e.g. HEAD~2 or a sha)
	pub revision: Option<String>,
	/// Run in another directory instead of the current one
	#[arg(short = 'C', long)]
	pub dir:      Option<PathBuf>,
}

pub fn execute(args: GitArgs) -> Result<u8> {
	if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
		eprintln!("harvest git is interactive and requires a TTY");
		return Ok(1);
	}
	let _ = (args.revision, args.dir);
	eprintln!("harvest git: the split-pane git TUI is not ported to the native application yet");
	Ok(1)
}
