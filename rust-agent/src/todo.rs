//! The `todo` tool: phased task state with the legacy ops, normalization,
//! all-or-nothing batches, Markdown round-trip and summary text.
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
	Pending,
	InProgress,
	Completed,
	Abandoned,
	Blocked,
}

impl Status {
	const fn is_open(self) -> bool {
		matches!(self, Self::Pending | Self::InProgress)
	}

	const fn is_closed(self) -> bool {
		matches!(self, Self::Completed | Self::Abandoned)
	}

	const fn name(self) -> &'static str {
		match self {
			Self::Pending => "pending",
			Self::InProgress => "in_progress",
			Self::Completed => "completed",
			Self::Abandoned => "abandoned",
			Self::Blocked => "blocked",
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
	pub content: String,
	pub status:  Status,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
	pub name:  String,
	pub tasks: Vec<Item>,
}

/// Session-scoped todo state.
#[derive(Debug, Default)]
pub struct TodoList(Mutex<Vec<Phase>>);

impl TodoList {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn phases(&self) -> Vec<Phase> {
		self
			.0
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.clone()
	}

	pub fn set(&self, phases: Vec<Phase>) {
		*self
			.0
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner) = phases;
	}
}

#[derive(Debug, Deserialize)]
struct ListEntry {
	phase: String,
	items: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
	op:     String,
	list:   Option<Vec<ListEntry>>,
	task:   Option<String>,
	phase:  Option<String>,
	items:  Option<Vec<String>>,
	reason: Option<String>,
}

const OPS: [&str; 9] =
	["init", "start", "done", "rm", "drop", "block", "unblock", "append", "view"];
const DEFAULT_INIT_PHASE: &str = "Tasks";

fn locate(phases: &[Phase], content: &str) -> Option<(usize, usize)> {
	phases.iter().enumerate().find_map(|(p, phase)| {
		phase
			.tasks
			.iter()
			.position(|task| task.content == content)
			.map(|t| (p, t))
	})
}

fn resolve_task(
	phases: &[Phase],
	content: Option<&str>,
	errors: &mut Vec<String>,
) -> Option<(usize, usize)> {
	let Some(content) = content.filter(|content| !content.is_empty()) else {
		errors.push("Missing task content".into());
		return None;
	};
	let hit = locate(phases, content);
	if hit.is_none() {
		let is_id = content
			.strip_prefix("task-")
			.is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
		if is_id {
			errors.push(format!(
				"Task \"{content}\" not found. Tasks are referenced by content, not by IDs — pass the \
				 task's full text from the previous result."
			));
		} else {
			let empty = phases.iter().all(|phase| phase.tasks.is_empty());
			let hint = if empty {
				" (todo list is empty — was it replaced or not yet created?)"
			} else {
				""
			};
			errors.push(format!("Task \"{content}\" not found{hint}"));
		}
	}
	hit
}

fn resolve_phase(phases: &[Phase], name: &str, errors: &mut Vec<String>) -> Option<usize> {
	if name.is_empty() {
		errors.push("Missing phase name".into());
		return None;
	}
	let index = phases.iter().position(|phase| phase.name == name);
	if index.is_none() {
		errors.push(format!("Phase \"{name}\" not found"));
	}
	index
}

/// `getTaskTargets` as `(phase, task)` indexes.
fn targets(phases: &[Phase], params: &Params, errors: &mut Vec<String>) -> Vec<(usize, usize)> {
	if let Some(task) = params.task.as_deref().filter(|task| !task.is_empty()) {
		return resolve_task(phases, Some(task), errors)
			.into_iter()
			.collect();
	}
	if let Some(phase) = params.phase.as_deref().filter(|phase| !phase.is_empty()) {
		return resolve_phase(phases, phase, errors)
			.map(|p| (0..phases[p].tasks.len()).map(|t| (p, t)).collect())
			.unwrap_or_default();
	}
	phases
		.iter()
		.enumerate()
		.flat_map(|(p, phase)| (0..phase.tasks.len()).map(move |t| (p, t)))
		.collect()
}

fn has(value: Option<&String>) -> bool {
	value.is_some_and(|value| !value.is_empty())
}

fn apply(mut phases: Vec<Phase>, params: &Params, errors: &mut Vec<String>) -> Vec<Phase> {
	match params.op.as_str() {
		"init" => {
			let synthesized;
			let list = match (&params.list, &params.items) {
				(Some(list), _) => list,
				(None, Some(items)) if !items.is_empty() => {
					synthesized = vec![ListEntry {
						phase: params
							.phase
							.clone()
							.unwrap_or_else(|| DEFAULT_INIT_PHASE.into()),
						items: items.clone(),
					}];
					&synthesized
				},
				_ => {
					errors.push("Missing list for init operation".into());
					return Vec::new();
				},
			};
			let (mut seen_phases, mut seen_tasks) = (Vec::new(), Vec::new());
			for entry in list {
				if seen_phases.contains(&&entry.phase) {
					errors.push(format!("Duplicate phase \"{}\" in init list", entry.phase));
				}
				seen_phases.push(&entry.phase);
				for content in &entry.items {
					if seen_tasks.contains(&content) {
						errors.push(format!("Duplicate task \"{content}\" in init list"));
					}
					seen_tasks.push(content);
				}
			}
			list
				.iter()
				.map(|entry| Phase {
					name:  entry.phase.clone(),
					tasks: entry
						.items
						.iter()
						.map(|content| Item {
							content: content.clone(),
							status:  Status::Pending,
							blocker: None,
						})
						.collect(),
				})
				.collect()
		},
		"start" => {
			if let Some((p, t)) = resolve_task(&phases, params.task.as_deref(), errors) {
				for phase in &mut phases {
					for task in &mut phase.tasks {
						if task.status == Status::InProgress {
							task.status = Status::Pending;
						}
					}
				}
				phases[p].tasks[t].status = Status::InProgress;
			}
			phases
		},
		op @ ("done" | "drop") => {
			let status = if op == "done" {
				Status::Completed
			} else {
				Status::Abandoned
			};
			for (p, t) in targets(&phases, params, errors) {
				phases[p].tasks[t].status = status;
			}
			phases
		},
		"block" => {
			if !has(params.task.as_ref()) && !has(params.phase.as_ref()) {
				errors.push("block requires a task or phase target".into());
				return phases;
			}
			let reason = params
				.reason
				.as_deref()
				.map(|reason| reason.split_whitespace().collect::<Vec<_>>().join(" "))
				.filter(|reason| !reason.is_empty());
			for (p, t) in targets(&phases, params, errors) {
				let task = &mut phases[p].tasks[t];
				if task.status.is_open() || task.status == Status::Blocked {
					task.status = Status::Blocked;
					task.blocker.clone_from(&reason);
				}
			}
			phases
		},
		"unblock" => {
			if !has(params.task.as_ref()) && !has(params.phase.as_ref()) {
				errors.push("unblock requires a task or phase target".into());
				return phases;
			}
			for (p, t) in targets(&phases, params, errors) {
				let task = &mut phases[p].tasks[t];
				if task.status == Status::Blocked {
					task.status = Status::Pending;
					task.blocker = None;
				}
			}
			phases
		},
		"rm" => {
			if has(params.task.as_ref()) {
				if let Some((p, t)) = resolve_task(&phases, params.task.as_deref(), errors) {
					phases[p].tasks.remove(t);
				}
			} else if let Some(name) = params.phase.as_deref().filter(|name| !name.is_empty()) {
				if let Some(p) = resolve_phase(&phases, name, errors) {
					phases[p].tasks.clear();
				}
			} else {
				for phase in &mut phases {
					phase.tasks.clear();
				}
			}
			phases
		},
		"append" => {
			let Some(name) = params.phase.as_deref().filter(|name| !name.is_empty()) else {
				errors.push("Missing phase name for append operation".into());
				return phases;
			};
			let Some(items) = params.items.as_ref().filter(|items| !items.is_empty()) else {
				errors.push("Missing items for append operation".into());
				return phases;
			};
			let mut seen: Vec<&String> = Vec::new();
			let mut duplicate = false;
			for content in items {
				if seen.contains(&content) || locate(&phases, content).is_some() {
					errors.push(format!("Task \"{content}\" already exists"));
					duplicate = true;
				}
				seen.push(content);
			}
			if duplicate {
				return phases;
			}
			let index = phases
				.iter()
				.position(|phase| phase.name == name)
				.unwrap_or_else(|| {
					phases.push(Phase { name: name.to_owned(), tasks: Vec::new() });
					phases.len() - 1
				});
			phases[index].tasks.extend(items.iter().map(|content| Item {
				content: content.clone(),
				status:  Status::Pending,
				blocker: None,
			}));
			phases
		},
		_ => phases,
	}
}

/// `normalizeInProgressTask`: at most one in-progress task; with none, the
/// first pending task starts.
fn normalize(phases: &mut [Phase]) {
	let mut seen_in_progress = false;
	for task in phases.iter_mut().flat_map(|phase| phase.tasks.iter_mut()) {
		if task.status == Status::InProgress {
			if seen_in_progress {
				task.status = Status::Pending;
			}
			seen_in_progress = true;
		}
	}
	if !seen_in_progress
		&& let Some(task) = phases
			.iter_mut()
			.flat_map(|phase| phase.tasks.iter_mut())
			.find(|task| task.status == Status::Pending)
	{
		task.status = Status::InProgress;
	}
}

/// `resolveTodoParams`: validate, inferring an omitted `op` from an
/// unambiguous payload.
fn resolve_params(raw: &Value, has_existing: bool) -> Result<Params, String> {
	let parse = |value: &Value| -> Result<Params, String> {
		let params: Params =
			serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
		if !OPS.contains(&params.op.as_str()) {
			return Err(format!("op must be one of {} (was \"{}\")", OPS.join(", "), params.op));
		}
		if params
			.list
			.iter()
			.flatten()
			.any(|entry| entry.items.is_empty())
		{
			return Err("list items must be non-empty".into());
		}
		Ok(params)
	};
	let direct = parse(raw);
	if direct.is_ok() {
		return direct;
	}
	if let Value::Object(map) = raw
		&& !map.contains_key("op")
	{
		let non_empty = |key: &str| {
			map.get(key)
				.and_then(Value::as_array)
				.is_some_and(|a| !a.is_empty())
		};
		let phase = map
			.get("phase")
			.and_then(Value::as_str)
			.is_some_and(|p| !p.is_empty());
		let inferred = if non_empty("list") {
			Some("init")
		} else if non_empty("items") && phase {
			Some("append")
		} else if non_empty("items") && !has_existing {
			Some("init")
		} else {
			None
		};
		if let Some(op) = inferred {
			let mut repaired = map.clone();
			repaired.insert("op".into(), Value::String(op.into()));
			if let Ok(params) = parse(&Value::Object(repaired)) {
				return Ok(params);
			}
		}
	}
	Err(format!("Invalid todo arguments: {}", direct.err().unwrap_or_default()))
}

/// `formatSummary`.
fn summary(phases: &[Phase], errors: &[String], read_only: bool) -> String {
	let tasks: Vec<&Item> = phases.iter().flat_map(|phase| &phase.tasks).collect();
	if tasks.is_empty() {
		if !errors.is_empty() {
			return format!("Errors: {}", errors.join("; "));
		}
		return if read_only {
			"Todo list is empty."
		} else {
			"Todo list cleared."
		}
		.into();
	}
	let remaining: Vec<(&Item, &str)> = phases
		.iter()
		.flat_map(|phase| {
			phase
				.tasks
				.iter()
				.filter(|task| task.status.is_open())
				.map(move |task| (task, phase.name.as_str()))
		})
		.collect();
	let current_index = phases
		.iter()
		.position(|phase| phase.tasks.iter().any(|task| task.status.is_open()))
		.unwrap_or(phases.len() - 1);
	let current = &phases[current_index];
	let done = current
		.tasks
		.iter()
		.filter(|task| task.status.is_closed())
		.count();
	let mut lines = Vec::new();
	if !errors.is_empty() {
		lines.push(format!("Errors: {}", errors.join("; ")));
	}
	if remaining.is_empty() {
		lines.push("Remaining items: none.".into());
	} else {
		lines.push(format!("Remaining items ({}):", remaining.len()));
		for (task, phase) in &remaining {
			lines.push(format!("  - {} [{}] ({phase})", task.content, task.status.name()));
		}
	}
	let closed = tasks.iter().filter(|task| task.status.is_closed()).count();
	let blocked = tasks
		.iter()
		.filter(|task| task.status == Status::Blocked)
		.count();
	let worked_ahead = phases
		.iter()
		.skip(current_index + 1)
		.any(|phase| phase.tasks.iter().any(|task| task.status.is_closed()));
	let blocked_text = if blocked > 0 {
		format!(", {blocked} blocked")
	} else {
		String::new()
	};
	lines.push(format!(
		"Overall: {closed}/{} done, {} open{blocked_text}.",
		tasks.len(),
		remaining.len()
	));
	lines.push(format!(
		"Active phase {}/{} \"{}\" ({done}/{}){}",
		current_index + 1,
		phases.len(),
		current.name,
		current.tasks.len(),
		if worked_ahead {
			" — earliest phase with open tasks; the in-progress pointer auto-advances to the earliest \
			 open task on each completion, so it can sit behind out-of-order work (nothing was \
			 un-completed)."
		} else {
			"."
		}
	));
	for phase in phases {
		lines.push(format!("  {}:", phase.name));
		for task in &phase.tasks {
			let checkbox = if task.status == Status::Completed {
				"[X]"
			} else {
				"[ ]"
			};
			let tag = match (task.status, &task.blocker) {
				(Status::InProgress, _) => " (in progress)".to_owned(),
				(Status::Abandoned, _) => " (dropped)".to_owned(),
				(Status::Blocked, Some(blocker)) => format!(" (blocked: {blocker})"),
				(Status::Blocked, None) => " (blocked)".to_owned(),
				_ => String::new(),
			};
			lines.push(format!("    - {checkbox} {}{tag}", task.content));
		}
	}
	lines.join("\n")
}

/// Run one todo call; errors are reported in the result text (the legacy tool
/// returns them as error results, not throws) and leave the state unchanged.
pub(crate) fn todo(state: &TodoList, arguments: &str) -> String {
	let previous = state.phases();
	let raw: Value = match serde_json::from_str(arguments) {
		Ok(raw) => raw,
		Err(error) => return format!("Invalid todo arguments: {error}"),
	};
	let params = match resolve_params(&raw, !previous.is_empty()) {
		Ok(params) => params,
		Err(message) => return message,
	};
	if params.op == "view" {
		return summary(&previous, &[], true);
	}
	let mut errors = Vec::new();
	let mut updated = apply(previous.clone(), &params, &mut errors);
	normalize(&mut updated);
	if errors.is_empty() {
		let text = summary(&updated, &errors, false);
		state.set(updated);
		text
	} else {
		summary(&previous, &errors, false)
	}
}

/// `phasesToMarkdown`.
pub fn to_markdown(phases: &[Phase]) -> String {
	if phases.is_empty() {
		return "# Todos\n".into();
	}
	let mut out = Vec::new();
	for (index, phase) in phases.iter().enumerate() {
		if index > 0 {
			out.push(String::new());
		}
		out.push(format!("# {}", phase.name));
		for task in &phase.tasks {
			let marker = match task.status {
				Status::Pending => ' ',
				Status::InProgress => '/',
				Status::Completed => 'x',
				Status::Abandoned => '-',
				Status::Blocked => '!',
			};
			let note = match (&task.status, &task.blocker) {
				(Status::Blocked, Some(blocker)) if !blocker.is_empty() => {
					format!(" <!-- blocker: {blocker} -->")
				},
				_ => String::new(),
			};
			out.push(format!("- [{marker}] {}{note}", task.content));
		}
	}
	format!("{}\n", out.join("\n"))
}

/// `markdownToPhases`.
pub fn from_markdown(markdown: &str) -> (Vec<Phase>, Vec<String>) {
	let (mut phases, mut errors): (Vec<Phase>, Vec<String>) = (Vec::new(), Vec::new());
	for (index, raw) in markdown.lines().enumerate() {
		let line = raw.trim();
		if line.is_empty() {
			continue;
		}
		let hashes = line.bytes().take_while(|&b| b == b'#').count();
		if (1..=6).contains(&hashes)
			&& line[hashes..].starts_with(char::is_whitespace)
			&& !line[hashes..].trim().is_empty()
		{
			phases.push(Phase { name: line[hashes..].trim().to_owned(), tasks: Vec::new() });
			continue;
		}
		if let Some((marker, content)) = parse_task_line(line) {
			let status = match marker {
				" " | "" => Status::Pending,
				"x" | "X" => Status::Completed,
				"/" | ">" => Status::InProgress,
				"-" | "~" => Status::Abandoned,
				"!" => Status::Blocked,
				_ => {
					errors.push(format!(
						"Line {}: unknown status marker \"[{marker}]\" (use [ ], [x], [/], [-], [!])",
						index + 1
					));
					continue;
				},
			};
			if phases.is_empty() {
				phases.push(Phase { name: "Todos".into(), tasks: Vec::new() });
			}
			let (content, blocker) = match content
				.strip_suffix("-->")
				.and_then(|head| head.rsplit_once("<!--"))
			{
				Some((text, note)) if status == Status::Blocked => {
					match note.trim().strip_prefix("blocker:") {
						Some(note) => (text.trim().to_owned(), Some(note.trim().to_owned())),
						None => (content.to_owned(), None),
					}
				},
				_ => (content.to_owned(), None),
			};
			phases
				.last_mut()
				.expect("phase")
				.tasks
				.push(Item { content, status, blocker });
			continue;
		}
		errors.push(format!("Line {}: unrecognized syntax \"{line}\"", index + 1));
	}
	normalize(&mut phases);
	(phases, errors)
}

/// `^[-*+]\s*\\?\[(.?)\\?\]\s+(.+?)\s*$`.
fn parse_task_line(line: &str) -> Option<(&str, &str)> {
	let rest = line.strip_prefix(['-', '*', '+'])?.trim_start();
	let rest = rest.strip_prefix('\\').unwrap_or(rest).strip_prefix('[')?;
	let close = rest.find(']')?;
	let mut marker = &rest[..close];
	marker = marker.strip_suffix('\\').unwrap_or(marker);
	if marker.chars().count() > 1 {
		return None;
	}
	let after = &rest[close + 1..];
	if !after.starts_with(char::is_whitespace) {
		return None;
	}
	let content = after.trim();
	(!content.is_empty()).then_some((marker, content))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn init_normalizes_and_done_advances_the_pointer() {
		let state = TodoList::new();
		let text = todo(
			&state,
			r#"{"op":"init","list":[{"phase":"A","items":["one","two"]},{"phase":"B","items":["three"]}]}"#,
		);
		assert!(
			text.starts_with("Remaining items (3):\n  - one [in_progress] (A)\n  - two [pending] (A)"),
			"{text}"
		);
		let text = todo(&state, r#"{"op":"done","task":"one"}"#);
		assert!(text.contains("  - two [in_progress] (A)"), "{text}");
		assert!(text.contains("Overall: 1/3 done, 2 open."), "{text}");
		assert!(text.contains("    - [X] one"), "{text}");
	}

	#[test]
	fn failed_batches_leave_state_untouched_and_report_errors() {
		let state = TodoList::new();
		todo(&state, r#"{"items":["a"]}"#);
		let before = state.phases();
		let text = todo(&state, r#"{"op":"append","phase":"Tasks","items":["a","b"]}"#);
		assert!(text.starts_with("Errors: Task \"a\" already exists"), "{text}");
		assert_eq!(state.phases(), before);
		assert!(todo(&state, r#"{"op":"done","task":"task-3"}"#).contains("referenced by content"));
		assert!(todo(&state, r#"{"task":"a"}"#).starts_with("Invalid todo arguments:"));
	}

	#[test]
	fn block_notes_round_trip_through_markdown() {
		let state = TodoList::new();
		todo(&state, r#"{"op":"init","items":["a","b"]}"#);
		todo(&state, r#"{"op":"block","task":"b","reason":"waiting\n on  CI"}"#);
		let markdown = to_markdown(&state.phases());
		assert_eq!(markdown, "# Tasks\n- [/] a\n- [!] b <!-- blocker: waiting on CI -->\n");
		let (phases, errors) = from_markdown(&markdown);
		assert!(errors.is_empty());
		assert_eq!(phases, state.phases());
		assert_eq!(from_markdown("- [?] x").1, [
			"Line 1: unknown status marker \"[?]\" (use [ ], [x], [/], [-], [!])"
		]);
	}
}
