use std::{cell::RefCell, collections::VecDeque, path::Path};

use anyhow::{Result, bail};
use harvest_omp::{
	agent::RunOptions,
	agent_session::{AgentSession, EventKind},
	chat::{self, ChatMessage, FunctionCall, ToolCall, ToolSpec},
	durable_session::DurableSession,
	provider::Model,
	session_format::SessionDocument,
	tools::Approval,
};

struct Script {
	replies:   RefCell<VecDeque<ChatMessage>>,
	histories: RefCell<Vec<Vec<ChatMessage>>>,
}
impl Model for Script {
	fn complete(&self, messages: &[ChatMessage], _tools: &[ToolSpec]) -> Result<ChatMessage> {
		self.histories.borrow_mut().push(messages.to_vec());
		self
			.replies
			.borrow_mut()
			.pop_front()
			.ok_or_else(|| anyhow::anyhow!("script exhausted"))
	}
}
struct Approve;
impl Approval for Approve {
	fn approve(&self, _: &ToolCall) -> Result<bool> {
		Ok(true)
	}
}
fn answer(text: &str) -> ChatMessage {
	ChatMessage {
		role:         "assistant".into(),
		content:      Some(text.into()),
		tool_calls:   None,
		tool_call_id: None,
	}
}
fn script(replies: Vec<ChatMessage>) -> Script {
	Script { replies: RefCell::new(replies.into()), histories: RefCell::new(Vec::new()) }
}
fn options() -> RunOptions {
	RunOptions {
		max_steps:      3,
		user_agent_dir: None,
		tags:           Default::default(),
		cancellation:   Default::default(),
	}
}

#[test]
fn approved_tool_results_are_durable_before_events_and_replayed_after_reopen() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = Path::new("session.jsonl");
	let call = ToolCall {
		id:       "write-1".into(),
		kind:     "function".into(),
		function: FunctionCall {
			name:      "write".into(),
			arguments: r#"{"path":"note","contents":"persisted"}"#.into(),
		},
	};
	let model = script(vec![
		ChatMessage { tool_calls: Some(vec![call]), ..answer("writing") },
		answer("saved"),
	]);
	let mut session = AgentSession::open(root.path(), path, None)?;
	let mut sequences = Vec::new();
	let result = session.submit("save a note", &model, &Approve, &options(), &mut |event| {
		sequences.push(event.sequence);
		if let EventKind::Committed { entry_id, .. } = event.event {
			let document = SessionDocument::decode(&std::fs::read(root.path().join(path))?)?;
			assert!(
				document
					.entries()
					.iter()
					.any(|row| row["id"].get() == serde_json::to_string(&entry_id).unwrap())
			);
		}
		Ok(())
	})?;
	assert_eq!(result, "saved");
	assert_eq!(std::fs::read_to_string(root.path().join("note"))?, "persisted");
	assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
	drop(session);
	let before = std::fs::read(root.path().join(path))?;
	let mut resumed = AgentSession::open(root.path(), path, None)?;
	let next = script(vec![answer("remembered")]);
	resumed.submit("what did you do?", &next, &Approve, &options(), &mut |_| Ok(()))?;
	let histories = next.histories.borrow();
	assert!(
		histories[0].iter().any(
			|message| message.role == "tool" && message.tool_call_id.as_deref() == Some("write-1")
		)
	);
	assert!(
		histories[0]
			.iter()
			.any(|message| message.content.as_deref() == Some("save a note"))
	);
	assert!(std::fs::read(root.path().join(path))?.starts_with(&before));
	Ok(())
}

#[test]
fn branch_selection_preserves_other_branches_and_writer_ownership() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = Path::new("session.jsonl");
	let original = concat!(
		r#"{"type":"session","version":3,"id":"existing","opaque":1844674407370955161999}"#,
		"\n",
		r#"{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"selected"}}"#,
		"\n",
		r#"{"type":"message","id":"b","parentId":null,"message":{"role":"user","content":"excluded"}}"#,
		"\n",
	);
	std::fs::write(root.path().join(path), original)?;
	let mut session = AgentSession::open(root.path(), path, Some("a"))?;
	assert!(AgentSession::open(root.path(), path, None).is_err());
	let model = script(vec![answer("branched")]);
	session.submit("continue", &model, &Approve, &options(), &mut |_| Ok(()))?;
	assert!(
		!model.histories.borrow()[0]
			.iter()
			.any(|message| message.content.as_deref() == Some("excluded"))
	);
	let bytes = std::fs::read(root.path().join(path))?;
	assert!(bytes.starts_with(original.as_bytes()));
	let document = SessionDocument::decode(&bytes)?;
	assert_eq!(document.entries()[2]["parentId"].get(), "\"a\"");
	Ok(())
}

#[test]
fn outside_writes_and_unsupported_replay_are_rejected_without_overwriting() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = Path::new("session.jsonl");
	let mut session = DurableSession::open(root.path(), path, None)?;
	std::fs::write(root.path().join(path), b"outside writer\n")?;
	assert!(
		session
			.commit(&chat::user("must not append"), "test")
			.is_err()
	);
	assert_eq!(std::fs::read(root.path().join(path))?, b"outside writer\n");
	drop(session);
	let source = concat!(
		"{\"type\":\"session\",\"version\":3,\"id\":\"signed\"}\n",
		"{\"type\":\"message\",\"id\":\"a\",\"parentId\":null,\"message\":{\"role\":\"assistant\",\"\
		 content\":[{\"type\":\"thinking\",\"signature\":\"opaque\"}]}}\n"
	);
	std::fs::write(root.path().join(path), source)?;
	assert!(DurableSession::open(root.path(), path, None).is_err());
	assert_eq!(std::fs::read(root.path().join(path))?, source.as_bytes());
	Ok(())
}

#[test]
fn old_cancellation_handles_cannot_abort_or_execute_a_new_generation() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut session = AgentSession::open(root.path(), Path::new("session.jsonl"), None)?;
	let old = session.begin(Default::default())?;
	old.abort();
	let model = script(vec![answer("new run")]);
	assert!(
		session
			.execute(&old, "aborted", &model, &Approve, &options(), &mut |_| Ok(()))
			.is_err()
	);
	let next = session.begin(Default::default())?;
	old.abort();
	assert!(
		session
			.execute(&old, "stale", &model, &Approve, &options(), &mut |_| Ok(()))
			.is_err()
	);
	assert_eq!(
		session.execute(&next, "current", &model, &Approve, &options(), &mut |_| Ok(()))?,
		"new run"
	);
	Ok(())
}

#[test]
fn failed_streams_do_not_persist_provisional_assistant_output() -> Result<()> {
	struct Broken;
	impl Model for Broken {
		fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ChatMessage> {
			bail!("unused")
		}

		fn stream(
			&self,
			_: &[ChatMessage],
			_: &[ToolSpec],
			_: &harvest_runtime::cancel::CancelToken,
			sink: &mut dyn FnMut(&str) -> Result<()>,
		) -> Result<ChatMessage> {
			sink("partial")?;
			bail!("disconnected")
		}
	}
	let root = tempfile::tempdir()?;
	let path = Path::new("session.jsonl");
	let mut session = AgentSession::open(root.path(), path, None)?;
	assert!(
		session
			.submit("hello", &Broken, &Approve, &options(), &mut |_| Ok(()))
			.is_err()
	);
	let document = SessionDocument::decode(&std::fs::read(root.path().join(path))?)?;
	assert_eq!(document.entries().len(), 1);
	assert!(!document.entries()[0]["message"].get().contains("partial"));
	drop(session);
	assert_eq!(
		DurableSession::open(root.path(), path, None)?.messages()[0]
			.content
			.as_deref(),
		Some("hello")
	);
	Ok(())
}
