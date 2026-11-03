//! `harvest auth-gateway` (`commands/auth-gateway.ts` + `cli/auth-gateway-cli.ts`).
//!
//! Ported: `token`, `status`. `serve` validates its bind policy (loopback-only
//! `--no-auth`) and then fails as not yet ported; `check` is not yet ported.

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use super::{
	Paint,
	auth_broker::{
		broker_request, ensure_token, generate_token, read_token, resolve_broker_config,
		token_path, write_token,
	},
};

const DEFAULT_BIND: &str = "127.0.0.1:4000";
const TOKEN_FILE: &str = "auth-gateway.token";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Action {
	Serve,
	Token,
	Status,
	Check,
}

#[derive(Args, Debug)]
pub struct AuthGatewayArgs {
	/// Sub-command
	#[arg(value_enum)]
	action:     Action,
	/// Output JSON (token/status/check)
	#[arg(long)]
	json:       bool,
	/// Bind address for `serve` (host:port)
	#[arg(short, long)]
	bind:       Option<String>,
	/// Regenerate the gateway bearer token (token)
	#[arg(long)]
	regenerate: bool,
	/// Disable inbound bearer-token auth (serve; loopback binds only)
	#[arg(long = "no-auth")]
	no_auth:    bool,
	/// For `check`: also probe each credential with a chat-completion request
	#[arg(long)]
	strict:     bool,
}

fn bind_host(bind: &str) -> &str {
	let host = match bind.rsplit_once(':') {
		Some((h, _)) => h,
		None => bind,
	};
	host.trim_start_matches('[').trim_end_matches(']')
}

fn is_loopback_host(host: &str) -> bool {
	host == "localhost"
		|| host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub fn execute(args: AuthGatewayArgs) -> Result<u8> {
	match args.action {
		Action::Token => token(&args),
		Action::Status => status(&args),
		Action::Serve => serve(&args),
		Action::Check => {
			let _ = args.strict;
			bail!(
				"`harvest auth-gateway check` is not yet ported to the native application; use the \
				 TypeScript CLI"
			)
		},
	}
}

fn token(args: &AuthGatewayArgs) -> Result<u8> {
	let path = token_path(TOKEN_FILE)?;
	let token = if args.regenerate {
		let t = generate_token()?;
		write_token(&path, &t)?;
		t
	} else {
		ensure_token(&path)?
	};
	if args.json {
		println!("{}", json!({"token": token, "path": path.display().to_string()}));
	} else {
		println!("{token}");
	}
	Ok(0)
}

fn serve(args: &AuthGatewayArgs) -> Result<u8> {
	if resolve_broker_config()?.is_none() {
		bail!(
			"`harvest auth-gateway serve` requires HARVEST_AUTH_BROKER_URL. The gateway is itself \
			 a broker client."
		);
	}
	let bind = args.bind.as_deref().unwrap_or(DEFAULT_BIND);
	let host = bind_host(bind);
	if args.no_auth && !is_loopback_host(host) {
		bail!(
			"--no-auth is only permitted when binding to loopback interfaces (e.g. 127.0.0.1 or \
			 localhost), got '{host}'."
		);
	}
	bail!(
		"`harvest auth-gateway serve` is not yet ported to the native application; use the \
		 TypeScript CLI"
	)
}

fn status(args: &AuthGatewayArgs) -> Result<u8> {
	let paint = Paint::stdout();
	let token_file = token_path(TOKEN_FILE)?;
	let present = read_token(&token_file)?.is_some();
	let tf = token_file.display().to_string();
	let token_line = |p: bool| {
		format!(
			"token: {} at {tf}",
			if p { paint.green("present") } else { paint.red("missing") }
		)
	};
	let Some(cfg) = resolve_broker_config()? else {
		if args.json {
			println!(
				"{}",
				json!({"ready": false, "reason": "not_configured", "tokenFile": tf,
					"tokenPresent": present, "broker": null, "brokerConfigured": false,
					"brokerAuthenticated": false})
			);
		} else {
			println!("{} Set HARVEST_AUTH_BROKER_URL.", paint.yellow("No broker configured."));
			println!("{}", token_line(present));
		}
		return Ok(1);
	};
	match broker_request(&cfg, "GET", "/v1/snapshot", true, None) {
		Ok(snapshot) => {
			let count = snapshot
				.get("credentials")
				.and_then(Value::as_array)
				.map_or(0, Vec::len);
			if args.json {
				println!(
					"{}",
					json!({"ready": present, "reason": if present { Value::Null } else { "token_missing".into() },
						"tokenFile": tf, "tokenPresent": present, "broker": cfg.url,
						"brokerConfigured": true, "brokerAuthenticated": true, "credentialCount": count})
				);
			} else {
				let plural = if count == 1 { "" } else { "s" };
				let head = if present { paint.green("ready") } else { paint.yellow("not ready") };
				println!("{head} upstream broker: {} ({count} credential{plural})", cfg.url);
				println!("{}", token_line(present));
				if !present {
					println!(
						"Run `harvest auth-gateway token` or `harvest auth-gateway serve` to create a \
						 bearer token."
					);
				}
			}
			Ok(u8::from(!present))
		},
		Err(err) => {
			if args.json {
				println!(
					"{}",
					json!({"ready": false, "reason": "broker_unavailable", "tokenFile": tf,
						"tokenPresent": present, "broker": cfg.url, "brokerConfigured": true,
						"brokerAuthenticated": false, "error": err.to_string()})
				);
			} else {
				println!("{} upstream broker: {}: {err}", paint.red("FAILED"), cfg.url);
				println!("{}", token_line(present));
			}
			Ok(1)
		},
	}
}
