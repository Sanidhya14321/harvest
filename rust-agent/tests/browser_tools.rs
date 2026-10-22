use std::{
	path::Path,
	sync::{
		Arc, Mutex,
		atomic::{AtomicU32, Ordering},
	},
};

use anyhow::Result;
use harvest_omp::{
	agent::RunOptions,
	agent_session::AgentSession,
	chat::{ChatMessage, FunctionCall, ToolCall, ToolSpec},
	provider::Model,
	tools::{Approval, DenyMutations, ToolRuntime},
};

fn browser_call() -> ToolCall {
	ToolCall {
		id:       "browser-1".into(),
		kind:     "function".into(),
		function: FunctionCall {
			name:      "browser".into(),
			arguments: r#"{"action":"open","url":"https://example.test/approved?final=1"}"#.into(),
		},
	}
}

#[test]
fn browser_is_opt_in_and_approval_precedes_profile_or_process_creation() -> Result<()> {
	let root = tempfile::tempdir()?;
	let profile = root.path().join("profile");
	let mut tools = ToolRuntime::default();
	assert!(!tools.specs().iter().any(|spec| spec.name() == "browser"));
	assert!(
		tools
			.invoke(root.path(), &browser_call(), &DenyMutations, &Default::default())
			.is_err()
	);
	tools.configure_browser(harvest_omp::browser::Options {
		chrome:     root.path().join("missing-chrome"),
		profile:    profile.clone(),
		headless:   true,
		no_sandbox: false,
	});
	assert!(tools.specs().iter().any(|spec| spec.name() == "browser"));
	let error = tools
		.invoke(root.path(), &browser_call(), &DenyMutations, &Default::default())
		.unwrap_err();
	assert!(error.to_string().contains("requires human approval"));
	assert!(!profile.exists());
	Ok(())
}

struct Reject {
	seen: Arc<Mutex<Vec<ToolCall>>>,
}
impl Approval for Reject {
	fn approve(&self, call: &ToolCall) -> Result<bool> {
		self.seen.lock().unwrap().push(call.clone());
		Ok(false)
	}
}
struct Script {
	count: AtomicU32,
}
impl Model for Script {
	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
		assert!(tools.iter().any(|spec| spec.name() == "browser"));
		let count = self.count.fetch_add(1, Ordering::SeqCst);
		let calls = if count == 0 {
			Some(vec![browser_call()])
		} else {
			assert_eq!(messages.last().unwrap().tool_call_id.as_deref(), Some("browser-1"));
			assert!(
				messages
					.last()
					.unwrap()
					.content
					.as_ref()
					.unwrap()
					.contains("requires human approval")
			);
			None
		};
		Ok(ChatMessage {
			role:         "assistant".into(),
			content:      Some("checked".into()),
			tool_calls:   calls,
			tool_call_id: None,
		})
	}
}

#[test]
fn session_uses_host_approval_with_final_browser_arguments_and_persists_the_denial() -> Result<()> {
	let root = tempfile::tempdir()?;
	let profile = root.path().join("profile");
	let seen = Arc::new(Mutex::new(Vec::new()));
	let mut session = AgentSession::open(root.path(), Path::new("session.jsonl"), None)?;
	session.configure_browser(harvest_omp::browser::Options {
		chrome:     root.path().join("missing-chrome"),
		profile:    profile.clone(),
		headless:   true,
		no_sandbox: false,
	});
	session.submit(
		"use the browser",
		&Script { count: AtomicU32::new(0) },
		&Reject { seen: Arc::clone(&seen) },
		&RunOptions {
			max_steps:      2,
			user_agent_dir: None,
			tags:           Default::default(),
			cancellation:   Default::default(),
		},
		&mut |_| Ok(()),
	)?;
	assert_eq!(*seen.lock().unwrap(), vec![browser_call()]);
	assert!(!profile.exists());
	drop(session);
	let log = harvest_omp::durable_session::DurableSession::open(
		root.path(),
		Path::new("session.jsonl"),
		None,
	)?;
	assert!(log.messages().iter().any(
		|message| message.role == "tool" && message.tool_call_id.as_deref() == Some("browser-1")
	));
	Ok(())
}

#[tokio::test]
async fn async_session_uses_the_same_browser_approval_and_drains_resources_on_shutdown()
-> Result<()> {
	use harvest_omp::session_engine::{SessionEngine, SessionEngineOptions};
	let root = tempfile::tempdir()?;
	let profile = root.path().join("profile");
	let seen = Arc::new(Mutex::new(Vec::new()));
	let session = SessionEngine::open(
		root.path(),
		Path::new("async.jsonl"),
		None,
		Arc::new(Script { count: AtomicU32::new(0) }),
		Arc::new(Reject { seen: Arc::clone(&seen) }),
		SessionEngineOptions {
			browser: Some(harvest_omp::browser::Options {
				chrome:     root.path().join("missing-chrome"),
				profile:    profile.clone(),
				headless:   true,
				no_sandbox: false,
			}),
			..Default::default()
		},
	)
	.await?;
	assert_eq!(session.submit("use the browser").await?.wait().await?, "checked");
	session.shutdown().await?;
	assert_eq!(*seen.lock().unwrap(), vec![browser_call()]);
	assert!(!profile.exists());
	let log = harvest_omp::durable_session::DurableSession::open(
		root.path(),
		Path::new("async.jsonl"),
		None,
	)?;
	assert!(log.messages().iter().any(
		|message| message.role == "tool" && message.tool_call_id.as_deref() == Some("browser-1")
	));
	Ok(())
}
