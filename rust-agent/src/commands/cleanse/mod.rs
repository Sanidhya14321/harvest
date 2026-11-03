//! `harvest cleanse` — detect project diagnostics, dispatch file-disjoint
//! repair agents, then verify. Ported from `commands/cleanse.ts` and
//! `cleanse/{index,balance,loop,agent,board}.ts`.

mod checkers;
mod parsers;

use std::{
	collections::{HashMap, HashSet},
	io::{BufRead, IsTerminal, Write},
	path::{Path, PathBuf},
	sync::{Mutex, mpsc},
};

use anyhow::{Context, Result, bail};

const DEFAULT_MODEL: &str = "@smol";
const DISPLAY_FILE_LIMIT: usize = 50;
const MAX_DIAGNOSTIC_MESSAGE: usize = 4_000;
const ERROR_WIDTH: usize = 300;
const ASSIGNMENT_PROMPT: &str = include_str!("../../prompts/cleanse/assignment.md");
const DISCOVERY_PROMPT: &str = include_str!("../../prompts/cleanse/discovery.md");

#[derive(Debug, clap::Args)]
#[command(after_help = "Examples:
  harvest cleanse
  harvest cleanse --all
  harvest cleanse \"ts errors\"
  harvest cleanse -n 8
  harvest cleanse -m opus
  harvest cleanse -t")]
pub struct Args {
	/// What to detect and fix (e.g. "ts errors"); a discovery agent works out the command
	request:   Option<String>,
	/// Maximum number of file-disjoint subagents
	#[arg(short = 'n', long, default_value_t = 32)]
	agents:    i64,
	/// Subagent model selector
	#[arg(short, long, default_value = DEFAULT_MODEL)]
	model:     String,
	/// Also run configured project test suites
	#[arg(short, long)]
	tests:     bool,
	/// Run every discovered checker without the interactive picker
	#[arg(short, long)]
	all:       bool,
	/// Agent directory override
	#[arg(long, hide = true)]
	agent_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
	Error,
	Warning,
	Info,
}

impl Severity {
	const fn as_str(self) -> &'static str {
		match self {
			Self::Error => "error",
			Self::Warning => "warning",
			Self::Info => "info",
		}
	}
	const fn weight(self) -> u32 {
		match self {
			Self::Error => 5,
			Self::Warning => 3,
			Self::Info => 1,
		}
	}
}

#[derive(Debug, Clone)]
#[allow(dead_code, reason = "mirrors the TS report shape")]
pub struct Diagnostic {
	pub checker:    String,
	pub file:       Option<String>,
	pub line:       Option<u32>,
	pub column:     Option<u32>,
	pub end_line:   Option<u32>,
	pub end_column: Option<u32>,
	pub code:       Option<String>,
	pub severity:   Severity,
	pub message:    String,
	pub suggestion: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code, reason = "mirrors the TS report shape")]
pub struct CheckResult {
	pub id:          String,
	pub label:       String,
	pub language:    String,
	pub cwd:         String,
	pub command:     String,
	pub exit_code:   Option<i32>,
	pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
#[allow(dead_code, reason = "mirrors the TS report shape")]
pub struct SkippedCheck {
	pub label:    String,
	pub language: String,
	pub reason:   String,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code, reason = "mirrors the TS report shape")]
pub struct Report {
	pub checks:      Vec<CheckResult>,
	pub diagnostics: Vec<Diagnostic>,
	pub skipped:     Vec<SkippedCheck>,
}

/// Identity key used for exactly-once dispatch and deduplication.
pub fn diagnostic_key(d: &Diagnostic) -> String {
	[
		d.file.clone().unwrap_or_default(),
		d.line.map(|v| v.to_string()).unwrap_or_default(),
		d.column.map(|v| v.to_string()).unwrap_or_default(),
		d.code.clone().unwrap_or_default(),
		d.message.clone(),
	]
	.join("\u{0}")
}

#[derive(Debug, Clone)]
struct FileIssues {
	file:        Option<String>,
	diagnostics: Vec<Diagnostic>,
	weight:      u32,
}

#[derive(Debug, Clone)]
struct Assignment {
	index:  usize,
	groups: Vec<FileIssues>,
	weight: u32,
}

struct Outcome {
	name:    String,
	success: bool,
	error:   Option<String>,
	model:   Option<String>,
}

fn diagnostic_weight(d: &Diagnostic) -> u32 {
	let mut weight = d.severity.weight();
	if d.line.is_none() {
		weight += 2;
	}
	if d.code.is_none() {
		weight += 1;
	}
	if d.suggestion.is_none() {
		weight += 1;
	}
	weight
}

fn group_by_file(diagnostics: &[Diagnostic]) -> Vec<FileIssues> {
	let mut order: Vec<String> = Vec::new();
	let mut groups: HashMap<String, Vec<Diagnostic>> = HashMap::new();
	for d in diagnostics {
		let key = d.file.clone().unwrap_or_default();
		if !groups.contains_key(&key) {
			order.push(key.clone());
		}
		groups.entry(key).or_default().push(d.clone());
	}
	let mut out: Vec<FileIssues> = order
		.into_iter()
		.map(|key| {
			let mut entries = groups.remove(&key).unwrap_or_default();
			entries.sort_by(|a, b| {
				a.line
					.unwrap_or(0)
					.cmp(&b.line.unwrap_or(0))
					.then(a.column.unwrap_or(0).cmp(&b.column.unwrap_or(0)))
					.then(a.checker.cmp(&b.checker))
					.then(a.message.cmp(&b.message))
			});
			let weight = entries.iter().map(diagnostic_weight).sum();
			FileIssues { file: (!key.is_empty()).then_some(key), diagnostics: entries, weight }
		})
		.collect();
	out.sort_by(|a, b| {
		b.weight
			.cmp(&a.weight)
			.then(a.file.as_deref().unwrap_or("").cmp(b.file.as_deref().unwrap_or("")))
	});
	out
}

const fn plural(n: usize) -> &'static str {
	if n == 1 { "" } else { "s" }
}

fn one_line(text: &str, width: usize) -> String {
	parsers::sanitize_text(text)
		.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ")
		.chars()
		.take(width)
		.collect()
}

fn format_duration(ms: u128) -> String {
	if ms < 1000 {
		format!("{ms}ms")
	} else if ms < 60_000 {
		format!("{:.1}s", ms as f64 / 1000.0)
	} else {
		format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
	}
}

/// Plain-line status board (the TS non-TTY protocol).
struct Board;

impl checkers::RunEvents for Board {
	fn checker_end(&self, check: &CheckResult, duration_ms: u128) {
		let count = check.diagnostics.len();
		let (glyph, verdict) = if count == 0 {
			("✓", "clean".to_owned())
		} else {
			("●", format!("{count} issue{}", plural(count)))
		};
		println!("{glyph} {} {verdict} · {}", check.label, format_duration(duration_ms));
	}
}

/// Single shared terminal approval for every mutating worker call; fails
/// closed when no human terminal is attached.
struct SharedApproval(Mutex<()>);

impl crate::tools::Approval for SharedApproval {
	fn approve(&self, call: &crate::chat::ToolCall) -> Result<bool> {
		let _guard = self.0.lock().map_err(|_| anyhow::anyhow!("approval lock poisoned"))?;
		crate::tools::TerminalApproval.approve(call)
	}
}

struct Runtime {
	root:      PathBuf,
	agent_dir: Option<PathBuf>,
	model:     Option<String>,
	display:   String,
	session:   PathBuf,
	approval:  SharedApproval,
}

impl Runtime {
	fn create(root: &Path, agent_dir: Option<PathBuf>, selector: &str) -> Result<Self> {
		// Role selectors (`@smol`) are not resolved natively yet; fall back to
		// the configured default model.
		let model = (!selector.starts_with('@')).then(|| selector.to_owned());
		let mut runtime = Self {
			root: root.to_path_buf(),
			agent_dir,
			model,
			display: String::new(),
			session: PathBuf::from(format!(
				".harvest/cleanse/{}",
				chrono::Local::now().format("%Y%m%dT%H%M%S")
			)),
			approval: SharedApproval(Mutex::new(())),
		};
		let (_, display) = runtime.http_model()?;
		runtime.display = display;
		Ok(runtime)
	}

	fn http_model(&self) -> Result<(crate::provider::HttpModel, String)> {
		let environment = crate::config::environment_from(|key| std::env::var(key).ok())?;
		let resolved = crate::configuration::load(
			&self.root,
			&crate::config::Overrides { model: self.model.clone(), ..Default::default() },
			&environment,
			&crate::configuration::LoadOptions {
				user_directory: self.agent_dir.clone(),
				environment_profile: std::env::var("OMP_PROFILE").ok(),
				..Default::default()
			},
		)?;
		let display = resolved.settings.model.clone();
		Ok((crate::provider::HttpModel::from_configuration(&self.root, &resolved, false, false)?, display))
	}

	fn run_agent(&self, name: &str, prompt: &str) -> Result<String> {
		let (model, _) = self.http_model()?;
		let path = self.session.join(format!("{name}.jsonl"));
		let mut session = crate::agent_session::AgentSession::open(&self.root, &path, None)?;
		let user_agent_dir = self.agent_dir.clone().filter(|p| p.is_dir());
		session.submit(prompt, &model, &self.approval, &crate::agent::RunOptions {
			max_steps: 200,
			user_agent_dir,
			tags: Default::default(),
			cancellation: Default::default(),
		}, &mut |_| Ok(()))
	}

	fn discover_checkers(&self, request: &str) -> Result<Vec<checkers::CustomSpec>> {
		let prompt = render(DISCOVERY_PROMPT, &serde_json::json!({ "request": request }))?;
		let answer = self
			.run_agent("CleanseDiscovery", &prompt)
			.map_err(|e| anyhow::anyhow!("Checker discovery failed: {e}"))?;
		Ok(parse_discovery_specs(&answer))
	}
}

fn render(template: &str, data: &serde_json::Value) -> Result<String> {
	let mut engine = handlebars::Handlebars::new();
	engine.register_escape_fn(handlebars::no_escape);
	Ok(engine.render_template(template, data)?)
}

fn parse_discovery_specs(answer: &str) -> Vec<checkers::CustomSpec> {
	let data = parsers::parse_json_values(answer)
		.into_iter()
		.rev()
		.find(|v| v.get("checkers").is_some_and(serde_json::Value::is_array));
	let Some(data) = data else { return Vec::new() };
	let mut specs = Vec::new();
	for value in data["checkers"].as_array().into_iter().flatten() {
		let command: Vec<String> = value
			.get("command")
			.and_then(|c| c.as_array())
			.map(|c| c.iter().filter_map(|p| p.as_str()).filter(|p| !p.is_empty()).map(str::to_owned).collect())
			.unwrap_or_default();
		let Some(label) = value.get("label").and_then(|l| l.as_str()).map(str::trim).filter(|l| !l.is_empty()) else {
			continue;
		};
		if command.is_empty() {
			continue;
		}
		let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
		specs.push(checkers::CustomSpec {
			label: label.to_owned(),
			language: text("language"),
			cwd: text("cwd"),
			command,
			parser: text("parser"),
		});
	}
	specs
}

fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
	diagnostics
		.iter()
		.map(|d| {
			let location = d.file.as_ref().map_or_else(
				|| "<project>".to_owned(),
				|file| match (d.line, d.column) {
					(Some(l), Some(c)) => format!("{file}:{l}:{c}"),
					(Some(l), None) => format!("{file}:{l}"),
					_ => file.clone(),
				},
			);
			let code = d.code.as_ref().map(|c| format!(" {c}")).unwrap_or_default();
			let message: String = d.message.chars().take(MAX_DIAGNOSTIC_MESSAGE).collect();
			let suggestion = d.suggestion.as_ref().map(|s| format!("\n  Suggested fix: {s}")).unwrap_or_default();
			format!("- [{}] {location} — {}{code}: {message}{suggestion}", d.severity.as_str(), d.checker)
		})
		.collect::<Vec<_>>()
		.join("\n")
}

fn render_assignment(assignment: &Assignment, peers: &[Assignment], worker: usize, checkers: &[checkers::Descriptor]) -> Result<String> {
	let has_project = assignment.groups.iter().any(|g| g.file.is_none());
	let files: Vec<&String> = assignment.groups.iter().filter_map(|g| g.file.as_ref()).collect();
	let mut scope: Vec<String> = if files.is_empty() {
		vec!["- No file is named by the project-level diagnostic.".into()]
	} else {
		files.iter().map(|f| format!("- {f}")).collect()
	};
	if has_project {
		scope.push("- Minimal additional files strictly required by project-level diagnostics.".into());
	}
	let mut seen = HashSet::new();
	let commands: Vec<String> = checkers
		.iter()
		.filter(|c| seen.insert(format!("{}\u{0}{}", c.cwd, c.command)))
		.map(|c| format!("- [{}] {}", c.cwd, c.command))
		.collect();
	let peer_lines: Vec<String> = peers
		.iter()
		.filter(|p| p.index != assignment.index)
		.map(|p| {
			let files = p.groups.iter().map(|g| g.file.clone().unwrap_or_else(|| "<project-level>".into())).collect::<Vec<_>>().join(", ");
			format!("- Worker {}: {files}", p.index + 1)
		})
		.collect();
	let diagnostics: Vec<Diagnostic> = assignment.groups.iter().flat_map(|g| g.diagnostics.clone()).collect();
	render(ASSIGNMENT_PROMPT, &serde_json::json!({
		"worker": worker,
		"write_scope": scope.join("\n"),
		"diagnostics": format_diagnostics(&diagnostics),
		"checker_commands": if commands.is_empty() { "- No command metadata available.".to_owned() } else { commands.join("\n") },
		"peer_assignments": if peer_lines.is_empty() { "- None.".to_owned() } else { peer_lines.join("\n") },
	}))
}

fn files_label(assignment: &Assignment) -> String {
	assignment.groups.iter().map(|g| g.file.clone().unwrap_or_else(|| "<project>".into())).collect::<Vec<_>>().join(", ")
}

/// Dispatch file-disjoint batches into a bounded worker pool.
fn run_repair(runtime: &Runtime, report: &Report, max_agents: usize, checkers: &[checkers::Descriptor]) -> usize {
	let mut seen = HashSet::new();
	let mut pending: Vec<Diagnostic> = report.diagnostics.iter().filter(|d| seen.insert(diagnostic_key(d))).cloned().collect();
	let (sender, receiver) = mpsc::channel::<(usize, Outcome)>();
	let mut in_flight: HashMap<usize, Assignment> = HashMap::new();
	let mut dispatched = 0usize;
	std::thread::scope(|scope| {
		loop {
			while !pending.is_empty() && in_flight.len() < max_agents {
				let groups = group_by_file(&pending);
				let total: u32 = groups.iter().map(|g| g.weight).sum();
				let budget = f64::from(total) / max_agents as f64;
				let mut batch = Vec::new();
				let mut weight = 0u32;
				for group in groups {
					if !batch.is_empty() && f64::from(weight) >= budget {
						break;
					}
					weight += group.weight;
					batch.push(group);
				}
				let taken: HashSet<Option<String>> = batch.iter().map(|g| g.file.clone()).collect();
				pending.retain(|d| !taken.contains(&d.file));
				dispatched += 1;
				let worker = dispatched;
				let assignment = Assignment { index: worker - 1, groups: batch, weight };
				let peers: Vec<Assignment> = in_flight.values().cloned().collect();
				let name = format!("CleanseA{worker}");
				println!("[start] {name}: {} (weight {})", files_label(&assignment), assignment.weight);
				let prompt = render_assignment(&assignment, &peers, worker, checkers);
				in_flight.insert(worker, assignment);
				let sender = sender.clone();
				scope.spawn(move || {
					let result = prompt.and_then(|prompt| runtime.run_agent(&name, &prompt));
					let outcome = match result {
						Ok(_) => Outcome { name, success: true, error: None, model: Some(runtime.display.clone()) },
						Err(error) => Outcome { name, success: false, error: Some(error.to_string()), model: None },
					};
					let _ = sender.send((worker, outcome));
				});
			}
			if in_flight.is_empty() {
				break;
			}
			let Ok((worker, outcome)) = receiver.recv() else { break };
			in_flight.remove(&worker);
			if outcome.success {
				println!("[done] {}{}", outcome.name, outcome.model.map(|m| format!(" ({m})")).unwrap_or_default());
			} else {
				eprintln!("[fail] {}: {}", outcome.name, one_line(outcome.error.as_deref().unwrap_or("subagent failed"), ERROR_WIDTH));
			}
		}
	});
	dispatched
}

fn print_skipped(skipped: &[SkippedCheck]) {
	for s in skipped {
		println!("- {}: skipped ({})", s.label, s.reason);
	}
}

fn print_remaining(report: &Report) {
	let groups = group_by_file(&report.diagnostics);
	eprintln!("Unresolved: {} diagnostic{}.", report.diagnostics.len(), plural(report.diagnostics.len()));
	for group in groups.iter().take(DISPLAY_FILE_LIMIT) {
		eprintln!("- {}: {}", group.file.as_deref().unwrap_or("<project>"), group.diagnostics.len());
	}
	if groups.len() > DISPLAY_FILE_LIMIT {
		eprintln!("- ... {} more files", groups.len() - DISPLAY_FILE_LIMIT);
	}
}

fn prompt_line(question: &str) -> Option<String> {
	eprint!("{question} ");
	let _ = std::io::stderr().flush();
	let mut line = String::new();
	std::io::stdin().lock().read_line(&mut line).ok()?;
	let line = line.trim().to_owned();
	(!line.is_empty()).then_some(line)
}

enum Choice {
	All,
	Checker(String),
	Request(String),
	Cancel,
}

fn pick_target(checkers: &[checkers::Descriptor]) -> Choice {
	eprintln!("Select what to cleanse:");
	eprintln!("  1) Run all {} discovered checker{}", checkers.len(), plural(checkers.len()));
	for (i, c) in checkers.iter().enumerate() {
		eprintln!("  {}) {}  {} · {}", i + 2, c.label, c.language, c.command);
	}
	eprintln!("  {}) Describe what to fix…  A discovery agent figures out the command to run", checkers.len() + 2);
	let Some(answer) = prompt_line(">") else { return Choice::Cancel };
	let Ok(n) = answer.parse::<usize>() else { return Choice::Cancel };
	if n == 1 {
		Choice::All
	} else if n >= 2 && n < checkers.len() + 2 {
		Choice::Checker(checkers[n - 2].id.clone())
	} else if n == checkers.len() + 2 {
		prompt_request().map_or(Choice::Cancel, Choice::Request)
	} else {
		Choice::Cancel
	}
}

fn prompt_request() -> Option<String> {
	prompt_line("Describe what to detect and fix (e.g. \"ts errors\"):")
}

pub fn execute(args: Args) -> Result<u8> {
	if args.agents <= 0 {
		bail!("--agents must be a positive integer");
	}
	let max_agents = usize::try_from(args.agents).context("--agents must be a positive integer")?;
	let model = if args.model.trim().is_empty() { DEFAULT_MODEL.to_owned() } else { args.model.trim().to_owned() };
	let cwd = std::env::current_dir()?;
	let agent_dir = crate::config::agent_dir_from(args.agent_dir.clone(), |k| std::env::var_os(k));
	let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
	let mut runtime: Option<Runtime> = None;
	let ensure = |runtime: &mut Option<Runtime>| -> Result<()> {
		if runtime.is_none() {
			println!("Resolving model {model}...");
			let created = Runtime::create(&cwd, agent_dir.clone(), &model)?;
			println!("Model: {}", created.display);
			println!("Session: {}", created.session.display());
			*runtime = Some(created);
		}
		Ok(())
	};

	let mut request = args.request.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned);
	let mut suite = None;
	if request.is_none() {
		println!("Detecting configured project checkers...");
		let mut discovered = checkers::discover(&cwd, args.tests);
		if !args.all && interactive {
			let list = discovered.checkers();
			if list.is_empty() {
				print_skipped(&discovered.skipped);
				println!("No supported checker with an available executable was found.");
				match prompt_request() {
					Some(answer) => request = Some(answer),
					None => return Ok(1),
				}
			} else {
				match pick_target(&list) {
					Choice::Cancel => {
						eprintln!("Cleanse cancelled.");
						return Ok(130);
					},
					Choice::All => suite = Some(discovered),
					Choice::Checker(id) => {
						discovered.select(&[id]);
						suite = Some(discovered);
					},
					Choice::Request(text) => request = Some(text),
				}
			}
		} else {
			suite = Some(discovered);
		}
	}
	if let Some(request) = &request {
		ensure(&mut runtime)?;
		let rt = runtime.as_ref().expect("runtime ensured");
		println!("Discovering checkers for \"{request}\"...");
		let specs = rt.discover_checkers(request)?;
		let built = checkers::build_custom(&cwd, &specs);
		for checker in built.checkers() {
			println!("[checker] {}: {}", checker.label, checker.command);
		}
		suite = Some(built);
	}
	let skipped = suite.as_ref().map(|s| s.skipped.clone()).unwrap_or_default();
	let Some(suite) = suite.filter(|s| !s.selected().is_empty()) else {
		print_skipped(&skipped);
		eprintln!(
			"{}",
			if request.is_some() {
				"Checker discovery produced no runnable command."
			} else {
				"No supported checker with an available executable was found."
			}
		);
		return Ok(1);
	};
	print_skipped(&suite.skipped);
	let report = suite.run(&Board);
	if !report.diagnostics.is_empty() {
		let files = group_by_file(&report.diagnostics).iter().filter(|g| g.file.is_some()).count();
		println!(
			"Checkers done: {} diagnostic{} across {files} file{}.",
			report.diagnostics.len(),
			plural(report.diagnostics.len()),
			plural(files)
		);
	}
	if report.diagnostics.is_empty() {
		println!("Clean: {} checker{} passed.", report.checks.len(), plural(report.checks.len()));
		return Ok(0);
	}
	ensure(&mut runtime)?;
	let rt = runtime.as_ref().expect("runtime ensured");
	let workers = run_repair(rt, &report, max_agents, &suite.selected());
	let verified = suite.run(&Board);
	println!("Verification: {} diagnostic{} remaining.", verified.diagnostics.len(), plural(verified.diagnostics.len()));
	if verified.diagnostics.is_empty() {
		println!(
			"{}",
			if workers == 0 { format!("Clean: {} checker{} passed.", verified.checks.len(), plural(verified.checks.len())) } else { "Clean: all detected diagnostics are resolved.".to_owned() }
		);
		return Ok(0);
	}
	print_remaining(&verified);
	Ok(1)
}
