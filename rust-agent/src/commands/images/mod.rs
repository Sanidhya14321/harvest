//! `harvest images` — inspect and maintain image publication backends.
//!
//! Ported from `packages/coding-agent/src/cli/images-cli.ts`. The native app
//! has no blob-broker daemon yet, so every daemon query reports a stopped
//! daemon exactly as the TypeScript command does when the daemon is
//! unreachable.
use std::{
	collections::BTreeMap,
	io::Write,
	path::{Path, PathBuf},
	time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};


const DESTINATIONS: &str = include_str!("destinations.json");
const DEFAULT_BACKENDS: [&str; 4] = ["provider-files", "tailscale", "cloudflared", "litterbox"];
const PROVIDERS: [&str; 3] = ["openai", "anthropic", "google"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Action {
	Status,
	Doctor,
	Probe,
	Purge,
}

impl Action {
	const fn name(self) -> &'static str {
		match self {
			Self::Status => "status",
			Self::Doctor => "doctor",
			Self::Probe => "probe",
			Self::Purge => "purge",
		}
	}
}

#[derive(Debug, clap::Args)]
pub struct ImagesArgs {
	/// status (default), doctor, probe, or purge
	#[arg(value_enum)]
	action:  Option<Action>,
	/// Output one JSON document
	#[arg(long)]
	json:    bool,
	/// Apply purge deletions (default is dry-run)
	#[arg(long)]
	apply:   bool,
	/// Purge all entries instead of expired entries only
	#[arg(long)]
	all:     bool,
	/// Project directory (default: current directory)
	#[arg(long)]
	dir:     Option<PathBuf>,
	/// External health probe timeout in seconds
	#[arg(long, allow_negative_numbers = true)]
	timeout: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Destination {
	label:                String,
	status:               String,
	reason:               Option<String>,
	required_options:     Vec<String>,
	required_credentials: Vec<String>,
}

struct Config {
	enabled:          bool,
	backends:         Vec<String>,
	options:          BTreeMap<String, Value>,
	credentials:      BTreeMap<String, Value>,
	settings:         Value,
	blobs_dir:        PathBuf,
	project_hash:     String,
	provider_index:   PathBuf,
	savings_journal:  PathBuf,
	agent_dir:        Option<PathBuf>,
}

fn setting<'a>(settings: &'a Value, key: &str) -> Option<&'a Value> {
	// Accept both nested YAML (`images: urls: enabled`) and dotted flat keys.
	if let Some(value) = settings.get(key) {
		return Some(value);
	}
	key.split('.').try_fold(settings, |value, part| value.get(part))
}

fn load_config(project_dir: &Path) -> Result<Config> {
	let agent_dir = crate::config::agent_dir_from(None, |key| std::env::var_os(key));
	let mut settings = Value::Null;
	if let Some(dir) = &agent_dir {
		for name in ["config.yml", "config.yaml"] {
			match std::fs::read_to_string(dir.join(name)) {
				Ok(text) => {
					settings = serde_yaml_ng::from_str(&text)?;
					break;
				},
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
				Err(error) => return Err(error.into()),
			}
		}
	}
	let enabled = setting(&settings, "images.urls.enabled")
		.and_then(Value::as_bool)
		.unwrap_or(false);
	let backends = setting(&settings, "images.urls.backends")
		.and_then(Value::as_array)
		.map_or_else(
			|| DEFAULT_BACKENDS.iter().map(|s| (*s).to_owned()).collect(),
			|items| items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
		);
	let object = |key: &str| -> BTreeMap<String, Value> {
		setting(&settings, key)
			.and_then(Value::as_object)
			.map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
			.unwrap_or_default()
	};
	let options = object("images.urls.options");
	let credentials = object("images.urls.credentials");
	let blobs_dir = agent_dir.clone().unwrap_or_default().join("blobs");
	let resolved = std::path::absolute(project_dir)?;
	let project_hash = format!("{:x}", super::paths::wyhash(resolved.to_string_lossy().as_bytes(), 0));
	Ok(Config {
		enabled,
		backends,
		options,
		credentials,
		provider_index: blobs_dir.join(format!("provider-files-index-{project_hash}.json")),
		savings_journal: blobs_dir.join(format!("image-savings-{project_hash}.jsonl")),
		blobs_dir,
		project_hash,
		settings,
		agent_dir,
	})
}

fn safe_detail(message: &str) -> String {
	let bearer = regex::Regex::new(r"(?i)\bBearer\s+[^\s]+").expect("valid regex");
	let query =
		regex::Regex::new(r"(?i)([?&](?:access_token|api[_-]?key|key|token)=)[^&#\s]+").expect("valid regex");
	let url = regex::Regex::new(r"(?i)https?://[^\s)]+").expect("valid regex");
	let text = bearer.replace_all(message, "Bearer [redacted]");
	let text = query.replace_all(&text, "${1}[redacted]");
	let text = url.replace_all(&text, |caps: &regex::Captures<'_>| {
		let raw = &caps[0];
		let rest = raw.split_once("://").map(|(scheme, rest)| (scheme, rest));
		match rest {
			Some((scheme, rest)) => {
				let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
				if host.is_empty() {
					"[redacted URL]".to_owned()
				} else {
					format!("{}://{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
				}
			},
			None => "[redacted URL]".to_owned(),
		}
	});
	text.chars().take(500).collect()
}

fn format_bytes(bytes: f64) -> String {
	const K: f64 = 1024.0;
	if bytes < K {
		format!("{bytes} B")
	} else if bytes < K * K {
		format!("{:.1} KiB", bytes / K)
	} else if bytes < K * K * K {
		format!("{:.1} MiB", bytes / (K * K))
	} else {
		format!("{:.1} GiB", bytes / (K * K * K))
	}
}

fn now_ms() -> f64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0.0, |d| d.as_millis() as f64)
}

fn is_hex64(value: Option<&Value>) -> bool {
	value
		.and_then(Value::as_str)
		.is_some_and(|s| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn valid_entry(value: &Value) -> bool {
	let Some(provider) = value.get("provider").and_then(Value::as_str) else {
		return false;
	};
	let Some(handle) = value.get("handle").filter(|h| h.is_object()) else {
		return false;
	};
	PROVIDERS.contains(&provider)
		&& is_hex64(value.get("credentialHash"))
		&& is_hex64(value.get("contentHash"))
		&& handle.get("provider").and_then(Value::as_str) == Some(provider)
		&& handle.get("mimeType").is_some_and(Value::is_string)
		&& handle.get("bytes").and_then(Value::as_u64).is_some()
		&& handle.get("expiresAt").is_none_or(Value::is_number)
}

struct Snapshot {
	entries:    Vec<Value>,
	last_error: Option<String>,
}

fn read_provider_snapshot(path: &Path) -> Snapshot {
	let source = match std::fs::read_to_string(path) {
		Ok(source) => source,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			return Snapshot { entries: Vec::new(), last_error: None };
		},
		Err(error) => {
			return Snapshot { entries: Vec::new(), last_error: Some(safe_detail(&error.to_string())) };
		},
	};
	let malformed = || Snapshot {
		entries:    Vec::new(),
		last_error: Some("Malformed provider file cache index".into()),
	};
	let Ok(parsed) = serde_json::from_str::<Value>(&source) else {
		return malformed();
	};
	if !parsed.is_object() {
		return malformed();
	}
	let entries = parsed.get("entries").and_then(Value::as_array);
	if parsed.get("version").and_then(Value::as_i64) != Some(1) || entries.is_none() {
		return Snapshot {
			entries:    Vec::new(),
			last_error: Some("Unsupported provider file cache index".into()),
		};
	}
	Snapshot {
		entries:    entries
			.into_iter()
			.flatten()
			.filter(|e| valid_entry(e))
			.cloned()
			.collect(),
		last_error: None,
	}
}

fn entry_bytes(entry: &Value) -> u64 {
	entry["handle"]["bytes"].as_u64().unwrap_or(0)
}

fn entry_expired(entry: &Value, now: f64) -> bool {
	entry["handle"]["expiresAt"].as_f64().is_some_and(|at| at <= now)
}

fn provider_file_status(path: &Path, snapshot: &Snapshot) -> Value {
	let mut providers = BTreeMap::from([("openai", 0u64), ("anthropic", 0), ("google", 0)]);
	let (mut bytes, mut entries) = (0u64, 0u64);
	let now = now_ms();
	for entry in &snapshot.entries {
		if entry_expired(entry, now) {
			continue;
		}
		entries += 1;
		bytes += entry_bytes(entry);
		if let Some(count) = entry["provider"].as_str().and_then(|p| providers.get_mut(p)) {
			*count += 1;
		}
	}
	let mut status = json!({
		"indexPath": path,
		"entries": entries,
		"bytes": bytes,
		"providers": {"openai": providers["openai"], "anthropic": providers["anthropic"], "google": providers["google"]},
		"dirty": false,
	});
	if let Some(error) = &snapshot.last_error {
		status["lastError"] = json!(error);
	}
	status
}

fn read_savings(path: &Path) -> Result<Value> {
	#[derive(Default, Clone, Copy)]
	struct Counters {
		entries:         u64,
		image_count:     u64,
		inline_bytes:    u64,
		reference_bytes: u64,
		saved_bytes:     i64,
	}
	impl Counters {
		fn add(&mut self, record: &Value) {
			self.entries += 1;
			self.image_count += record["imageCount"].as_u64().unwrap_or(0);
			self.inline_bytes += record["inlineBytes"].as_u64().unwrap_or(0);
			self.reference_bytes += record["referenceBytes"].as_u64().unwrap_or(0);
			self.saved_bytes += record["savedBytes"].as_i64().unwrap_or(0);
		}
		fn json(self) -> Value {
			json!({
				"entries": self.entries,
				"imageCount": self.image_count,
				"inlineBytes": self.inline_bytes,
				"referenceBytes": self.reference_bytes,
				"savedBytes": self.saved_bytes,
			})
		}
	}
	let text = match std::fs::read_to_string(path) {
		Ok(text) => text,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
		Err(error) => return Err(error.into()),
	};
	let mut total = Counters::default();
	let mut by_destination: BTreeMap<String, Counters> = BTreeMap::new();
	for line in text.lines().filter(|line| !line.trim().is_empty()) {
		let Ok(record) = serde_json::from_str::<Value>(line) else {
			continue;
		};
		let destination = record["destination"].as_str().unwrap_or_default();
		let valid = record["timestamp"].as_u64().is_some()
			&& record["provider"].is_string()
			&& record["model"].is_string()
			&& !destination.is_empty()
			&& record["imageCount"].as_u64().is_some_and(|n| n > 0)
			&& record["inlineBytes"].as_u64().is_some()
			&& record["referenceBytes"].as_u64().is_some()
			&& record["savedBytes"].as_i64().is_some();
		if !valid {
			continue;
		}
		total.add(&record);
		by_destination.entry(destination.to_owned()).or_default().add(&record);
	}
	let mut value = total.json();
	value["journalPath"] = json!(path);
	value["byDestination"] =
		Value::Object(by_destination.into_iter().map(|(k, v)| (k, v.json())).collect());
	Ok(value)
}

fn collect_status(project_dir: &Path, config: &Config) -> Result<Value> {
	let savings = read_savings(&config.savings_journal)?;
	let snapshot = read_provider_snapshot(&config.provider_index);
	Ok(json!({
		"action": "status",
		"exitCode": 0,
		"projectDir": project_dir,
		"enabled": config.enabled,
		"backends": config.backends,
		"daemon": {"state": "stopped"},
		"providerFiles": provider_file_status(&config.provider_index, &snapshot),
		"savings": savings,
	}))
}

fn configured(value: Option<&Value>) -> bool {
	match value {
		None | Some(Value::Null) => false,
		Some(Value::String(s)) => !s.trim().is_empty(),
		Some(_) => true,
	}
}

fn binary_for(destination: &str) -> Option<&'static str> {
	Some(match destination {
		"cloudflared" | "named-cloudflared" => "cloudflared",
		"ngrok" => "ngrok",
		"tailscale" => "tailscale",
		"localhost-run" | "pinggy" | "ssh" => "ssh",
		"devtunnel" => "devtunnel",
		"zrok" => "zrok",
		"bore" => "bore",
		_ => return None,
	})
}

fn which(binary: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path)
		.map(|dir| dir.join(binary))
		.find(|candidate| candidate.is_file())
}

fn check(name: impl Into<String>, severity: &str, detail: impl Into<String>) -> Value {
	json!({"name": name.into(), "severity": severity, "detail": detail.into()})
}

fn config_checks(config: &Config, registry: &BTreeMap<String, Destination>) -> Vec<Value> {
	let mut checks = vec![check(
		"enabled",
		if config.enabled { "ok" } else { "error" },
		if config.enabled { "Image publication is enabled" } else { "images.urls.enabled is false" },
	)];
	if config.backends.is_empty() {
		checks.push(check("backends", "error", "No image publication backends are configured"));
	}
	for backend in &config.backends {
		let Some(metadata) = registry.get(backend) else {
			checks.push(check(format!("registry:{backend}"), "error", format!("{backend}: unknown")));
			continue;
		};
		let unavailable = metadata.status == "defunct" || metadata.status == "incompatible";
		let severity = if unavailable {
			"error"
		} else if metadata.status == "requires-account" {
			"warn"
		} else {
			"ok"
		};
		let detail = if unavailable {
			format!("{}: {}", metadata.label, metadata.reason.as_deref().unwrap_or(&metadata.status))
		} else {
			format!("{}: {}", metadata.label, metadata.status)
		};
		checks.push(check(format!("registry:{backend}"), severity, detail));
		if backend != "provider-files" {
			let options = config.options.get(backend);
			let credentials = config.credentials.get(backend);
			let serve_field = |key: &str| -> Option<&Value> {
				match key {
					"publicBaseUrl" | "bindHost" | "sshTarget" | "sshRemotePort" => {
						setting(&config.settings, &format!("images.urls.{key}"))
					},
					_ => None,
				}
			};
			let missing: Vec<&str> = metadata
				.required_options
				.iter()
				.filter(|key| {
					!configured(options.and_then(|o| o.get(key.as_str())).or_else(|| serve_field(key)))
				})
				.chain(
					metadata
						.required_credentials
						.iter()
						.filter(|key| !configured(credentials.and_then(|c| c.get(key.as_str())))),
				)
				.map(String::as_str)
				.collect();
			checks.push(check(
				format!("config:{backend}"),
				if missing.is_empty() { "ok" } else { "error" },
				if missing.is_empty() {
					"Required configuration is present".to_owned()
				} else {
					format!("Missing required fields: {}", missing.join(", "))
				},
			));
		}
		if let Some(binary) = binary_for(backend) {
			let found = which(binary).is_some();
			checks.push(check(
				format!("binary:{backend}"),
				if found { "ok" } else { "error" },
				if found { format!("{binary} is available") } else { format!("{binary} is not on PATH") },
			));
		}
	}
	checks
}

fn writable(directory: &Path) -> bool {
	std::fs::metadata(directory).is_ok_and(|m| m.is_dir() && !m.permissions().readonly())
}

fn disk_checks(config: &Config) -> Vec<Value> {
	let mut directories: Vec<PathBuf> = Vec::new();
	let mut push = |dir: PathBuf| {
		if !directories.contains(&dir) {
			directories.push(dir);
		}
	};
	push(config.provider_index.parent().unwrap_or(Path::new("")).to_path_buf());
	push(config.savings_journal.parent().unwrap_or(Path::new("")).to_path_buf());
	if config.backends.iter().any(|b| b != "provider-files") {
		push(config.blobs_dir.clone());
	}
	let _ = &config.project_hash;
	directories
		.into_iter()
		.map(|dir| {
			if writable(&dir) {
				check(
					"disk",
					"ok",
					format!("Cache directory is readable and writable: {}", dir.display()),
				)
			} else {
				check(
					"disk",
					"warn",
					format!("Cache directory is absent or not writable: {}", dir.display()),
				)
			}
		})
		.collect()
}

fn authenticated_providers(config: &Config) -> Result<Vec<&'static str>> {
	let path = config.agent_dir.clone().unwrap_or_default().join("agent.db");
	let connection = rusqlite::Connection::open_with_flags(
		&path,
		rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
	)?;
	let mut found = Vec::new();
	for provider in PROVIDERS {
		let count: i64 = connection.query_row(
			"SELECT COUNT(*) FROM auth_credentials WHERE provider = ?1 AND disabled_cause IS NULL",
			[provider],
			|row| row.get(0),
		)?;
		let env_key = match provider {
			"openai" => "OPENAI_API_KEY",
			"anthropic" => "ANTHROPIC_API_KEY",
			_ => "GEMINI_API_KEY",
		};
		if count > 0 || std::env::var_os(env_key).is_some_and(|v| !v.is_empty()) {
			found.push(provider);
		}
	}
	Ok(found)
}

fn collect_doctor(project_dir: &Path, config: &Config) -> Result<Value> {
	let registry: BTreeMap<String, Destination> = serde_json::from_str(DESTINATIONS)?;
	let mut checks = config_checks(config, &registry);
	checks.extend(disk_checks(config));
	if config.backends.iter().any(|b| b == "provider-files") {
		checks.push(match authenticated_providers(config) {
			Ok(found) if !found.is_empty() => check(
				"config:provider-files",
				"ok",
				format!("Authentication is available for: {}", found.join(", ")),
			),
			Ok(_) => check(
				"config:provider-files",
				"error",
				"No OpenAI, Anthropic, or Google authentication is available",
			),
			Err(_) => check(
				"config:provider-files",
				"warn",
				"Provider authentication storage could not be inspected",
			),
		});
	}
	checks.push(check("daemon", "warn", "Image daemon is stopped or unreachable"));
	let healthy = !checks.iter().any(|c| c["severity"] == "error");
	Ok(json!({
		"action": "doctor",
		"exitCode": if healthy { 0 } else { 1 },
		"projectDir": project_dir,
		"healthy": healthy,
		"checks": checks,
	}))
}

fn collect_probe(project_dir: &Path, config: &Config) -> Value {
	let first = config.backends.iter().find(|b| *b != "provider-files");
	match first {
		Some(backend) if config.enabled => json!({
			"action": "probe",
			"exitCode": 1,
			"projectDir": project_dir,
			"backend": backend,
			"daemonState": "stopped",
			"ok": false,
			"detail": "Image daemon could not be started or reached",
		}),
		_ => json!({
			"action": "probe",
			"exitCode": 1,
			"projectDir": project_dir,
			"daemonState": "stopped",
			"ok": false,
			"detail": if config.enabled { "No URL backend is configured" } else { "Image publication is disabled" },
		}),
	}
}

fn collect_purge(project_dir: &Path, config: &Config, apply: bool, all: bool) -> Value {
	let snapshot = read_provider_snapshot(&config.provider_index);
	let now = now_ms();
	let selected: Vec<&Value> = snapshot
		.entries
		.iter()
		.filter(|entry| all || entry_expired(entry, now))
		.collect();
	let bytes: u64 = selected.iter().map(|e| entry_bytes(e)).sum();
	// Remote provider deletion needs the provider file clients, which the
	// native runtime does not have yet; applied purges skip them as
	// unauthenticated so the command fails closed and deletes nothing.
	let skipped_auth = if apply { selected.len() } else { 0 };
	let mut errors: Vec<String> = Vec::new();
	if let Some(error) = &snapshot.last_error {
		errors.push(error.clone());
	}
	let failed = !errors.is_empty() || (apply && skipped_auth > 0);
	json!({
		"action": "purge",
		"exitCode": if failed { 1 } else { 0 },
		"projectDir": project_dir,
		"applied": apply,
		"all": all,
		"daemon": null,
		"providerFiles": {
			"selected": selected.len(),
			"bytes": bytes,
			"deleted": 0,
			"skippedAuth": skipped_auth,
			"errors": errors,
		},
	})
}

fn num(value: &Value) -> f64 {
	value.as_f64().unwrap_or(0.0)
}

fn render_human(result: &Value) -> String {
	if let Some(error) = result["error"].as_str() {
		return format!("images {}: {error}\n", result["action"].as_str().unwrap_or_default());
	}
	let mut lines: Vec<String> = Vec::new();
	match result["action"].as_str().unwrap_or_default() {
		"status" => {
			let backends: Vec<&str> =
				result["backends"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
			lines.push(format!(
				"Image backends: {}",
				if backends.is_empty() { "none".to_owned() } else { backends.join(" → ") }
			));
			lines.push(format!("Enabled: {}", if result["enabled"] == true { "yes" } else { "no" }));
			let daemon = &result["daemon"];
			let base = daemon["baseUrl"].as_str().map(|u| format!(" ({u})")).unwrap_or_default();
			lines.push(format!("Daemon: {}{base}", daemon["state"].as_str().unwrap_or("stopped")));
			let savings = &result["savings"];
			lines.push(format!(
				"Bytes saved: {} ({} inline → {} references)",
				format_bytes(num(&savings["savedBytes"])),
				format_bytes(num(&savings["inlineBytes"])),
				format_bytes(num(&savings["referenceBytes"]))
			));
			let files = &result["providerFiles"];
			lines.push(format!(
				"Provider files: {} active, {}",
				files["entries"],
				format_bytes(num(&files["bytes"]))
			));
		},
		"doctor" => {
			for check in result["checks"].as_array().into_iter().flatten() {
				lines.push(format!(
					"[{}] {}: {}",
					check["severity"].as_str().unwrap_or_default().to_ascii_uppercase(),
					check["name"].as_str().unwrap_or_default(),
					check["detail"].as_str().unwrap_or_default()
				));
			}
			lines.push(
				if result["healthy"] == true {
					"Image diagnostics passed."
				} else {
					"Image diagnostics found errors."
				}
				.to_owned(),
			);
		},
		"probe" => {
			let duration = result["durationMs"]
				.as_f64()
				.map(|ms| format!(" in {ms} ms"))
				.unwrap_or_default();
			lines.push(format!(
				"Image probe {}{duration}: {}",
				if result["ok"] == true { "passed" } else { "failed" },
				result["detail"].as_str().unwrap_or_default()
			));
		},
		_ => {
			let files = &result["providerFiles"];
			lines.push(
				if result["applied"] == true {
					"Image purge applied."
				} else {
					"Image purge dry-run; pass --apply to delete."
				}
				.to_owned(),
			);
			lines.push(format!(
				"Daemon blobs: {}, {}",
				result["daemon"]["purgedBlobs"].as_u64().unwrap_or(0),
				format_bytes(num(&result["daemon"]["reclaimedBytes"]))
			));
			lines.push(format!(
				"Provider files: {} selected, {} deleted, {}",
				files["selected"],
				files["deleted"],
				format_bytes(num(&files["bytes"]))
			));
			let skipped = files["skippedAuth"].as_u64().unwrap_or(0);
			if skipped > 0 {
				lines.push(format!(
					"Skipped {skipped} provider file(s): matching authentication was unavailable."
				));
			}
			for error in files["errors"].as_array().into_iter().flatten() {
				lines.push(format!("Provider error: {}", error.as_str().unwrap_or_default()));
			}
		},
	}
	format!("{}\n", lines.join("\n"))
}

pub fn execute(args: ImagesArgs) -> Result<u8> {
	let action = args.action.unwrap_or(Action::Status);
	let result = run(&args, action);
	let exit = result["exitCode"].as_u64().unwrap_or(1);
	let text = if args.json {
		format!("{result}\n")
	} else {
		render_human(&result)
	};
	// Standalone command output: no TUI or protocol is active.
	if args.json || result.get("error").is_none() {
		std::io::stdout().lock().write_all(text.as_bytes())?;
	} else {
		std::io::stderr().lock().write_all(text.as_bytes())?;
	}
	Ok(u8::try_from(exit).unwrap_or(1))
}

fn run(args: &ImagesArgs, action: Action) -> Value {
	if args.timeout.is_some_and(|t| t <= 0) {
		return json!({"action": action.name(), "exitCode": 2, "error": "--timeout must be a positive integer"});
	}
	let attempt = || -> Result<Value> {
		let cwd = std::env::current_dir()?;
		let project_dir = std::path::absolute(cwd.join(args.dir.as_deref().unwrap_or(Path::new("."))))?;
		let config = load_config(&project_dir)?;
		Ok(match action {
			Action::Status => collect_status(&project_dir, &config)?,
			Action::Doctor => collect_doctor(&project_dir, &config)?,
			Action::Probe => collect_probe(&project_dir, &config),
			Action::Purge => collect_purge(&project_dir, &config, args.apply, args.all),
		})
	};
	attempt().unwrap_or_else(|error| {
		json!({"action": action.name(), "exitCode": 1, "error": safe_detail(&format!("{error:#}"))})
	})
}
