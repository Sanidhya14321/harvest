//! `harvest agents unpack` — write bundled agent definitions to disk (`agents-cli.ts`).
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use harvest_runtime::JailedFs;
use serde_yaml_ng::{Mapping, Value};

use super::{CONFIG_DIR_NAME, Paint};

const SCOUT: &str = include_str!("../../../packages/coding-agent/src/prompts/agents/scout.md");
const REVIEWER: &str = include_str!("../../../packages/coding-agent/src/prompts/agents/reviewer.md");
const SECURITY_REVIEWER: &str =
	include_str!("../../../packages/coding-agent/src/prompts/agents/security-reviewer.md");
const TASK: &str = include_str!("../../../packages/coding-agent/src/prompts/agents/task.md");

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum AgentsAction {
	Unpack,
}

#[derive(Debug, Args)]
pub struct AgentsArgs {
	/// Agents action
	#[arg(value_enum)]
	pub action:  Option<AgentsAction>,
	/// Overwrite existing agent files
	#[arg(short, long)]
	pub force:   bool,
	/// Output JSON
	#[arg(long)]
	pub json:    bool,
	/// Output directory (overrides --user/--project)
	#[arg(long)]
	pub dir:     Option<String>,
	/// Write to ~/.omp/agent/agents (default)
	#[arg(long)]
	pub user:    bool,
	/// Write to ./.omp/agents
	#[arg(long)]
	pub project: bool,
}

struct Agent {
	name:        String,
	frontmatter: Mapping,
	body:        String,
}

fn split_frontmatter(content: &str) -> (Mapping, String) {
	let normalized = content.replace("\r\n", "\n");
	if let Some(rest) = normalized.strip_prefix("---\n")
		&& let Some(end) = rest.find("\n---")
	{
		let yaml = &rest[..end];
		let body = rest[end + 4..].trim_start_matches(['\n']).to_owned();
		let map = serde_yaml_ng::from_str::<Mapping>(yaml).unwrap_or_default();
		return (map, body);
	}
	(Mapping::new(), normalized)
}

fn csv_list(value: &Value) -> Option<Value> {
	match value {
		Value::String(s) => {
			let items: Vec<Value> = s
				.split(',')
				.map(str::trim)
				.filter(|s| !s.is_empty())
				.map(|s| Value::String(s.to_owned()))
				.collect();
			(!items.is_empty()).then_some(Value::Sequence(items))
		},
		Value::Sequence(items) if !items.is_empty() => Some(value.clone()),
		_ => None,
	}
}

/// `parseAgentFields` → `toFrontmatter` projection.
fn to_frontmatter(source: &Mapping) -> (String, Mapping) {
	let get = |key: &str| source.get(key);
	let name = get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
	let mut out = Mapping::new();
	out.insert("name".into(), Value::String(name.clone()));
	out.insert("description".into(), get("description").cloned().unwrap_or(Value::String(String::new())));
	let tools = get("tools").and_then(csv_list);
	if let Some(tools) = &tools {
		out.insert("tools".into(), tools.clone());
	}
	let spawns = match get("spawns") {
		Some(Value::String(s)) if s.trim() == "*" => Some(Value::String("*".into())),
		Some(v) => csv_list(v),
		None => tools
			.as_ref()
			.and_then(Value::as_sequence)
			.filter(|t| t.iter().any(|v| v.as_str() == Some("task")))
			.map(|_| Value::String("*".into())),
	};
	if let Some(spawns) = spawns {
		out.insert("spawns".into(), spawns);
	}
	if let Some(model) = get("model").and_then(csv_list) {
		out.insert("model".into(), model);
	}
	if let Some(level) = get("thinking-level").or_else(|| get("thinkingLevel")).filter(|v| !v.is_null()) {
		out.insert("thinkingLevel".into(), level.clone());
	}
	if let Some(output) = get("output") {
		out.insert("output".into(), output.clone());
	}
	if get("blocking").and_then(Value::as_bool) == Some(true) {
		out.insert("blocking".into(), Value::Bool(true));
	}
	(name, out)
}

fn bundled_agents() -> Vec<Agent> {
	let mut agents = Vec::new();
	for content in [SCOUT, REVIEWER, SECURITY_REVIEWER] {
		let (fm, body) = split_frontmatter(content);
		let (name, frontmatter) = to_frontmatter(&fm);
		agents.push(Agent { name, frontmatter, body });
	}
	let (_, task_body) = split_frontmatter(TASK);
	for (name, description, spawns, model, level) in [
		(
			"task",
			"General-purpose subagent with full capabilities for delegated multi-step tasks",
			Some("*"),
			"@task",
			"auto",
		),
		(
			"sonic",
			"Low-reasoning agent for strictly mechanical updates or data collection only",
			None,
			"@smol",
			"medium",
		),
	] {
		let mut fm = Mapping::new();
		fm.insert("name".into(), name.into());
		fm.insert("description".into(), description.into());
		if let Some(spawns) = spawns {
			fm.insert("spawns".into(), spawns.into());
		}
		fm.insert("model".into(), model.into());
		fm.insert("thinking-level".into(), level.into());
		let (name, frontmatter) = to_frontmatter(&fm);
		agents.push(Agent { name, frontmatter, body: task_body.clone() });
	}
	agents.sort_by(|a, b| a.name.cmp(&b.name));
	agents
}

fn serialize(agent: &Agent) -> Result<String> {
	let yaml = serde_yaml_ng::to_string(&agent.frontmatter)?;
	Ok(format!("---\n{}\n---\n\n{}\n", yaml.trim_end(), agent.body.trim()))
}

fn target_dir(args: &AgentsArgs) -> Result<PathBuf> {
	let cwd = std::env::current_dir()?;
	if let Some(dir) = args.dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
		return Ok(cwd.join(dir));
	}
	if args.user && args.project {
		bail!("Choose either --user or --project, not both.");
	}
	if args.project {
		return Ok(cwd.join(CONFIG_DIR_NAME).join("agents"));
	}
	Ok(super::agent_dir()?.join("agents"))
}

pub fn execute(args: AgentsArgs) -> Result<u8> {
	let Some(AgentsAction::Unpack) = args.action else {
		use clap::CommandFactory;
		#[derive(clap::Parser)]
		#[command(name = "omp agents", about = "Manage bundled task agents")]
		struct Help {
			#[command(flatten)]
			args: AgentsArgs,
		}
		Help::command().print_help()?;
		println!();
		return Ok(0);
	};
	let dir = target_dir(&args)?;
	let fs = JailedFs::open_or_create(&dir)?;
	let agents = bundled_agents();
	let mut written = Vec::new();
	let mut skipped = Vec::new();
	for agent in &agents {
		let file = format!("{}.md", agent.name);
		let full = dir.join(&file).display().to_string();
		if !args.force && fs.entry_exists(Path::new(&file))? {
			skipped.push(full);
			continue;
		}
		fs.atomic_write(Path::new(&file), serialize(agent)?.as_bytes())?;
		written.push(full);
	}
	if args.json {
		let payload = serde_json::json!({
			"targetDir": dir.display().to_string(),
			"total": agents.len(),
			"written": written,
			"skipped": skipped,
		});
		println!("{}", serde_json::to_string_pretty(&payload)?);
		return Ok(0);
	}
	let p = Paint::stdout();
	println!("{}", p.bold(&format!("Bundled agents: {}", agents.len())));
	println!("{}", p.dim(&format!("Target directory: {}", dir.display())));
	println!("{}", p.green(&format!("✔ Written: {}", written.len())));
	if !skipped.is_empty() {
		println!(
			"{}",
			p.yellow(&format!("⚠ Skipped existing: {} (use --force to overwrite)", skipped.len()))
		);
	}
	for path in &written {
		println!("{}", p.dim(&format!("  + {path}")));
	}
	for path in &skipped {
		println!("{}", p.dim(&format!("  = {path}")));
	}
	Ok(0)
}
