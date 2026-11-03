//! `harvest auth-broker` (`commands/auth-broker.ts` + `cli/auth-broker-cli.ts`).
//!
//! Ported: `token`, `logout`, `import`, `status`. Not yet ported (fail with a
//! pointer to the TypeScript app): `serve`, `login`, `list`, `migrate`.

use std::{
	io::{BufRead, Read, Write},
	path::{Path, PathBuf},
	time::Duration,
};

use anyhow::{Context, Result, bail};
use base64::Engine;
use clap::{Args, ValueEnum};
use serde_json::{Map, Value, json};

use super::Paint;
use crate::auth_storage::AuthStorage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Action {
	Serve,
	Token,
	Login,
	Logout,
	Import,
	Migrate,
	Status,
	List,
}

#[derive(Args, Debug)]
pub struct AuthBrokerArgs {
	/// Sub-command
	#[arg(value_enum)]
	action:           Action,
	/// OAuth provider id (login/logout) or path (import)
	source:           Option<String>,
	/// Output JSON
	#[arg(long)]
	json:             bool,
	/// Bind address for `serve` (host:port)
	#[arg(short, long)]
	bind:             Option<String>,
	/// Regenerate the bearer token
	#[arg(long)]
	regenerate:       bool,
	/// SSH user@host for remote login (login --via=user@host)
	#[arg(long)]
	via:              Option<String>,
	/// Override provider id for `import`
	#[arg(long)]
	provider:         Option<String>,
	/// Import credentials whose JSON has `disabled: true` (import)
	#[arg(long = "include-disabled")]
	include_disabled: bool,
	/// migrate source: local SQLite + env vars
	#[arg(long = "from-local")]
	from_local:       bool,
	/// Capture env-var API keys for providers not yet on broker (migrate)
	#[arg(long = "include-env")]
	include_env:      bool,
	/// Also upload OAuth from local SQLite during migrate
	#[arg(long = "include-oauth")]
	include_oauth:    bool,
	/// Print actions without executing (import / login --via / migrate)
	#[arg(long = "dry-run")]
	dry_run:          bool,
}

const MAX_BODY: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Shared token-file and broker-client helpers (also used by auth-gateway).
// ---------------------------------------------------------------------------

pub(crate) fn token_path(name: &str) -> Result<PathBuf> {
	Ok(super::root_dir()?.join(name))
}

pub(crate) fn read_token(path: &Path) -> Result<Option<String>> {
	match std::fs::read_to_string(path) {
		Ok(raw) => {
			let t = raw.trim();
			Ok((!t.is_empty()).then(|| t.to_owned()))
		},
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
	}
}

fn open_options(exclusive: bool) -> std::fs::OpenOptions {
	let mut o = std::fs::OpenOptions::new();
	o.write(true);
	if exclusive {
		o.create_new(true);
	} else {
		o.create(true).truncate(true);
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		o.mode(0o600);
	}
	o
}

fn ensure_parent(path: &Path) -> Result<()> {
	if let Some(dir) = path.parent() {
		std::fs::create_dir_all(dir)?;
	}
	Ok(())
}

pub(crate) fn write_token(path: &Path, token: &str) -> Result<()> {
	ensure_parent(path)?;
	open_options(false).open(path)?.write_all(token.as_bytes())?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
	}
	Ok(())
}

pub(crate) fn generate_token() -> Result<String> {
	let mut bytes = [0u8; 32];
	std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
	Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// `ensureToken` with an O_EXCL create so concurrent callers converge.
pub(crate) fn ensure_token(path: &Path) -> Result<String> {
	if let Some(t) = read_token(path)? {
		return Ok(t);
	}
	let token = generate_token()?;
	ensure_parent(path)?;
	match open_options(true).open(path) {
		Ok(mut f) => {
			f.write_all(token.as_bytes())?;
			Ok(token)
		},
		Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match read_token(path)? {
			Some(t) => Ok(t),
			None => {
				write_token(path, &token)?;
				Ok(token)
			},
		},
		Err(e) => Err(e.into()),
	}
}

pub(crate) struct BrokerConfig {
	pub url:   String,
	pub token: String,
}

fn env(keys: &[&str]) -> Option<String> {
	keys.iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
}

/// `resolveAuthBrokerConfig` (env + token file; `config.yml` keys not ported).
pub(crate) fn resolve_broker_config() -> Result<Option<BrokerConfig>> {
	let Some(url) = env(&["HARVEST_AUTH_BROKER_URL", "OMP_AUTH_BROKER_URL"]) else {
		return Ok(None);
	};
	let token = match env(&["HARVEST_AUTH_BROKER_TOKEN", "OMP_AUTH_BROKER_TOKEN"]) {
		Some(t) => t,
		None => read_token(&token_path("auth-broker.token")?)?.with_context(|| {
			format!(
				"auth broker {url} is configured but no token was found (set HARVEST_AUTH_BROKER_TOKEN)"
			)
		})?,
	};
	Ok(Some(BrokerConfig { url: url.trim_end_matches('/').to_owned(), token }))
}

/// Bounded JSON request against the broker. Never logs the bearer.
pub(crate) fn broker_request(
	cfg: &BrokerConfig,
	method: &str,
	path: &str,
	auth: bool,
	body: Option<&Value>,
) -> Result<Value> {
	let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
	let mut req = agent.request(method, &format!("{}{path}", cfg.url));
	if auth {
		req = req.set("Authorization", &format!("Bearer {}", cfg.token));
	}
	let resp = match body {
		Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
		None => req.call(),
	};
	let resp = match resp {
		Ok(r) => r,
		Err(ureq::Error::Status(code, r)) => {
			let text = harvest_runtime::read_bounded(r.into_reader(), 4096).unwrap_or_default();
			let text = String::from_utf8_lossy(&text);
			bail!("auth broker {method} {path} failed: HTTP {code} {}", text.trim());
		},
		Err(e) => bail!("auth broker {method} {path} failed: {e}"),
	};
	let bytes = harvest_runtime::read_bounded(resp.into_reader(), MAX_BODY)?;
	serde_json::from_slice(&bytes).context("auth broker returned invalid JSON")
}

fn not_ported(what: &str) -> Result<u8> {
	bail!(
		"`harvest auth-broker {what}` is not yet ported to the native application; use the \
		 TypeScript CLI"
	)
}

pub fn execute(args: AuthBrokerArgs) -> Result<u8> {
	// Flags accepted for CLI compatibility with the TS command.
	let _ = (&args.bind, &args.via, args.from_local, args.include_env, args.include_oauth);
	match args.action {
		Action::Token => token(&args),
		Action::Logout => logout(&args),
		Action::Import => import(&args),
		Action::Status => status(&args),
		Action::Serve => not_ported("serve"),
		Action::Login => not_ported("login"),
		Action::List => not_ported("list"),
		Action::Migrate => not_ported("migrate"),
	}
}

fn token(args: &AuthBrokerArgs) -> Result<u8> {
	let path = token_path("auth-broker.token")?;
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

fn prompt_pick(title: &str, items: &[String]) -> Result<String> {
	println!("{title}\n");
	for (i, p) in items.iter().enumerate() {
		println!("  {}. {p}", i + 1);
	}
	print!("\nEnter number (1-{}): ", items.len());
	std::io::stdout().flush()?;
	let mut line = String::new();
	std::io::stdin().lock().read_line(&mut line)?;
	let choice = line.trim();
	match choice.parse::<usize>() {
		Ok(n) if n >= 1 && n <= items.len() => Ok(items[n - 1].clone()),
		_ => bail!("Invalid selection: {choice}"),
	}
}

fn logout(args: &AuthBrokerArgs) -> Result<u8> {
	let store = AuthStorage::discover()?;
	let provider = match args.source.clone().or_else(|| args.provider.clone()) {
		Some(p) => p,
		None => {
			let stored = store.providers()?;
			if stored.is_empty() {
				println!("No credentials stored.");
				return Ok(0);
			}
			prompt_pick("Select a provider to logout:", &stored)?
		},
	};
	store.disable_provider(&provider, "logged out by user")?;
	println!("Logged out of {provider}");
	Ok(0)
}

const CLIPROXY_TYPES: &[(&str, &str)] = &[
	("claude", "anthropic"),
	("codex", "openai-codex"),
	("gemini", "google-gemini-cli"),
	("antigravity", "google-antigravity"),
	("gemini-cli", "google-gemini-cli"),
];

struct PlanEntry {
	file:       String,
	provider:   String,
	email:      Option<String>,
	account_id: Option<String>,
	expires_at: i64,
	disabled:   bool,
	credential: Map<String, Value>,
}

impl PlanEntry {
	fn describe(&self) -> String {
		let ident = self
			.email
			.as_deref()
			.or(self.account_id.as_deref())
			.unwrap_or("(no identity)");
		let stale =
			if self.expires_at < chrono::Utc::now().timestamp_millis() { " [expired]" } else { "" };
		let disabled = if self.disabled { " [disabled]" } else { "" };
		format!("{}: {ident}{stale}{disabled} from {}", self.provider, self.file)
	}
}

fn resolve_provider(
	json: &Map<String, Value>,
	file: &Path,
	override_id: Option<&str>,
) -> Option<String> {
	if let Some(id) = override_id.filter(|s| !s.is_empty()) {
		return Some(id.to_owned());
	}
	if let Some(t) = json.get("type").and_then(Value::as_str).map(|s| s.trim().to_lowercase())
		&& let Some((_, p)) = CLIPROXY_TYPES.iter().find(|(k, _)| *k == t)
	{
		return Some((*p).to_owned());
	}
	let base = file.file_stem()?.to_string_lossy().to_lowercase();
	CLIPROXY_TYPES
		.iter()
		.find(|(prefix, _)| base.starts_with(&format!("{prefix}-")) || base == *prefix)
		.map(|(_, p)| (*p).to_owned())
}

fn parse_expiry(raw: Option<&str>) -> Option<i64> {
	chrono::DateTime::parse_from_rfc3339(raw?)
		.ok()
		.map(|d| d.timestamp_millis())
}

fn collect_sources(target: &Path) -> Result<Vec<PathBuf>> {
	let meta =
		std::fs::metadata(target).with_context(|| format!("stat {}", target.display()))?;
	if meta.is_file() {
		return Ok(vec![target.to_path_buf()]);
	}
	if !meta.is_dir() {
		bail!("Import source is neither file nor directory: {}", target.display());
	}
	let mut files: Vec<PathBuf> = std::fs::read_dir(target)?
		.filter_map(Result::ok)
		.filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	files.sort();
	Ok(files)
}

fn read_json(file: &Path) -> Result<Value> {
	let f = std::fs::File::open(file)?;
	let bytes = harvest_runtime::read_bounded(f, harvest_runtime::MAX_FILE_BYTES)?;
	Ok(serde_json::from_slice(&bytes)?)
}

fn import(args: &AuthBrokerArgs) -> Result<u8> {
	let Some(target) = &args.source else {
		bail!(
			"Usage: harvest auth-broker import <file|dir> [--provider=<id>] [--include-disabled] \
			 [--dry-run]"
		);
	};
	let expanded = match target.strip_prefix('~') {
		Some(rest) => format!("{}{rest}", std::env::var("HOME").unwrap_or_default()),
		None => target.clone(),
	};
	let target = std::path::absolute(&expanded)?;

	let mut entries = Vec::new();
	let mut skipped: Vec<(String, String)> = Vec::new();
	for file in collect_sources(&target)? {
		let name = file.display().to_string();
		let json = match read_json(&file) {
			Ok(Value::Object(m)) => m,
			Ok(_) => {
				skipped.push((name, "unreadable JSON: not an object".into()));
				continue;
			},
			Err(e) => {
				skipped.push((name, format!("unreadable JSON: {e}")));
				continue;
			},
		};
		let disabled = json.get("disabled").and_then(Value::as_bool) == Some(true);
		if disabled && !args.include_disabled {
			skipped.push((
				name,
				"credential marked disabled (use --include-disabled to import anyway)".into(),
			));
			continue;
		}
		let Some(provider) = resolve_provider(&json, &file, args.provider.as_deref()) else {
			let t = json.get("type").and_then(Value::as_str).unwrap_or("?");
			skipped.push((
				name,
				format!("cannot determine provider from type={t} (pass --provider to override)"),
			));
			continue;
		};
		let s = |k: &str| {
			json.get(k)
				.and_then(Value::as_str)
				.filter(|v| !v.is_empty())
				.map(str::to_owned)
		};
		let (Some(access), Some(refresh)) = (s("access_token"), s("refresh_token")) else {
			skipped.push((name, "missing access_token or refresh_token".into()));
			continue;
		};
		let Some(expires_at) = parse_expiry(json.get("expired").and_then(Value::as_str)) else {
			let e = json.get("expired").and_then(Value::as_str).unwrap_or("?");
			skipped.push((name, format!("cannot parse expired={e}")));
			continue;
		};
		let email = s("email");
		let account_id = s("account_id");
		let mut credential = Map::new();
		credential.insert("type".into(), "oauth".into());
		credential.insert("access".into(), access.into());
		credential.insert("refresh".into(), refresh.into());
		credential.insert("expires".into(), expires_at.into());
		if let Some(e) = &email {
			credential.insert("email".into(), e.clone().into());
		}
		if let Some(a) = &account_id {
			credential.insert("accountId".into(), a.clone().into());
		}
		entries.push(PlanEntry {
			file: name,
			provider,
			email,
			account_id,
			expires_at,
			disabled,
			credential,
		});
	}

	let paint = Paint::stdout();
	if args.json {
		let plan: Vec<Value> = entries
			.iter()
			.map(|e| {
				json!({"provider": e.provider, "email": e.email, "accountId": e.account_id,
					"expiresAt": e.expires_at, "disabled": e.disabled, "file": e.file})
			})
			.collect();
		let imported: Vec<Value> = if args.dry_run {
			Vec::new()
		} else {
			entries
				.iter()
				.map(|e| json!({"provider": e.provider, "email": e.email, "file": e.file}))
				.collect()
		};
		let skipped: Vec<Value> =
			skipped.iter().map(|(f, r)| json!({"file": f, "reason": r})).collect();
		println!(
			"{}",
			json!({"dryRun": args.dry_run, "imported": imported, "plan": plan, "skipped": skipped})
		);
	} else {
		for (f, r) in &skipped {
			println!("{} {f}: {r}", paint.yellow("skip"));
		}
	}
	if entries.is_empty() {
		if !args.json {
			println!("No importable credentials in {}.", target.display());
		}
		return Ok(0);
	}
	if args.dry_run {
		if !args.json {
			println!("Dry run — would import {} credential(s):", entries.len());
			for e in &entries {
				println!("  {}", e.describe());
			}
		}
		return Ok(0);
	}

	if let Some(cfg) = resolve_broker_config()? {
		let mut code = 0;
		for e in &entries {
			let body = json!({"provider": e.provider, "credential": e.credential});
			match broker_request(&cfg, "POST", "/v1/credential", true, Some(&body)) {
				Ok(_) if !args.json => {
					println!("{} {} → {}", paint.green("uploaded"), e.describe(), cfg.url);
				},
				Ok(_) => {},
				Err(err) => {
					if args.json {
						println!("{}", json!({"error": err.to_string(), "file": e.file}));
					} else {
						println!("{} {}: {err}", paint.red("failed"), e.describe());
					}
					code = 1;
				},
			}
		}
		return Ok(code);
	}
	let store = AuthStorage::discover()?;
	for e in &entries {
		store.upsert_oauth(&e.provider, &e.credential)?;
		if !args.json {
			println!("{} {}", paint.green("imported"), e.describe());
		}
	}
	Ok(0)
}

fn status(args: &AuthBrokerArgs) -> Result<u8> {
	let paint = Paint::stdout();
	let Some(cfg) = resolve_broker_config()? else {
		if args.json {
			println!("{}", json!({"ok": false, "reason": "not_configured"}));
		} else {
			println!(
				"{}",
				paint.yellow("No auth-broker configured (set HARVEST_AUTH_BROKER_URL to enable).")
			);
		}
		return Ok(0);
	};
	match broker_request(&cfg, "GET", "/v1/healthz", false, None) {
		Ok(health) => {
			if args.json {
				let mut out = Map::new();
				out.insert("url".into(), cfg.url.clone().into());
				if let Value::Object(h) = health {
					out.extend(h);
				}
				println!("{}", Value::Object(out));
			} else {
				let v = health.get("version").and_then(Value::as_str).unwrap_or("unknown");
				println!("{} {} (version={v})", paint.green("OK"), cfg.url);
			}
			Ok(0)
		},
		Err(err) => {
			if args.json {
				println!("{}", json!({"ok": false, "url": cfg.url, "error": err.to_string()}));
			} else {
				println!("{} {}: {err}", paint.red("FAILED"), cfg.url);
			}
			Ok(1)
		},
	}
}
