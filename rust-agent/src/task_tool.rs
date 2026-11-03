//! `task` (synchronous subagents), `ask`, `yield` and `goal` tools
//! (`tools/task`, `tools/ask.ts`, `tools/yield.ts`, goal mode).
//!
//! Subagents reuse the parent's model and the parent's [`Approval`]: a
//! subagent's write/edit/bash call is approved exactly like the parent's.
use std::{
	cell::Cell,
	io::{self, IsTerminal, Write},
	path::Path,
	sync::{Mutex, MutexGuard, PoisonError},
};

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::{agent::RunOptions, provider::Model, tools::Approval};

/// Nested subagents beyond this depth are refused.
const MAX_DEPTH: u32 = 2;
/// Step budget for one subagent run.
const SUBAGENT_STEPS: u32 = 64;

thread_local! {
	static DEPTH: Cell<u32> = const { Cell::new(0) };
}

#[derive(Deserialize)]
struct TaskArgs {
	task:    String,
	agent:   Option<String>,
	name:    Option<String>,
	context: Option<String>,
}

/// Run one subagent to completion and return its yielded (or final) output.
pub fn run(
	root: &Path,
	arguments: &str,
	model: &dyn Model,
	approval: &dyn Approval,
	parent: &RunOptions,
) -> Result<String> {
	let args: TaskArgs = serde_json::from_str(arguments)
		.map_err(|error| anyhow::anyhow!("invalid task arguments JSON: {error}"))?;
	let task = args.task.trim();
	if task.is_empty() {
		bail!("task is empty");
	}
	let agent = args.agent.as_deref().unwrap_or("task");
	let Some(body) = crate::commands::agents::bundled_agent_body(agent) else {
		bail!("unknown agent {agent}");
	};
	let name = args
		.name
		.as_deref()
		.filter(|name| {
			!name.is_empty()
				&& name.len() <= 32
				&& name.chars().all(|c| c.is_ascii_alphanumeric())
		})
		.unwrap_or("Subagent")
		.to_owned();
	let depth = DEPTH.with(Cell::get);
	if depth >= MAX_DEPTH {
		bail!("subagent nesting limit ({MAX_DEPTH}) reached; do the work directly");
	}
	let mut prompt = format!("<agent name=\"{agent}\">\n{}\n</agent>\n\n", body.trim());
	if let Some(context) = args.context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
		prompt.push_str(&format!("<context>\n{context}\n</context>\n\n"));
	}
	prompt.push_str(task);
	prompt.push_str("\n\nWhen finished, call `yield` with your result.");
	let stamp = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_nanos())
		.unwrap_or_default();
	let path = format!(".omp/subagents/{name}-{stamp}.jsonl");
	let options = RunOptions {
		max_steps:      SUBAGENT_STEPS.min(parent.max_steps.max(1) * 4),
		user_agent_dir: parent.user_agent_dir.clone(),
		tags:           parent.tags.clone(),
		cancellation:   parent.cancellation.clone(),
	};
	DEPTH.with(|d| d.set(depth + 1));
	let result = (|| {
		let mut session =
			crate::agent_session::AgentSession::open(root, Path::new(&path), None)?;
		let answer = session.submit(&prompt, model, approval, &options, &mut |_| Ok(()))?;
		Ok::<_, anyhow::Error>((answer, session.take_yield()))
	})();
	DEPTH.with(|d| d.set(depth));
	let (answer, yielded) = result?;
	match yielded {
		Some(Ok(data)) => Ok(format!("[{name}] completed\n{data}")),
		Some(Err(error)) => bail!("[{name}] failed: {error}"),
		None => Ok(format!("[{name}] completed\n{answer}")),
	}
}

#[derive(Deserialize)]
struct AskArgs {
	questions: Vec<Question>,
}

#[derive(Deserialize)]
struct Question {
	id:          String,
	question:    String,
	#[serde(default)]
	options:     Vec<AskOption>,
	#[serde(default)]
	multi:       bool,
	recommended: Option<usize>,
}

#[derive(Deserialize)]
struct AskOption {
	label:       String,
	description: Option<String>,
}

/// Ask the user on the controlling terminal. Without one, the model is told to
/// proceed with the conservative default rather than block.
pub fn ask(arguments: &str) -> Result<String> {
	let args: AskArgs = serde_json::from_str(arguments)
		.map_err(|error| anyhow::anyhow!("invalid ask arguments JSON: {error}"))?;
	if args.questions.is_empty() {
		bail!("ask requires at least one question");
	}
	if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
		bail!(
			"ask requires an interactive terminal; pick the most conservative option, proceed, and \
			 state your choice"
		);
	}
	let mut stderr = io::stderr();
	let mut answers = Vec::new();
	for question in &args.questions {
		writeln!(stderr, "\n{}", question.question)?;
		for (index, option) in question.options.iter().enumerate() {
			let recommended = if question.recommended == Some(index) { " (Recommended)" } else { "" };
			let description =
				option.description.as_deref().map(|d| format!(" — {d}")).unwrap_or_default();
			writeln!(stderr, "  {}. {}{recommended}{description}", index + 1, option.label)?;
		}
		writeln!(stderr, "  {}. Other (type your own)", question.options.len() + 1)?;
		write!(
			stderr,
			"{}> ",
			if question.multi { "Numbers separated by commas, or text " } else { "" }
		)?;
		stderr.flush()?;
		let mut line = String::new();
		io::stdin().read_line(&mut line)?;
		let line = line.trim();
		let picked: Option<Vec<&str>> = line
			.split(',')
			.map(|part| {
				part.trim()
					.parse::<usize>()
					.ok()
					.and_then(|n| n.checked_sub(1))
					.and_then(|n| question.options.get(n))
					.map(|option| option.label.as_str())
			})
			.collect();
		let answer = match picked {
			Some(labels) if !labels.is_empty() && (question.multi || labels.len() == 1) =>
				labels.join(", "),
			_ if line.is_empty() => match question.recommended.and_then(|n| question.options.get(n)) {
				Some(option) => option.label.clone(),
				None => "(no answer)".to_owned(),
			},
			_ => format!("Other: {line}"),
		};
		answers.push(format!("{}: {answer}", question.id));
	}
	Ok(format!("User answers:\n{}", answers.join("\n")))
}

#[derive(Default)]
pub struct AgentState {
	inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
	yielded: Option<Result<String, String>>,
	goal:    Option<Goal>,
}

struct Goal {
	objective:    String,
	token_budget: Option<u64>,
	paused:       bool,
}

#[derive(Deserialize)]
struct YieldArgs {
	data:  Option<serde_json::Value>,
	error: Option<String>,
}

#[derive(Deserialize)]
struct GoalArgs {
	op:           String,
	objective:    Option<String>,
	token_budget: Option<f64>,
}

impl AgentState {
	fn lock(&self) -> MutexGuard<'_, Inner> {
		self.inner.lock().unwrap_or_else(PoisonError::into_inner)
	}

	pub fn take(&self) -> Option<Result<String, String>> {
		self.lock().yielded.take()
	}

	/// `yield`: record the subagent's terminal result.
	pub fn submit(&self, arguments: &str) -> Result<String> {
		let args: YieldArgs = serde_json::from_str(arguments)
			.map_err(|error| anyhow::anyhow!("invalid yield arguments JSON: {error}"))?;
		let outcome = match (args.data, args.error) {
			(Some(_), Some(_)) => bail!("yield takes `data` or `error`, never both"),
			(_, Some(error)) => Err(error),
			(Some(serde_json::Value::String(text)), None) => Ok(text),
			(Some(data), None) => Ok(serde_json::to_string_pretty(&data)?),
			(None, None) => bail!("yield requires `data` or `error`"),
		};
		self.lock().yielded = Some(outcome);
		Ok("Result submitted. End your turn now.".to_owned())
	}

	/// `goal`: manage the goal-mode objective.
	pub fn goal(&self, arguments: &str) -> Result<String> {
		let args: GoalArgs = serde_json::from_str(arguments)
			.map_err(|error| anyhow::anyhow!("invalid goal arguments JSON: {error}"))?;
		let mut inner = self.lock();
		match args.op.as_str() {
			"create" => {
				if inner.goal.is_some() {
					bail!("A goal already exists; complete, resume, or drop it first.");
				}
				let objective = args.objective.as_deref().map(str::trim).unwrap_or_default();
				if objective.is_empty() {
					bail!("create requires `objective`");
				}
				let token_budget = match args.token_budget {
					Some(budget) if budget >= 1.0 && budget.is_finite() => Some(budget as u64),
					Some(_) => bail!("token_budget must be a positive integer"),
					None => None,
				};
				inner.goal = Some(Goal { objective: objective.to_owned(), token_budget, paused: false });
				Ok(format!("Goal created: {objective}"))
			},
			"get" => Ok(match &inner.goal {
				None => "No active goal.".to_owned(),
				Some(goal) => format!(
					"Goal ({}): {}\nToken budget: {}",
					if goal.paused { "paused" } else { "active" },
					goal.objective,
					goal.token_budget.map_or_else(|| "unlimited".to_owned(), |b| b.to_string())
				),
			}),
			"resume" => match &mut inner.goal {
				None => bail!("No goal to resume."),
				Some(goal) => {
					goal.paused = false;
					Ok(format!("Goal resumed: {}", goal.objective))
				},
			},
			"complete" => match inner.goal.take() {
				None => bail!("No active goal."),
				Some(goal) => Ok(format!("Goal complete: {}", goal.objective)),
			},
			"drop" => match inner.goal.take() {
				None => bail!("No active goal."),
				Some(goal) => Ok(format!("Goal dropped: {}", goal.objective)),
			},
			op => bail!("unknown goal op {op}"),
		}
	}
}
