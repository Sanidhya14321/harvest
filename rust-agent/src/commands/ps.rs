//! `harvest ps` — inspect and control daemon-broker supervised processes
//! (port of `packages/coding-agent/src/cli/{ps-cli,ps-data}.ts`).
//!
//! Collection never spawns a broker: live scopes are queried over the broker
//! socket, dead scopes are read from persisted `meta.json` snapshots.

use std::{
	fs,
	io::{BufRead, BufReader, Write},
	path::{Path, PathBuf},
	time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use super::paths::{self, Paint};

const KILL_GRACE_MS: u64 = 100;
const TABLE_HEADER: [&str; 7] = ["NAME", "STATE", "PID", "UPTIME", "RESTARTS", "FLAGS", "COMMAND"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PsAction {
	List,
	Info,
	Logs,
	Stop,
	Kill,
	Restart,
}

impl PsAction {
	fn name(self) -> &'static str {
		match self {
			Self::List => "list",
			Self::Info => "info",
			Self::Logs => "logs",
			Self::Stop => "stop",
			Self::Kill => "kill",
			Self::Restart => "restart",
		}
	}
}

#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  harvest ps\n  harvest ps --all\n  harvest ps logs web \
                        --follow\n  harvest ps stop web\n  harvest ps kill web\n  harvest ps \
                        info worker --global local-service")]
pub struct PsArgs {
	/// list (default), info, logs, stop, kill, or restart
	#[arg(value_enum)]
	action:  Option<PsAction>,
	/// Process name (required for every action except list)
	name:    Option<String>,
	/// List every project and global service scope (list)
	#[arg(short = 'a', long)]
	all:     bool,
	/// Emit machine-readable JSON
	#[arg(short = 'j', long)]
	json:    bool,
	/// Static listing instead of the interactive monitor (list)
	#[arg(long)]
	plain:   bool,
	/// Target another project directory instead of the current one
	#[arg(long)]
	dir:     Option<String>,
	/// Target a machine-global service scope (e.g. local-service)
	#[arg(long)]
	global:  Option<String>,
	/// Keep streaming new output (logs)
	#[arg(short = 'f', long)]
	follow:  bool,
	/// Read from the beginning instead of the tail (logs)
	#[arg(long)]
	head:    bool,
	/// Number of log lines, max 1000 (logs)
	#[arg(short = 'n', long)]
	lines:   Option<i64>,
	/// Regex filter applied to log lines (logs)
	#[arg(long)]
	grep:    Option<String>,
	/// Grace period in seconds before hard kill (stop)
	#[arg(long)]
	timeout: Option<f64>,
}

pub fn execute(args: PsArgs) -> Result<u8> {
	let paint = Paint::new();
	let action = args.action.unwrap_or(PsAction::List);
	if action == PsAction::List {
		// The interactive monitor is not ported; every list is the static view.
		return run_list(&args, &paint);
	}
	let Some(name) = args.name.clone() else {
		eprintln!(
			"{}",
			paint.red(&format!(
				"{} requires a process name. Run `omp ps` to list processes.",
				action.name()
			))
		);
		return Ok(1);
	};
	match run_action(&args, action, &name, &paint) {
		Ok(()) => Ok(0),
		Err(error) => {
			eprintln!("{}", paint.red(&format!("{error:#}")));
			Ok(1)
		},
	}
}

// ───────────────────────────── scopes ─────────────────────────────

#[derive(Debug, Clone)]
struct Scope {
	global:      bool,
	runtime_dir: PathBuf,
	project_dir: Option<String>,
	service:     Option<String>,
	broker_pid:  Option<i64>,
}

struct Row {
	snapshot:   Value,
	command:    Option<String>,
	cwd:        Option<String>,
	supervised: bool,
}

fn canonical(path: &Path) -> PathBuf {
	fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn absolute(path: &str) -> PathBuf {
	let path = Path::new(path);
	if path.is_absolute() {
		path.to_path_buf()
	} else {
		std::env::current_dir().unwrap_or_default().join(path)
	}
}

fn read_json(path: &Path) -> Option<Value> {
	serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn live_broker_pid(runtime_dir: &Path) -> Option<i64> {
	let pid = read_json(&runtime_dir.join("broker.pid"))?
		.get("pid")?
		.as_i64()?;
	paths::probe_pid(pid).is_ok().then_some(pid)
}

fn valid_service(service: &str) -> bool {
	let mut chars = service.chars();
	chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
		&& chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn target_scope(args: &PsArgs) -> Result<Scope> {
	if let Some(service) = &args.global {
		if !valid_service(service) {
			bail!("Invalid global daemon service name: {}", serde_json::to_string(service)?);
		}
		let runtime_dir = canonical(&paths::global_daemon_runtime_root().join(service));
		let broker_pid = live_broker_pid(&runtime_dir);
		return Ok(Scope {
			global: true,
			runtime_dir,
			project_dir: None,
			service: Some(service.clone()),
			broker_pid,
		});
	}
	let project = match &args.dir {
		Some(dir) => absolute(dir),
		None => std::env::current_dir()?,
	};
	let project = canonical(&project);
	let runtime_dir = paths::daemon_runtime_dir(&project);
	let broker_pid = live_broker_pid(&runtime_dir);
	Ok(Scope {
		global: false,
		runtime_dir,
		project_dir: Some(project.to_string_lossy().into_owned()),
		service: None,
		broker_pid,
	})
}

fn sorted_dirs(dir: &Path) -> Vec<String> {
	let mut names: Vec<String> = fs::read_dir(dir)
		.map(|entries| {
			entries
				.filter_map(|entry| entry.ok())
				.filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
				.map(|entry| entry.file_name().to_string_lossy().into_owned())
				.collect()
		})
		.unwrap_or_default();
	names.sort();
	names
}

fn scope_project_dir(runtime_dir: &Path) -> Option<String> {
	if let Some(dir) = read_json(&runtime_dir.join("scope.json"))
		.and_then(|value| value.get("projectDir")?.as_str().map(str::to_owned))
	{
		return Some(dir);
	}
	let clients = runtime_dir.join("clients");
	let mut files: Vec<PathBuf> = fs::read_dir(&clients)
		.map(|e| e.filter_map(|e| e.ok()).map(|e| e.path()).collect())
		.unwrap_or_default();
	files.sort();
	files.iter().find_map(|file| {
		read_json(file)?
			.get("projectDir")?
			.as_str()
			.map(str::to_owned)
	})
}

fn discover_scopes() -> Vec<Scope> {
	let mut scopes = Vec::new();
	let root = paths::daemon_runtime_root();
	for name in sorted_dirs(&root) {
		if name.len() != 16 || !name.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
			continue;
		}
		let runtime_dir = root.join(&name);
		scopes.push(Scope {
			global: false,
			project_dir: scope_project_dir(&runtime_dir),
			broker_pid: live_broker_pid(&runtime_dir),
			runtime_dir,
			service: None,
		});
	}
	let global_root = paths::global_daemon_runtime_root();
	for name in sorted_dirs(&global_root) {
		let runtime_dir = canonical(&global_root.join(&name));
		scopes.push(Scope {
			global: true,
			broker_pid: live_broker_pid(&runtime_dir),
			runtime_dir,
			project_dir: None,
			service: Some(name),
		});
	}
	scopes
}

// ───────────────────────────── broker client ─────────────────────────────

struct Client {
	reader: BufReader<std::os::unix::net::UnixStream>,
	writer: std::os::unix::net::UnixStream,
	token:  String,
	next:   u64,
}

impl Client {
	fn connect(runtime_dir: &Path) -> Result<Self> {
		let endpoint = runtime_dir.join("broker.sock");
		let token = fs::read_to_string(runtime_dir.join("broker.token"))
			.map(|token| token.trim().to_owned())
			.ok()
			.filter(|token| !token.is_empty())
			.ok_or_else(|| anyhow!("Daemon broker is not running at {}", endpoint.display()))?;
		let writer = std::os::unix::net::UnixStream::connect(&endpoint).map_err(|error| {
			anyhow!("Daemon broker is not running at {}: {error}", endpoint.display())
		})?;
		let reader = BufReader::new(writer.try_clone()?);
		Ok(Self { reader, writer, token, next: 0 })
	}

	fn request(&mut self, operation: Value, timeout_ms: u64) -> Result<Value> {
		self.next += 1;
		let id = format!("harvest-ps-{}-{}", std::process::id(), self.next);
		let op = operation
			.get("op")
			.and_then(Value::as_str)
			.unwrap_or_default()
			.to_owned();
		let line = json!({ "id": id, "token": self.token, "operation": operation });
		self.writer.write_all(format!("{line}\n").as_bytes())?;
		self
			.writer
			.set_read_timeout(Some(Duration::from_millis(timeout_ms)))?;
		let mut buffer = String::new();
		loop {
			buffer.clear();
			let read = self.reader.read_line(&mut buffer).map_err(|error| {
				if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
				{
					anyhow!("Daemon {op} request timed out")
				} else {
					error.into()
				}
			})?;
			if read == 0 {
				bail!("Daemon broker connection closed");
			}
			let Ok(message) = serde_json::from_str::<Value>(buffer.trim()) else {
				continue;
			};
			if message.get("event").is_some() || message.get("id").and_then(Value::as_str) != Some(&id)
			{
				continue;
			}
			if message.get("ok").and_then(Value::as_bool) != Some(true) {
				let error = message
					.get("error")
					.and_then(Value::as_str)
					.unwrap_or("Daemon broker rejected request");
				bail!("{error}");
			}
			return Ok(message.get("result").cloned().unwrap_or(Value::Null));
		}
	}
}

// ───────────────────────────── collection ─────────────────────────────

fn is_terminal_state(state: &str) -> bool {
	matches!(state, "exited" | "failed")
}

fn state_of(snapshot: &Value) -> &str {
	snapshot
		.get("state")
		.and_then(Value::as_str)
		.unwrap_or("unknown")
}

fn format_command(spec: Option<&Value>) -> Option<String> {
	let spec = spec?;
	let mut parts = vec![spec.get("application")?.as_str()?.to_owned()];
	for arg in spec
		.get("args")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
	{
		parts.push(arg.as_str().unwrap_or_default().to_owned());
	}
	Some(parts.join(" "))
}

fn persisted_daemons(runtime_dir: &Path) -> Vec<(String, Value, Value)> {
	let root = runtime_dir.join("daemons");
	let mut out = Vec::new();
	for name in sorted_dirs(&root) {
		let Some(meta) = read_json(&root.join(name).join("meta.json")) else {
			continue;
		};
		let (Some(daemon), Some(spec)) = (meta.get("daemon"), meta.get("spec")) else {
			continue;
		};
		let Some(daemon_name) = daemon.get("name").and_then(Value::as_str) else {
			continue;
		};
		out.push((daemon_name.to_owned(), daemon.clone(), spec.clone()));
	}
	out
}

fn collect_scope(mut scope: Scope) -> (Scope, Vec<Row>) {
	let persisted = persisted_daemons(&scope.runtime_dir);
	let spec_for = |name: &str| {
		persisted
			.iter()
			.find(|(n, ..)| n == name)
			.map(|(_, _, spec)| spec)
	};
	if scope.broker_pid.is_some()
		&& let Ok(mut client) = Client::connect(&scope.runtime_dir)
	{
		let live = (|| -> Result<Vec<Value>> {
			if scope.project_dir.is_none()
				&& let Ok(ping) = client.request(json!({ "op": "ping" }), 30_000)
			{
				scope.project_dir = ping
					.get("projectDir")
					.and_then(Value::as_str)
					.map(str::to_owned);
			}
			let result = client.request(json!({ "op": "list" }), 30_000)?;
			Ok(result
				.get("daemons")
				.and_then(Value::as_array)
				.cloned()
				.unwrap_or_default())
		})();
		if let Ok(daemons) = live {
			let rows = daemons
				.into_iter()
				.map(|snapshot| {
					let name = snapshot
						.get("name")
						.and_then(Value::as_str)
						.unwrap_or_default()
						.to_owned();
					let spec = spec_for(&name);
					Row {
						command: format_command(spec),
						cwd: spec.and_then(|s| s.get("cwd")?.as_str().map(str::to_owned)),
						snapshot,
						supervised: true,
					}
				})
				.collect();
			return (scope, rows);
		}
	}
	let mut rows: Vec<Row> = persisted
		.iter()
		.map(|(_, snapshot, spec)| {
			let mut snapshot = snapshot.clone();
			let state = state_of(&snapshot).to_owned();
			if !is_terminal_state(&state) {
				let detached = spec
					.get("detached")
					.and_then(Value::as_bool)
					.unwrap_or(false);
				let alive = snapshot
					.get("pid")
					.and_then(Value::as_i64)
					.is_some_and(|pid| paths::probe_pid(pid).is_ok());
				if !(detached && state != "stopping" && alive)
					&& let Some(object) = snapshot.as_object_mut()
				{
					object.insert("state".into(), json!("exited"));
					if object.get("exitReason").is_none_or(Value::is_null) {
						object.insert("exitReason".into(), json!("broker exited"));
					}
				}
			}
			Row {
				snapshot,
				command: format_command(Some(spec)),
				cwd: spec.get("cwd").and_then(Value::as_str).map(str::to_owned),
				supervised: false,
			}
		})
		.collect();
	rows.sort_by(|a, b| {
		let (at, bt) =
			(is_terminal_state(state_of(&a.snapshot)), is_terminal_state(state_of(&b.snapshot)));
		at.cmp(&bt).then_with(|| {
			let name = |row: &Row| {
				row.snapshot
					.get("name")
					.and_then(Value::as_str)
					.unwrap_or_default()
					.to_owned()
			};
			name(a).cmp(&name(b))
		})
	});
	(scope, rows)
}

// ───────────────────────────── list ─────────────────────────────

fn run_list(args: &PsArgs, paint: &Paint) -> Result<u8> {
	let scopes = if args.all {
		discover_scopes()
	} else {
		vec![target_scope(args)?]
	};
	let reports: Vec<(Scope, Vec<Row>)> = scopes
		.into_iter()
		.map(collect_scope)
		.filter(|(scope, rows)| !args.all || !rows.is_empty() || scope.broker_pid.is_some())
		.collect();
	if args.json {
		let value: Vec<Value> = reports
			.iter()
			.map(|(scope, rows)| {
				let mut object = serde_json::Map::new();
				object.insert("kind".into(), json!(if scope.global { "global" } else { "project" }));
				if let Some(dir) = &scope.project_dir {
					object.insert("projectDir".into(), json!(dir));
				}
				if let Some(service) = &scope.service {
					object.insert("service".into(), json!(service));
				}
				object.insert("runtimeDir".into(), json!(scope.runtime_dir.to_string_lossy()));
				if let Some(pid) = scope.broker_pid {
					object.insert("brokerPid".into(), json!(pid));
				}
				let daemons: Vec<Value> = rows
					.iter()
					.map(|row| {
						let mut daemon = row.snapshot.as_object().cloned().unwrap_or_default();
						if let Some(command) = &row.command {
							daemon.insert("command".into(), json!(command));
						}
						if let Some(cwd) = &row.cwd {
							daemon.insert("cwd".into(), json!(cwd));
						}
						daemon.insert("supervised".into(), json!(row.supervised));
						Value::Object(daemon)
					})
					.collect();
				object.insert("daemons".into(), Value::Array(daemons));
				Value::Object(object)
			})
			.collect();
		println!("{}", serde_json::to_string_pretty(&value)?);
		return Ok(0);
	}
	if reports.is_empty() {
		println!("{}", paint.dim("No daemon broker scopes found."));
		return Ok(0);
	}
	for (index, (scope, rows)) in reports.iter().enumerate() {
		if index > 0 {
			println!();
		}
		println!("{}", scope_header(scope, paint));
		if rows.is_empty() {
			println!("{}", paint.dim("  no processes"));
			continue;
		}
		print_table(rows, paint);
	}
	if !args.all {
		println!("{}", paint.dim("\nUse --all to include other projects and global services."));
	}
	Ok(0)
}

fn scope_header(scope: &Scope, paint: &Paint) -> String {
	let fallback = scope
		.runtime_dir
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default();
	let label = if scope.global {
		format!("global {}", paint.bold(scope.service.as_deref().unwrap_or(&fallback)))
	} else {
		format!("project {}", paint.bold(scope.project_dir.as_deref().unwrap_or(&fallback)))
	};
	let broker = match scope.broker_pid {
		Some(pid) => paint.green(&format!("broker pid {pid}")),
		None => paint.dim("broker not running"),
	};
	format!("{label} {} {broker}", paint.dim("—"))
}

fn table_cells(row: &Row) -> [String; 7] {
	let snapshot = &row.snapshot;
	let state = state_of(snapshot);
	let terminal = is_terminal_state(state);
	let mut state_text = state.to_owned();
	if terminal && let Some(code) = snapshot.get("exitCode").and_then(Value::as_i64) {
		state_text.push_str(&format!("({code})"));
	}
	let pid = snapshot
		.get("pid")
		.and_then(Value::as_i64)
		.filter(|_| !terminal)
		.map_or("-".to_owned(), |pid| pid.to_string());
	let uptime = if terminal {
		"-".to_owned()
	} else {
		let started = snapshot
			.get("startedAt")
			.and_then(Value::as_f64)
			.unwrap_or(0.0);
		paths::format_duration(paths::now_ms() - started)
	};
	let mut flags = Vec::new();
	if snapshot.get("detached").and_then(Value::as_bool) == Some(true) {
		flags.push("detached");
	} else if snapshot.get("persist").and_then(Value::as_bool) == Some(true) {
		flags.push("persist");
	}
	if !row.supervised && !terminal {
		flags.push("unsupervised");
	}
	let command = row
		.command
		.as_deref()
		.map_or(String::new(), |c| c.split_whitespace().collect::<Vec<_>>().join(" "));
	[
		snapshot
			.get("name")
			.and_then(Value::as_str)
			.unwrap_or_default()
			.to_owned(),
		state_text,
		pid,
		uptime,
		snapshot
			.get("restartCount")
			.and_then(Value::as_i64)
			.unwrap_or(0)
			.to_string(),
		flags.join(","),
		command,
	]
}

fn paint_state(state: &str, text: &str, paint: &Paint) -> String {
	match state {
		"ready" | "running" => paint.green(text),
		"failed" => paint.red(text),
		"exited" => paint.dim(text),
		_ => paint.yellow(text),
	}
}

fn print_table(rows: &[Row], paint: &Paint) {
	let max_width = paths::terminal_columns();
	let cells: Vec<[String; 7]> = rows.iter().map(table_cells).collect();
	let widths: Vec<usize> = (0..TABLE_HEADER.len())
		.map(|column| {
			cells
				.iter()
				.map(|row| row[column].chars().count())
				.fold(TABLE_HEADER[column].len(), usize::max)
		})
		.collect();
	let render = |row: &[String]| -> String {
		let padded: Vec<String> = row
			.iter()
			.enumerate()
			.map(|(column, cell)| format!("{cell:<width$}", width = widths[column]))
			.collect();
		let line = format!("  {}", padded.join("  ")).trim_end().to_owned();
		match max_width {
			Some(width) if line.chars().count() > width => {
				line
					.chars()
					.take(width.saturating_sub(1))
					.collect::<String>()
					+ "…"
			},
			_ => line,
		}
	};
	let header: Vec<String> = TABLE_HEADER
		.iter()
		.map(|title| (*title).to_owned())
		.collect();
	println!("{}", paint.dim(&render(&header)));
	for (row, cell) in rows.iter().zip(&cells) {
		let line = render(cell);
		let state = state_of(&row.snapshot);
		if is_terminal_state(state) {
			println!("{}", paint.dim(&line));
		} else if paint.enabled && line.contains(&cell[1]) {
			// Color only the STATE cell, keeping alignment computed on plain text.
			let offset = 2 + widths[0] + 2;
			let (before, after) = line.split_at(offset.min(line.len()));
			let rest = after.strip_prefix(cell[1].as_str()).unwrap_or(after);
			println!("{before}{}{rest}", paint_state(state, &cell[1], paint));
		} else {
			println!("{line}");
		}
	}
}

// ───────────────────────────── actions ─────────────────────────────

fn daemon_label(daemon: &Value) -> String {
	let name = daemon
		.get("name")
		.and_then(Value::as_str)
		.unwrap_or_default();
	let pid = daemon
		.get("pid")
		.and_then(Value::as_i64)
		.map_or(String::new(), |pid| format!(" pid={pid}"));
	let exit = daemon
		.get("exitCode")
		.and_then(Value::as_i64)
		.map_or(String::new(), |code| format!(" exit={code}"));
	format!("{name}: {}{pid}{exit}", state_of(daemon))
}

fn display(value: Option<&Value>) -> String {
	match value {
		Some(Value::String(text)) => text.clone(),
		Some(Value::Null) | None => "undefined".to_owned(),
		Some(other) => other.to_string(),
	}
}

fn run_action(args: &PsArgs, action: PsAction, name: &str, paint: &Paint) -> Result<()> {
	let scope = target_scope(args)?;
	let mut client = Client::connect(&scope.runtime_dir)
		.context("actions require a running broker (broker revival is not ported)")?;
	match action {
		PsAction::Info => {
			let result = client.request(json!({ "op": "describe", "name": name }), 30_000)?;
			let daemon = result.get("daemon").cloned().unwrap_or(Value::Null);
			let spec = result.get("spec").cloned().unwrap_or(Value::Null);
			if args.json {
				let mut object = daemon.as_object().cloned().unwrap_or_default();
				object.insert("spec".into(), spec);
				println!("{}", serde_json::to_string_pretty(&Value::Object(object))?);
				return Ok(());
			}
			println!("{}", daemon_label(&daemon));
			println!("  command:  {}", format_command(Some(&spec)).unwrap_or_default());
			println!("  cwd:      {}", display(spec.get("cwd")));
			if !is_terminal_state(state_of(&daemon)) {
				let started = daemon
					.get("startedAt")
					.and_then(Value::as_f64)
					.unwrap_or(0.0);
				println!("  uptime:   {}", paths::format_duration(paths::now_ms() - started));
			}
			if let Some(reason) = daemon
				.get("exitReason")
				.and_then(Value::as_str)
				.filter(|r| !r.is_empty())
			{
				println!("  exit:     {reason}");
			}
			println!(
				"  restarts: {} (policy: {})",
				daemon
					.get("restartCount")
					.and_then(Value::as_i64)
					.unwrap_or(0),
				display(spec.get("restart"))
			);
			println!(
				"  pty: {}  persist: {}  detached: {}  owner: {}",
				display(spec.get("pty")),
				display(spec.get("persist")),
				display(spec.get("detached")),
				daemon.get("owner").and_then(Value::as_str).unwrap_or("-")
			);
		},
		PsAction::Logs => run_logs(args, &mut client, name, paint)?,
		PsAction::Stop | PsAction::Kill => {
			let timeout_ms = if action == PsAction::Kill {
				KILL_GRACE_MS
			} else {
				(args.timeout.unwrap_or(5.0) * 1000.0).round().max(0.0) as u64
			};
			let result = client.request(
				json!({ "op": "stop", "name": name, "timeoutMs": timeout_ms }),
				timeout_ms + 5_000,
			)?;
			let verb = if action == PsAction::Kill {
				"Killed"
			} else {
				"Stopped"
			};
			print_daemon_result(args, verb, result.get("daemon"))?;
		},
		PsAction::Restart => {
			let result = client.request(json!({ "op": "restart", "name": name }), 30_000)?;
			print_daemon_result(args, "Restarted", result.get("daemon"))?;
		},
		PsAction::List => unreachable!("list handled before actions"),
	}
	Ok(())
}

fn print_daemon_result(args: &PsArgs, verb: &str, daemon: Option<&Value>) -> Result<()> {
	let daemon = daemon.cloned().unwrap_or(Value::Null);
	if args.json {
		println!("{}", serde_json::to_string_pretty(&daemon)?);
	} else {
		println!("{verb} {}", daemon_label(&daemon));
	}
	Ok(())
}

fn run_logs(args: &PsArgs, client: &mut Client, name: &str, paint: &Paint) -> Result<()> {
	let lines = args.lines.unwrap_or(100).clamp(1, 1_000);
	let mut request = json!({
		"op": "logs", "name": name,
		"lines": if args.follow { 1_000 } else { lines },
		"head": args.head, "follow": false,
		"renderTerminalRows": !args.follow, "timeoutMs": 30_000,
	});
	if let Some(grep) = &args.grep {
		request["grep"] = json!(grep);
	}
	let first = client.request(request, 35_000)?;
	let text_of = |value: &Value| {
		value
			.get("text")
			.and_then(Value::as_str)
			.unwrap_or_default()
			.to_owned()
	};
	let state_text = |value: &Value| {
		value
			.get("state")
			.and_then(Value::as_str)
			.unwrap_or("unknown")
			.to_owned()
	};
	if !args.follow {
		let text = match first.get("terminalRows").and_then(Value::as_array) {
			Some(rows) => rows
				.iter()
				.map(|row| row.as_str().unwrap_or_default())
				.collect::<Vec<_>>()
				.join("\n"),
			None => text_of(&first)
				.strip_suffix('\n')
				.map(str::to_owned)
				.unwrap_or_else(|| text_of(&first)),
		};
		if !text.is_empty() {
			println!("{text}");
		}
		println!("{}", paint.dim(&format!("[{name}: {}]", state_text(&first))));
		return Ok(());
	}
	let first_text = text_of(&first);
	let trimmed = first_text.strip_suffix('\n').unwrap_or(&first_text);
	let all: Vec<&str> = trimmed.split('\n').collect();
	let initial = all[all.len().saturating_sub(lines as usize)..].join("\n");
	let mut stdout = std::io::stdout();
	if !initial.is_empty() {
		writeln!(stdout, "{initial}")?;
	}
	let mut previous = first_text;
	let mut cursor = first.get("cursor").cloned().unwrap_or(json!(0));
	let mut state = state_text(&first);
	while !is_terminal_state(&state) {
		let mut request = json!({
			"op": "logs", "name": name, "lines": 1_000, "head": false, "follow": true,
			"cursor": cursor, "renderTerminalRows": false, "timeoutMs": 30_000,
		});
		if let Some(grep) = &args.grep {
			request["grep"] = json!(grep);
		}
		let next = client.request(request, 35_000)?;
		let next_text = text_of(&next);
		let fresh = &next_text[overlap_length(&previous, &next_text)..];
		if !fresh.is_empty() {
			if fresh.ends_with('\n') {
				write!(stdout, "{fresh}")?;
			} else {
				writeln!(stdout, "{fresh}")?;
			}
			stdout.flush()?;
		}
		previous = next_text;
		cursor = next.get("cursor").cloned().unwrap_or(cursor);
		state = state_text(&next);
	}
	println!("{}", paint.dim(&format!("[{name}: {state}]")));
	Ok(())
}

/// Longest suffix of `previous` that is a prefix of `next` (byte-wise,
/// char-boundary safe).
fn overlap_length(previous: &str, next: &str) -> usize {
	let max = previous.len().min(next.len());
	(1..=max)
		.rev()
		.find(|&k| {
			next.is_char_boundary(k)
				&& previous.as_bytes()[previous.len() - k..] == next.as_bytes()[..k]
		})
		.unwrap_or(0)
}
