//! `harvest install <target>` — port of `commands/install.ts`: local paths
//! route to `plugin link`, everything else to `plugin install`.

use std::path::Path;

use anyhow::Result;
use clap::Args as ClapArgs;

use super::{
	plugin::{Action, Flags, Scope, run},
	plugin_store::is_drive_path,
};

#[derive(Debug, ClapArgs)]
#[command(about = "Install a plugin from a local path, npm spec, or marketplace ref")]
pub struct InstallArgs {
	/// Local path, npm spec, or marketplace ref (e.g. ./my-ext, my-pkg@1.2.3,
	/// name@marketplace)
	pub targets: Vec<String>,
	/// Output JSON
	#[arg(long)]
	pub json:    bool,
	/// Force install
	#[arg(long)]
	pub force:   bool,
	/// Show actions without applying changes
	#[arg(long = "dry-run")]
	pub dry_run: bool,
	/// Install scope: "user" (default) or "project" (marketplace installs only)
	#[arg(long, value_enum)]
	pub scope:   Option<Scope>,
}

/// `looksLikeLocalPath`.
fn looks_like_local_path(target: &str) -> bool {
	target.starts_with(['.', '/', '~']) || is_drive_path(target) || Path::new(target).exists()
}

pub fn execute(args: InstallArgs) -> Result<u8> {
	if args.targets.is_empty() {
		eprintln!("Usage: harvest install <path | npm-spec | name@marketplace> [...]");
		return Ok(1);
	}
	let flags = Flags {
		json: args.json,
		force: args.force,
		dry_run: args.dry_run,
		scope: args.scope,
		..Flags::default()
	};
	let (local, remote): (Vec<String>, Vec<String>) =
		args.targets.into_iter().partition(|t| looks_like_local_path(t));
	for path in local {
		let code = run(Action::Link, &[path], &flags)?;
		if code != 0 {
			return Ok(code);
		}
	}
	if !remote.is_empty() {
		return run(Action::Install, &remote, &flags);
	}
	Ok(0)
}
