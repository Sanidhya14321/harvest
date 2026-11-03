//! `harvest ssh` — manage SSH host configurations (`ssh-cli.ts`, `ssh/config-writer.ts`).
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use harvest_runtime::JailedFs;
use serde_json::{Map, Value};

use super::{CONFIG_DIR_NAME, Paint};

const MAX_CONFIG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SshAction {
	Add,
	Remove,
	List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scope {
	Project,
	User,
}

impl Scope {
	fn name(self) -> &'static str {
		match self {
			Self::Project => "project",
			Self::User => "user",
		}
	}
}

#[derive(Debug, Args)]
pub struct SshArgs {
	/// SSH action
	#[arg(value_enum)]
	pub action:  Option<SshAction>,
	/// Host name or arguments
	pub targets: Vec<String>,
	/// Output JSON
	#[arg(long)]
	pub json:    bool,
	/// Host address
	#[arg(long)]
	pub host:    Option<String>,
	/// Username
	#[arg(long)]
	pub user:    Option<String>,
	/// Port number
	#[arg(long)]
	pub port:    Option<String>,
	/// Identity key path
	#[arg(long)]
	pub key:     Option<String>,
	/// Host description
	#[arg(long)]
	pub desc:    Option<String>,
	/// Enable compatibility mode
	#[arg(long)]
	pub compat:  bool,
	/// Config scope (project|user)
	#[arg(long, value_enum)]
	pub scope:   Option<Scope>,
}

fn config_path(scope: Scope) -> Result<PathBuf> {
	Ok(match scope {
		Scope::User => super::agent_dir()?.join("ssh.json"),
		Scope::Project => std::env::current_dir()?.join(CONFIG_DIR_NAME).join("ssh.json"),
	})
}

fn read_config(path: &Path) -> Result<Map<String, Value>> {
	let bytes = match std::fs::read(path) {
		Ok(bytes) => bytes,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
		Err(error) => return Err(error.into()),
	};
	if bytes.len() > MAX_CONFIG_BYTES {
		bail!("SSH config file {} is too large", path.display());
	}
	match serde_json::from_slice::<Value>(&bytes) {
		Ok(Value::Object(map)) => Ok(map),
		Ok(_) => bail!("Failed to parse SSH config file {}: expected an object", path.display()),
		Err(error) => bail!("Failed to parse SSH config file {}: {error}", path.display()),
	}
}

fn hosts_of(config: &Map<String, Value>) -> Map<String, Value> {
	config.get("hosts").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn write_config(path: &Path, config: &Map<String, Value>) -> Result<()> {
	let dir = path.parent().context("SSH config path has no parent")?;
	let fs = JailedFs::open_or_create(dir)?;
	let content = serde_json::to_string_pretty(config)?;
	fs.atomic_write(Path::new("ssh.json"), content.as_bytes())
}

fn validate_host_name(name: &str) -> Option<&'static str> {
	if name.is_empty() {
		return Some("Host name cannot be empty");
	}
	if name.len() > 100 {
		return Some("Host name is too long (max 100 characters)");
	}
	if !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')) {
		return Some("Host name can only contain letters, numbers, dash, underscore, and dot");
	}
	None
}

pub fn execute(args: SshArgs) -> Result<u8> {
	match args.action.unwrap_or(SshAction::List) {
		SshAction::Add => add(&args),
		SshAction::Remove => remove(&args),
		SshAction::List => list(&args),
	}
}

fn add(args: &SshArgs) -> Result<u8> {
	let p = Paint::stdout();
	let Some(name) = args.targets.first() else {
		print!("{}", p.red("Error: Host name required\n"));
		print!(
			"{}",
			p.dim(
				"Usage: omp ssh add <name> --host <address> [--user <user>] [--port <port>] [--key \
				 <path>]\n"
			)
		);
		return Ok(1);
	};
	let Some(host) = args.host.as_deref() else {
		print!("{}", p.red("Error: --host is required\n"));
		print!("{}", p.dim("Usage: omp ssh add <name> --host <address>\n"));
		return Ok(1);
	};
	let mut port = None;
	if let Some(raw) = args.port.as_deref() {
		// parseInt semantics: leading integer prefix.
		let digits: String = raw
			.trim_start()
			.chars()
			.enumerate()
			.take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
			.map(|(_, c)| c)
			.collect();
		match digits.parse::<i64>() {
			Ok(value) if (1..=65535).contains(&value) => port = Some(value),
			_ => {
				print!("{}", p.red("Error: Port must be an integer between 1 and 65535\n"));
				return Ok(1);
			},
		}
	}
	let mut host_config = Map::new();
	host_config.insert("host".into(), Value::from(host));
	if let Some(user) = args.user.as_deref().filter(|v| !v.is_empty()) {
		host_config.insert("username".into(), Value::from(user));
	}
	if let Some(port) = port {
		host_config.insert("port".into(), Value::from(port));
	}
	if let Some(key) = args.key.as_deref().filter(|v| !v.is_empty()) {
		host_config.insert("keyPath".into(), Value::from(key));
	}
	if let Some(desc) = args.desc.as_deref().filter(|v| !v.is_empty()) {
		host_config.insert("description".into(), Value::from(desc));
	}
	if args.compat {
		host_config.insert("compat".into(), Value::Bool(true));
	}
	let scope = args.scope.unwrap_or(Scope::Project);
	let result = (|| -> Result<()> {
		let path = config_path(scope)?;
		if let Some(error) = validate_host_name(name) {
			bail!("{error}");
		}
		if host.is_empty() {
			bail!("Host address cannot be empty");
		}
		let mut config = read_config(&path)?;
		let mut hosts = hosts_of(&config);
		if hosts.contains_key(name) {
			bail!("Host \"{name}\" already exists in {}", path.display());
		}
		hosts.insert(name.clone(), Value::Object(host_config));
		config.insert("hosts".into(), Value::Object(hosts));
		write_config(&path, &config)
	})();
	match result {
		Ok(()) => {
			print!("{}", p.green(&format!("Added SSH host \"{name}\" to {} config\n", scope.name())));
			Ok(0)
		},
		Err(error) => {
			print!("{}", p.red(&format!("Error: {error}\n")));
			Ok(1)
		},
	}
}

fn remove(args: &SshArgs) -> Result<u8> {
	let p = Paint::stdout();
	let Some(name) = args.targets.first() else {
		print!("{}", p.red("Error: Host name required\n"));
		print!("{}", p.dim("Usage: omp ssh remove <name> [--scope project|user]\n"));
		return Ok(1);
	};
	let scope = args.scope.unwrap_or(Scope::Project);
	let result = (|| -> Result<()> {
		let path = config_path(scope)?;
		let mut config = read_config(&path)?;
		let mut hosts = hosts_of(&config);
		if hosts.remove(name).is_none() {
			bail!("Host \"{name}\" not found in {}", path.display());
		}
		config.insert("hosts".into(), Value::Object(hosts));
		write_config(&path, &config)
	})();
	match result {
		Ok(()) => {
			print!("{}", p.green(&format!("Removed SSH host \"{name}\" from {} config\n", scope.name())));
			Ok(0)
		},
		Err(error) => {
			print!("{}", p.red(&format!("Error: {error}\n")));
			Ok(1)
		},
	}
}

fn list(args: &SshArgs) -> Result<u8> {
	let p = Paint::stdout();
	let project_hosts = hosts_of(&read_config(&config_path(Scope::Project)?)?);
	let user_hosts = hosts_of(&read_config(&config_path(Scope::User)?)?);
	if args.json {
		let payload = serde_json::json!({ "project": project_hosts, "user": user_hosts });
		println!("{}", serde_json::to_string_pretty(&payload)?);
		return Ok(0);
	}
	let has_project = !project_hosts.is_empty();
	let has_user = !user_hosts.is_empty();
	if !has_project && !has_user {
		print!("{}", p.dim("No SSH hosts configured\n"));
		print!("{}", p.dim("Add one with: omp ssh add <name> --host <address>\n"));
		return Ok(0);
	}
	if has_project {
		print!("{}", p.bold("Project SSH Hosts (.omp/ssh.json):\n"));
		print_hosts(&p, &project_hosts);
	}
	if has_project && has_user {
		println!();
	}
	if has_user {
		print!("{}", p.bold("User SSH Hosts (~/.omp/agent/ssh.json):\n"));
		print_hosts(&p, &user_hosts);
	}
	Ok(0)
}

fn print_hosts(p: &Paint, hosts: &Map<String, Value>) {
	let text = |config: &Value, key: &str| {
		config.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned)
	};
	for (name, config) in hosts {
		let mut parts = vec![p.cyan(name), text(config, "host").unwrap_or_default()];
		if let Some(user) = text(config, "username") {
			parts.push(p.dim(&user));
		}
		if let Some(port) = config.get("port").and_then(Value::as_i64).filter(|port| *port != 22) {
			parts.push(p.dim(&format!("port:{port}")));
		}
		if let Some(key) = text(config, "keyPath") {
			parts.push(p.dim(&key));
		}
		if let Some(desc) = text(config, "description") {
			parts.push(p.dim(&format!("- {desc}")));
		}
		println!("  {}", parts.join("  "));
	}
}
