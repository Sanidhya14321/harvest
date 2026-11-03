//! `harvest setup [python|speech]` (`commands/setup.ts` + `cli/setup-cli.ts`).
//!
//! `python` reports the retired Python evaluation backend. The onboarding
//! wizard and speech model download are not yet ported.

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Component {
	Python,
	Speech,
}

#[derive(Args, Debug)]
pub struct SetupArgs {
	/// Optional component to install
	#[arg(value_enum)]
	component: Option<Component>,
	/// Check if dependencies are installed
	#[arg(short, long)]
	check:     bool,
	/// Output status as JSON
	#[arg(long)]
	json:      bool,
}

pub fn execute(args: SetupArgs) -> Result<u8> {
	let Some(component) = args.component else {
		if args.check || args.json {
			bail!("setup --check/--json requires a COMPONENT (python|speech)");
		}
		bail!("the onboarding setup wizard is not yet ported to the native application");
	};
	match component {
		Component::Python => {
			let reason = crate::tools::PYTHON_EVAL_RETIRED.trim();
			if args.json {
				println!(
					"{}",
					serde_json::json!({"available": false, "retired": true, "reason": reason})
				);
			} else {
				eprintln!("{reason}");
			}
			Ok(1)
		},
		Component::Speech => bail!(
			"`harvest setup speech` is not yet ported to the native application; use the \
			 TypeScript CLI"
		),
	}
}
