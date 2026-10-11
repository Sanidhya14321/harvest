use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::{
	chat::{self, ChatMessage},
	provider::Model,
	tools,
};

const SYSTEM: &str = include_str!("prompts/system.md");

pub fn run(workspace: &Path, prompt: &str, max_steps: u32, model: &dyn Model) -> Result<String> {
	run_with_options(workspace, prompt, model, &tools::DenyMutations, &RunOptions {
		max_steps,
		user_agent_dir: None,
		tags: Default::default(),
		cancellation: Default::default(),
	})
}

/// Host configuration shared by new and resumed Rust sessions.
pub struct RunOptions {
	pub max_steps:      u32,
	pub user_agent_dir: Option<std::path::PathBuf>,
	pub tags:           std::collections::BTreeSet<String>,
	pub cancellation:   harvest_runtime::cancel::CancelToken,
}

pub fn run_with_options(
	workspace: &Path,
	prompt: &str,
	model: &dyn Model,
	approval: &dyn tools::Approval,
	options: &RunOptions,
) -> Result<String> {
	let max_steps = options.max_steps;
	if prompt.is_empty() {
		bail!("prompt is empty");
	}
	if max_steps == 0 {
		bail!("max steps must be at least 1");
	}
	let root = workspace
		.canonicalize()
		.with_context(|| format!("failed to resolve {}", workspace.display()))?;
	let mut session =
		crate::agent_session::AgentSession::open(&root, Path::new(".omp/session.jsonl"), None)?;
	session.submit(prompt, model, approval, options, &mut |_| Ok(()))
}

pub(crate) fn context(workspace: &Path, prompt: &str, options: &RunOptions) -> Result<String> {
	let root = workspace;
	let brain = harvest_brain::BrainStore::project(root)?;
	if brain.has_pending_recovery()? {
		brain.recover()?;
	}
	let user_brain = options
		.user_agent_dir
		.as_deref()
		.map(harvest_brain::BrainStore::user)
		.transpose()?;
	if let Some(user) = &user_brain
		&& user.has_pending_recovery()?
	{
		user.recover()?;
	}
	let mut stores = vec![&brain];
	if let Some(user) = &user_brain {
		stores.push(user);
	}
	let mut index = harvest_brain::BrainIndex::default();
	index.refresh(&stores)?;
	let context = index.context_block(prompt, &harvest_brain::RetrievalContext {
		tags: options.tags.clone(),
		..Default::default()
	});
	Ok(context)
}

pub(crate) fn system_message() -> ChatMessage {
	chat::system(SYSTEM)
}

#[cfg(test)]
mod tests {
	use std::{cell::RefCell, collections::VecDeque};

	use anyhow::Result;

	use super::run;
	use crate::{
		chat::{ChatMessage, FunctionCall, ToolCall, specs},
		provider::Model,
	};

	struct Script {
		replies: RefCell<VecDeque<ChatMessage>>,
	}

	impl Model for Script {
		fn complete(
			&self,
			_messages: &[ChatMessage],
			_tools: &[crate::chat::ToolSpec],
		) -> Result<ChatMessage> {
			self
				.replies
				.borrow_mut()
				.pop_front()
				.ok_or_else(|| anyhow::anyhow!("the scripted model ran out of replies"))
		}
	}

	fn tool_call(path: &str) -> ChatMessage {
		ChatMessage {
			role:         "assistant".to_owned(),
			content:      None,
			tool_calls:   Some(vec![ToolCall {
				id:       "call-1".to_owned(),
				kind:     "function".to_owned(),
				function: FunctionCall {
					name:      "read".to_owned(),
					arguments: format!(r#"{{"path":"{path}"}}"#),
				},
			}]),
			tool_call_id: None,
		}
	}

	#[test]
	fn agent_reads_a_file_and_returns_the_final_answer() {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("note.txt"), "hello from disk").unwrap();
		let model = Script {
			replies: RefCell::new(VecDeque::from([tool_call("note.txt"), ChatMessage {
				role:         "assistant".to_owned(),
				content:      Some("the note says hello".to_owned()),
				tool_calls:   None,
				tool_call_id: None,
			}])),
		};
		let answer = run(root.path(), "read the note", 4, &model).unwrap();
		assert_eq!(answer, "the note says hello");
		let session = std::fs::read_to_string(root.path().join(".omp/session.jsonl")).unwrap();
		assert!(session.contains("hello from disk"));
		assert!(!specs().is_empty());
	}

	#[test]
	fn agent_stops_when_the_model_keeps_calling_tools() {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("note.txt"), "x").unwrap();
		let model = Script {
			replies: RefCell::new(VecDeque::from([tool_call("note.txt"), tool_call("note.txt")])),
		};
		let error = run(root.path(), "loop", 1, &model).unwrap_err();
		assert!(error.to_string().contains("stopped after 1"));
	}
}
