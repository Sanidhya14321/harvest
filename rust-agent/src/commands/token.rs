//! `harvest token` (`packages/coding-agent/src/commands/token.ts`).

use std::io::Write;

use anyhow::Result;
use clap::Args as ClapArgs;

use crate::auth_storage::AuthStorage;

#[derive(Debug, ClapArgs)]
pub struct Args {
	/// Provider ID (e.g. anthropic, openai)
	provider:      String,
	/// Output the raw credential value without parsing nested JSON structures
	#[arg(long)]
	raw:           bool,
	/// Force refresh the OAuth token even if it has not expired (not yet native)
	#[arg(long = "force-refresh")]
	force_refresh: bool,
	/// Select the Nth OAuth account (1-based) in stored order
	#[arg(short = 'a', long)]
	account:       Option<i64>,
	/// List the provider's OAuth accounts (index + identity) and exit
	#[arg(short = 'l', long)]
	list:          bool,
}

pub fn execute(args: Args) -> Result<u8> {
	let name = args.provider.as_str();
	if name.starts_with("mcp_oauth:") || name.eq_ignore_ascii_case("perplexity") {
		eprintln!("Provider \"{name}\" requires a flow not yet ported to the native application.");
		return Ok(1);
	}
	if args.force_refresh {
		eprintln!("--force-refresh: OAuth refresh is not yet ported to the native application.");
		return Ok(1);
	}
	let provider = name.to_lowercase();
	let store = AuthStorage::discover()?;
	let mut out = std::io::stdout().lock();
	if args.list || args.account.is_some() {
		let accounts = store.oauth_accounts(&provider)?;
		if accounts.is_empty() {
			eprintln!("No OAuth accounts found for provider \"{name}\".");
			eprintln!("--account/--list select among OAuth accounts; this provider has none stored.");
			return Ok(1);
		}
		if args.list {
			for acct in &accounts {
				let base = acct
					.email
					.clone()
					.or_else(|| acct.account_id.clone())
					.or_else(|| acct.project_id.clone())
					.or_else(|| acct.enterprise.clone())
					.unwrap_or_else(|| format!("credential #{}", acct.credential_id));
				let label = match acct.org_name.as_ref().or(acct.org_id.as_ref()) {
					Some(org) if *org != base => format!("{base} ({org})"),
					_ => base,
				};
				writeln!(out, "{}. {label}", acct.position + 1)?;
			}
			return Ok(0);
		}
		let count = accounts.len();
		let n = args.account.unwrap_or(0);
		if n < 1 || usize::try_from(n).unwrap_or(usize::MAX) > count {
			eprintln!(
				"Invalid --account {n}. Provider \"{name}\" has {count} OAuth account(s) (1-{count})."
			);
			return Ok(1);
		}
		let index = usize::try_from(n - 1).unwrap_or_default();
		return match store.oauth_access_at(&provider, index)? {
			Some(token) => {
				writeln!(out, "{token}")?;
				Ok(0)
			},
			None => {
				eprintln!(
					"Could not get token for account {n} of \"{name}\": no OAuth credential available"
				);
				Ok(1)
			},
		};
	}

	let env = crate::catalog::provider_env_vars(&provider)
		.iter()
		.find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()));
	let Some(key) = store.api_key(&provider, env)?.filter(|k| !k.is_empty()) else {
		eprintln!("No active credential found for provider \"{name}\".");
		let active = store.providers()?;
		if !active.is_empty() {
			eprintln!("Configured providers: {}", active.join(", "));
		}
		return Ok(1);
	};
	if !args.raw
		&& let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&key)
		&& let Some(token) = map.get("token").and_then(serde_json::Value::as_str)
	{
		writeln!(out, "{token}")?;
		return Ok(0);
	}
	writeln!(out, "{key}")?;
	Ok(0)
}
