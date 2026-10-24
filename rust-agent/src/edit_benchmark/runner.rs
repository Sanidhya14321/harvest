//! Native benchmark orchestration. Scoring owns fixture semantics; this owner
//! provides independent workspaces, bounded workers and durable run artifacts.
use std::{
	cell::RefCell,
	path::{Path, PathBuf},
	sync::{
		Mutex,
		atomic::{AtomicUsize, Ordering},
	},
	time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use harvest_runtime::{
	JailedFs, MAX_FILE_BYTES,
	cancel::{AbortError, AbortReason, CancelToken},
};
use serde::Serialize;
use serde_json::value::RawValue;

use super::{SCORING_VERSION, Task, compare, list_files, load_tasks};
use crate::{
	agent::RunOptions,
	agent_session::AgentSession,
	chat::{ChatMessage, ToolSpec},
	provider::{Model, ModelResponse},
	tools::{Approval, DenyMutations},
	usage::TokenUsage,
};

#[derive(Debug, Clone)]
pub struct RunConfiguration {
	pub edit_mode:           crate::edit::Mode,
	/// Explicit oracle-assisted guidance, separately versioned in the run plan.
	pub guided:              bool,
	/// Check exact output after all results in a tool batch are durable.
	pub early_stop_on_match: bool,
	/// Scored attempts per run, excluding bounded no-edit retries.
	pub max_attempts:        u32,
	pub no_edit_retries:     u32,
	pub provider_retries:    u32,
	pub repetitions:         u32,
	pub concurrency:         usize,
	pub max_steps:           u32,
	pub timeout:             Duration,
	/// Optional cooperative deadline for each submit; never extends run timeout.
	pub attempt_timeout:     Option<Duration>,
	pub timeout_retries:     u32,
	/// An empty list selects all tasks, otherwise require exact existing IDs.
	pub task_ids:            Vec<String>,
	pub require_edit_tool:   bool,
	pub require_read_tool:   bool,
	pub export_messages:     bool,
}
impl Default for RunConfiguration {
	fn default() -> Self {
		Self {
			edit_mode:           Default::default(),
			guided:              false,
			early_stop_on_match: true,
			max_attempts:        1,
			no_edit_retries:     2,
			provider_retries:    3,
			repetitions:         1,
			concurrency:         1,
			max_steps:           16,
			timeout:             Duration::from_secs(300),
			attempt_timeout:     None,
			timeout_retries:     3,
			task_ids:            Vec::new(),
			require_edit_tool:   false,
			require_read_tool:   false,
			export_messages:     false,
		}
	}
}
#[derive(Debug, Serialize)]
pub struct RunOutcome {
	pub early_stopped:          bool,
	pub attempts:               u32,
	pub provider_retries:       u32,
	pub timeout_retries:        u32,
	pub no_edit_retries:        u32,
	pub tool_evidence:          Option<super::ToolEvidence>,
	pub requirements_satisfied: Option<bool>,
	pub index:                  usize,
	pub task_id:                String,
	pub repetition:             u32,
	pub state:                  &'static str,
	pub success:                bool,
	pub elapsed_micros:         u128,
	/// Relative to the explicitly selected output directory.
	pub artifacts:              Option<PathBuf>,
	pub error:                  Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct RunSummary {
	pub scoring_version: &'static str,
	pub runs:            Vec<RunOutcome>,
	pub successful:      usize,
	pub unsuccessful:    usize,
}
#[derive(Default, Serialize)]
struct Measurements {
	calls:        usize,
	failed_calls: usize,
	/// None if any response lacks valid accounting, or checked sums overflow.
	tokens:       Option<TokenUsage>,
	raw:          Vec<Option<Box<RawValue>>>,
	#[serde(skip)]
	bytes:        usize,
}
impl Measurements {
	fn record(&mut self, response: &ModelResponse) -> Result<()> {
		self.calls += 1;
		let raw = response.usage.as_ref().map(|u| u.raw.clone());
		self.bytes = self
			.bytes
			.checked_add(raw.as_ref().map_or(0, |r| r.get().len()))
			.ok_or_else(|| anyhow::anyhow!("benchmark usage size overflow"))?;
		ensure!(self.calls <= 1024 && self.bytes <= 1024 * 1024, "benchmark usage exceeds limit");
		let incoming = response.usage.as_ref().and_then(|u| u.tokens.clone());
		self.tokens = if self.calls == 1 {
			incoming
		} else {
			self.tokens.take().zip(incoming).and_then(|(a, b)| {
				Some(TokenUsage {
					input:            a.input.checked_add(b.input)?,
					output:           a.output.checked_add(b.output)?,
					cache_read:       a.cache_read.checked_add(b.cache_read)?,
					cache_write:      a.cache_write.checked_add(b.cache_write)?,
					total_tokens:     a.total_tokens.checked_add(b.total_tokens)?,
					reasoning_tokens: match (a.reasoning_tokens, b.reasoning_tokens) {
						(Some(a), Some(b)) => Some(a.checked_add(b)?),
						_ => None,
					},
				})
			})
		};
		self.raw.push(raw);
		Ok(())
	}
}
struct MeasuredModel<M> {
	inner:        M,
	measurements: RefCell<Measurements>,
}
impl<M: Model> Model for MeasuredModel<M> {
	fn model_id(&self) -> &str {
		self.inner.model_id()
	}

	fn invalidate_command_credentials(&self, cancellation: &CancelToken) -> Result<()> {
		self.inner.invalidate_command_credentials(cancellation)
	}

	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
		let tools: Vec<_> = tools
			.iter()
			.filter(|t| matches!(t.name(), "read" | "write" | "edit" | "grep"))
			.cloned()
			.collect();
		let response = match self.inner.complete_response(messages, &tools) {
			Ok(response) => response,
			Err(error) => {
				self.measurements.borrow_mut().failed_calls += 1;
				return Err(error);
			},
		};
		self.measurements.borrow_mut().record(&response)?;
		Ok(response.message)
	}

	fn stream_response(
		&self,
		messages: &[ChatMessage],
		tools: &[ToolSpec],
		cancellation: &CancelToken,
		on_text: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ModelResponse> {
		let tools: Vec<_> = tools
			.iter()
			.filter(|t| matches!(t.name(), "read" | "write" | "edit" | "grep"))
			.cloned()
			.collect();
		let response = match self
			.inner
			.stream_response(messages, &tools, cancellation, on_text)
		{
			Ok(response) => response,
			Err(error) => {
				self.measurements.borrow_mut().failed_calls += 1;
				return Err(error);
			},
		};
		self.measurements.borrow_mut().record(&response)?;
		Ok(response)
	}
}
struct ScopedApproval<'a>(&'a (dyn Approval + Sync));
impl Approval for ScopedApproval<'_> {
	fn approve(&self, call: &crate::chat::ToolCall) -> Result<bool> {
		// A model may return an unadvertised call; never authorize a shell,
		// browser, memory mutation, service or plugin through this benchmark.
		if matches!(call.function.name.as_str(), "write" | "edit") {
			self.0.approve(call)
		} else {
			DenyMutations.approve(call)
		}
	}
}
fn persist(fs: &JailedFs, name: &Path, value: &impl Serialize) -> Result<()> {
	let bytes = serde_json::to_vec(value)?;
	ensure!(bytes.len() <= 16 * 1024 * 1024, "benchmark report exceeds limit");
	fs.atomic_create(name, &bytes)
}

/// Each factory invocation creates an independent model in its worker thread.
/// No configured plugins, MCP, browser, user brain or language servers are
/// loaded. `approval` sees the actual final write/edit arguments; tools retain
/// the jail. Cooperative cancellation cannot forcibly interrupt a custom SDK
/// Model that ignores its cancellation token. The native HTTP model uses
/// cancellable I/O.
pub fn run_tasks<M: Model>(
	fixtures: &Path,
	output: &Path,
	configuration: &RunConfiguration,
	cancellation: &CancelToken,
	model: &(impl Fn(&Task, u32) -> Result<M> + Sync),
	approval: &(dyn Approval + Sync),
) -> Result<RunSummary> {
	ensure!(
		!configuration.guided || configuration.edit_mode == crate::edit::Mode::Hashline,
		"native guidance requires hashline edit mode"
	);
	ensure!((1..=16).contains(&configuration.max_attempts), "max attempts must be 1..16");
	ensure!(configuration.no_edit_retries <= 16, "no-edit retries must be 0..16");
	ensure!(configuration.provider_retries <= 16, "provider retries must be 0..16");
	ensure!(configuration.timeout_retries <= 16, "timeout retries must be 0..16");
	ensure!(
		configuration
			.attempt_timeout
			.is_none_or(|timeout| !timeout.is_zero() && timeout <= configuration.timeout),
		"attempt timeout must be positive and at most the run timeout"
	);
	ensure!((1..=100).contains(&configuration.repetitions), "repetitions must be 1..100");
	ensure!((1..=16).contains(&configuration.concurrency), "concurrency must be 1..16");
	ensure!((1..=1024).contains(&configuration.max_steps), "max steps must be 1..1024");
	ensure!(
		!configuration.timeout.is_zero() && configuration.timeout <= Duration::from_secs(3600),
		"run timeout must be positive and at most one hour"
	);
	cancellation.heartbeat()?;
	let fixture_source = super::FixtureSource::open(fixtures, cancellation)?;
	let fixtures = fixture_source.directory();
	let mut tasks = load_tasks(fixtures)?;
	let selected: std::collections::BTreeSet<_> = configuration.task_ids.iter().collect();
	ensure!(selected.len() == configuration.task_ids.len(), "duplicate task selection");
	ensure!(
		selected
			.iter()
			.all(|id| tasks.iter().any(|task| &task.id == *id)),
		"unknown task selection"
	);
	if !selected.is_empty() {
		tasks.retain(|t| selected.contains(&t.id));
	}
	let count = tasks
		.len()
		.checked_mul(configuration.repetitions as usize)
		.ok_or_else(|| anyhow::anyhow!("benchmark run count overflow"))?;
	ensure!(count > 0 && count <= 1000, "benchmark must contain 1..1000 runs");
	let fixtures = fixtures.canonicalize()?;
	let output = JailedFs::open_or_create(output)?;
	ensure!(
		!output.root().starts_with(&fixtures) && !fixtures.starts_with(output.root()),
		"output and fixture trees must be disjoint"
	);
	let _owner = output.resource_lock(Path::new(".benchmark-owner"))?;
	ensure!(
		output
			.entries(Path::new(""), 4)?
			.iter()
			.all(|e| e.name == ".benchmark-owner.lock"),
		"benchmark output must be empty; use a new directory for each run"
	);
	persist(
		&output,
		Path::new("run-plan.json"),
		&serde_json::json!({
			 "edit_mode":configuration.edit_mode,"guided":configuration.guided,
				 "guidance_version":if configuration.guided {Some("rust-hashline-guidance-v1")} else {None},
				 "scoring_version":SCORING_VERSION,"runs":count,"repetitions":configuration.repetitions,
			 "concurrency":configuration.concurrency,"max_steps":configuration.max_steps,
			 "early_stop_on_match":configuration.early_stop_on_match,"max_attempts":configuration.max_attempts,"no_edit_retries":configuration.no_edit_retries,"provider_retries":configuration.provider_retries,
			 "timeout_ms":configuration.timeout.as_millis(),
			"attempt_timeout_ms":configuration.attempt_timeout.map(|v| v.as_millis()),"timeout_retries":configuration.timeout_retries,
			"require_edit_tool":configuration.require_edit_tool,"require_read_tool":configuration.require_read_tool,
			"export_messages":configuration.export_messages,"tasks":tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
		}),
	)?;
	let next = AtomicUsize::new(0);
	let outcomes = Mutex::new(Vec::with_capacity(count));
	std::thread::scope(|scope| -> Result<()> {
		let mut workers = Vec::new();
		for _ in 0..configuration.concurrency.min(count) {
			workers.push(scope.spawn(|| -> Result<()> {
				loop {
					let index = next.fetch_add(1, Ordering::Relaxed);
					if index >= count {
						break;
					}
					let task = &tasks[index / configuration.repetitions as usize];
					let repetition = (index % configuration.repetitions as usize) as u32 + 1;
					let result = run_one(
						task,
						repetition,
						index,
						&output,
						configuration,
						cancellation,
						model,
						approval,
					)?;
					outcomes
						.lock()
						.map_err(|_| anyhow::anyhow!("benchmark result owner poisoned"))?
						.push(result);
				}
				Ok(())
			}));
		}
		for worker in workers {
			worker
				.join()
				.map_err(|_| anyhow::anyhow!("benchmark worker panicked"))??;
		}
		Ok(())
	})?;
	let mut runs = outcomes
		.into_inner()
		.map_err(|_| anyhow::anyhow!("benchmark result owner poisoned"))?;
	runs.sort_by_key(|r| r.index);
	let successful = runs.iter().filter(|r| r.success).count();
	let summary = RunSummary {
		scoring_version: SCORING_VERSION,
		unsuccessful: runs.len() - successful,
		successful,
		runs,
	};
	persist(&output, Path::new("summary.json"), &summary)?;
	Ok(summary)
}
fn run_one<M: Model>(
	task: &Task,
	repetition: u32,
	index: usize,
	output: &JailedFs,
	configuration: &RunConfiguration,
	cancellation: &CancelToken,
	factory: &(impl Fn(&Task, u32) -> Result<M> + Sync),
	approval: &(dyn Approval + Sync),
) -> Result<RunOutcome> {
	let started = Instant::now();
	let mut outcome = RunOutcome {
		provider_retries: 0,
		timeout_retries: 0,
		early_stopped: false,
		attempts: 0,
		no_edit_retries: 0,
		tool_evidence: None,
		requirements_satisfied: None,
		index,
		task_id: task.id.clone(),
		repetition,
		state: "not_started",
		success: false,
		elapsed_micros: 0,
		artifacts: None,
		error: None,
	};
	if cancellation.heartbeat().is_err() {
		outcome.state = "cancelled";
		outcome.error = Some("cancelled_before_start");
		return Ok(outcome);
	}
	let directory = PathBuf::from(format!("run-{index:05}"));
	let artifacts = JailedFs::open_or_create(&output.root().join(&directory))?;
	let workspace = JailedFs::open_or_create(&artifacts.root().join("workspace"))?;
	let token = cancellation.child_with_timeout(configuration.timeout);
	outcome.artifacts = Some(directory);
	let mut attempt_timeout_failure = false;
	let execution = (|| -> Result<()> {
		let input = JailedFs::open(&task.input_directory)?;
		let files = list_files(&input, Path::new(""))?;
		for path in files {
			token.heartbeat()?;
			workspace.atomic_create(&path, &input.read(&path, MAX_FILE_BYTES)?)?;
		}
		// Expected fixtures stay outside the agent's tool root. Session and
		// event artifacts also stay outside it, so exact file-set scoring has
		// no hidden exceptions for generated application files.
		let mut session = AgentSession::open_with_storage(
			workspace.root(),
			artifacts.root(),
			Path::new("session.jsonl"),
			None,
		)?;
		session.configure_edit(configuration.edit_mode)?;
		let model = MeasuredModel {
			inner:        factory(task, repetition)?,
			measurements: RefCell::default(),
		};
		let mut prompt = task.prompt.clone();
		let mut scored_attempts = 0;
		let attempts_result = (|| -> Result<()> {
			loop {
				token.heartbeat()?;
				let first_message = session.stored_messages().len();
				outcome.attempts += 1;
				let attempt_started = Instant::now();
				let attempt_token = configuration
					.attempt_timeout
					.map_or_else(|| token.clone(), |timeout| token.child_with_timeout(timeout));
				let mut event_count = 0_u64;
				let mut text_deltas = 0_u64;
				let mut last_event = None;
				// Recompute from current bytes on every retry. Guidance is data,
				// never a read/snapshot or a bypass of seen-line enforcement.
				let delivered_prompt = if configuration.guided {
					super::guidance::with_guidance(
						&prompt,
						task,
						&workspace,
						&artifacts,
						outcome.attempts,
						&attempt_token,
					)?
				} else {
					prompt.clone()
				};
				let answer = session.submit_until_tool_checkpoint(
					&delivered_prompt,
					&model,
					&ScopedApproval(approval),
					&RunOptions {
						max_steps:      configuration.max_steps,
						user_agent_dir: None,
						tags:           Default::default(),
						cancellation:   attempt_token.clone(),
					},
					&mut |event| {
						let mut bytes = serde_json::to_vec(&event)?;
						bytes.push(b'\n');
						artifacts.append(Path::new("events.jsonl"), &bytes, 16 * 1024 * 1024)?;
						event_count += 1;
						last_event = Some(match event.event {
							crate::agent_session::EventKind::Started => "started",
							crate::agent_session::EventKind::TextDelta { .. } => {
								text_deltas += 1;
								"text_delta"
							},
							crate::agent_session::EventKind::PluginEvent { .. } => "plugin_event",
							crate::agent_session::EventKind::Committed { .. } => "committed",
							crate::agent_session::EventKind::Completed => "completed",
							crate::agent_session::EventKind::Failed => "failed",
						});
						Ok(())
					},
					&mut |messages| {
						if !configuration.early_stop_on_match {
							return Ok(false);
						}
						let evidence = super::tool_evidence(messages)?;
						let requirements = (!configuration.require_read_tool
							|| evidence.successful_reads > 0)
							&& (!configuration.require_edit_tool || evidence.successful_edits > 0);
						if !requirements {
							return Ok(false);
						}
						let matched =
							compare(&task.expected_directory, workspace.root(), &attempt_token)?.success;
						outcome.early_stopped |= matched;
						Ok(matched)
					},
				);
				let timed_out = answer.as_ref().err().is_some_and(|error| {
					error.chain().any(|cause| {
						cause
							.downcast_ref::<AbortError>()
							.is_some_and(|error| matches!(error.reason, AbortReason::Timeout))
					})
				});
				let elapsed_micros = attempt_started.elapsed().as_micros();
				// Keep cumulative accounting even when a later turn or export fails.
				let measurements = model.measurements.borrow();
				persist(
					&artifacts,
					Path::new(&format!("usage-{:02}.json", outcome.attempts)),
					&serde_json::json!({
						"complete":answer.is_ok(),
						"usage_unavailable":answer.is_err() || measurements.calls == 0 || measurements.failed_calls > 0 || measurements.tokens.is_none(),
						"measurement": &*measurements
					}),
				)?;
				drop(measurements);
				let evidence = super::tool_evidence(session.stored_messages())?;
				let attempt_evidence =
					super::tool_evidence(&session.stored_messages()[first_message..])?;
				if configuration.export_messages {
					persist(
						&artifacts,
						Path::new(&format!("messages-{:02}.json", outcome.attempts)),
						&session.stored_messages(),
					)?;
				}
				let requirements = (!configuration.require_read_tool || evidence.successful_reads > 0)
					&& (!configuration.require_edit_tool || evidence.successful_edits > 0);
				outcome.requirements_satisfied = Some(requirements);
				let patch_applied = evidence.patch_applied();
				outcome.tool_evidence = Some(evidence);
				persist(
					&artifacts,
					Path::new(&format!("attempt-{:02}.json", outcome.attempts)),
					&serde_json::json!({"attempt":outcome.attempts,"complete":answer.is_ok(),
					"tool_evidence":attempt_evidence,"requirements_satisfied":requirements,
						"elapsed_micros":elapsed_micros,"event_count":event_count,"text_deltas":text_deltas,
						"last_event":last_event,"committed_messages":session.stored_messages().len() - first_message,
						"timed_out":timed_out}),
				)?;
				if let Err(error) = &answer {
					// Outer cancellation/deadline always wins, even when the child
					// also expired. Never reset the overall budget on recovery.
					token.heartbeat()?;
					if timed_out && configuration.attempt_timeout.is_some() {
						if attempt_evidence.mutation_calls == 0
							&& attempt_evidence.unresolved_calls == 0
							&& outcome.timeout_retries < configuration.timeout_retries
						{
							outcome.timeout_retries += 1;
							persist(
								&artifacts,
								Path::new(&format!("timeout-retry-{:02}.json", outcome.timeout_retries)),
								&serde_json::json!({"attempt":outcome.attempts,"retry":outcome.timeout_retries,
									"elapsed_micros":elapsed_micros,"event_count":event_count,"text_deltas":text_deltas,
									"last_event":last_event,"mutation_calls":attempt_evidence.mutation_calls}),
							)?;
							prompt = include_str!("../prompts/benchmark-timeout-retry.md").into();
							continue;
						}
						attempt_timeout_failure = true;
					}

					if !timed_out
						&& attempt_evidence.mutation_calls == 0
						&& attempt_evidence.unresolved_calls == 0
						&& outcome.provider_retries < configuration.provider_retries
						&& let Some(failure) = super::retry::Failure::classify(error)
					{
						outcome.provider_retries += 1;
						let delay = super::retry::delay(outcome.provider_retries);
						persist(
							&artifacts,
							Path::new(&format!("provider-retry-{:02}.json", outcome.provider_retries)),
							&serde_json::json!({"attempt":outcome.attempts,"retry":outcome.provider_retries,
								"failure":failure,"delay_ms":delay.as_millis()}),
						)?;
						if failure.kind == "auth" {
							model.invalidate_command_credentials(&token)?;
						}
						super::retry::wait(delay, &token)?;
						prompt = include_str!("../prompts/benchmark-provider-retry.md").into();
						continue;
					}
				}
				// Unknown failures, failures after mutation and exhausted budgets
				// remain failures, never output mismatches or guessed recoveries.
				answer?;
				token.heartbeat()?;
				let mut report = compare(&task.expected_directory, workspace.root(), &token)?;
				report.patch_applied = patch_applied;
				report.success &= requirements;
				outcome.success = report.success;
				persist(
					&artifacts,
					Path::new(&format!("score-{:02}.json", outcome.attempts)),
					&report,
				)?;
				if report.success {
					persist(&artifacts, Path::new("score.json"), &report)?;
					break;
				}
				// Inspect this turn, not the whole session: an old edit must not hide
				// a later read-only turn. No-edit retries have their own run-wide cap.
				let no_edit = attempt_evidence.edit_calls == 0 && attempt_evidence.write_calls == 0;
				if no_edit && outcome.no_edit_retries < configuration.no_edit_retries {
					outcome.no_edit_retries += 1;
					prompt = include_str!("../prompts/benchmark-no-edit-retry.md").into();
				} else {
					scored_attempts += 1;
					if scored_attempts >= configuration.max_attempts {
						persist(&artifacts, Path::new("score.json"), &report)?;
						break;
					}
					prompt = include_str!("../prompts/benchmark-verification-retry.md").into();
				}
			}

			Ok(())
		})();
		persist(
			&artifacts,
			Path::new("usage.json"),
			&serde_json::json!({
				"complete":attempts_result.is_ok(),"usage_unavailable":attempts_result.is_err() || model.measurements.borrow().calls == 0 || model.measurements.borrow().failed_calls > 0 || model.measurements.borrow().tokens.is_none(),
				"measurement": &*model.measurements.borrow()
			}),
		)?;
		if configuration.export_messages {
			persist(&artifacts, Path::new("messages.json"), &session.stored_messages())?;
		}

		attempts_result
	})();
	outcome.success &= execution.is_ok();
	outcome.state = if execution.is_ok() {
		"completed"
	} else if cancellation.heartbeat().is_err() {
		"cancelled"
	} else if attempt_timeout_failure || token.heartbeat().is_err() {
		"timed_out"
	} else {
		"failed"
	};
	outcome.error = execution.err().map(|_| {
		if attempt_timeout_failure {
			"attempt_timeout"
		} else {
			"run_did_not_complete"
		}
	});
	outcome.elapsed_micros = started.elapsed().as_micros();
	persist(&artifacts, Path::new("outcome.json"), &outcome)?;
	Ok(outcome)
}
