use std::{
	collections::VecDeque,
	path::Path,
	sync::{Arc, Mutex, OnceLock},
	time::Duration,
};

use anyhow::{Result, bail};
use harvest_omp::{
	chat::{ChatMessage, FunctionCall, ToolCall, ToolSpec},
	config::Settings,
	durable_session::DurableSession,
	provider::{HttpModel, Model},
	session_engine::{
		RunHandle, SessionEngine, SessionEngineEventKind, SessionEngineOptions, SessionError,
		SessionHandle, SessionState,
	},
	session_format::SessionDocument,
	tools::Approval,
};
use harvest_runtime::cancel::CancelToken;
use tokio::sync::{Notify, Semaphore, broadcast};

fn answer(text: &str) -> ChatMessage {
	ChatMessage {
		role:         "assistant".into(),
		content:      Some(text.into()),
		tool_calls:   None,
		tool_call_id: None,
	}
}

enum Reply {
	Answer(ChatMessage),
	Wait {
		entered:     Arc<Notify>,
		release:     Arc<Semaphore>,
		message:     ChatMessage,
		cooperative: bool,
	},
}

struct Script {
	replies:   Mutex<VecDeque<Reply>>,
	histories: Mutex<Vec<Vec<ChatMessage>>>,
	session:   OnceLock<SessionHandle>,
}
impl Script {
	fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
		Self {
			replies:   Mutex::new(replies.into_iter().collect()),
			histories: Mutex::new(Vec::new()),
			session:   OnceLock::new(),
		}
	}
}
impl Model for Script {
	fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ChatMessage> {
		bail!("fixture requires the streaming interface")
	}

	fn stream(
		&self,
		messages: &[ChatMessage],
		_: &[ToolSpec],
		cancellation: &CancelToken,
		sink: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ChatMessage> {
		self.histories.lock().unwrap().push(messages.to_vec());
		if let Some(session) = self.session.get() {
			let snapshot = tokio::runtime::Handle::current().block_on(session.snapshot())?;
			assert_eq!(
				snapshot.state,
				SessionState::Running,
				"callbacks can query their owner without deadlocking"
			);
		}
		let reply = self
			.replies
			.lock()
			.unwrap()
			.pop_front()
			.ok_or_else(|| anyhow::anyhow!("fixture exhausted"))?;
		let message = match reply {
			Reply::Answer(message) => message,
			Reply::Wait { entered, release, message, cooperative } => {
				entered.notify_one();
				tokio::runtime::Handle::current().block_on(async {
					if cooperative {
						tokio::select! {
							_ = cancellation.wait() => bail!("cancelled fixture request"),
							permit = release.acquire() => { permit?.forget(); },
						}
					} else {
						release.acquire().await?.forget();
					}
					Ok::<(), anyhow::Error>(())
				})?;
				message
			},
		};
		if let Some(text) = &message.content {
			sink(text)?;
		}
		Ok(message)
	}
}

struct Allow;
impl Approval for Allow {
	fn approve(&self, _: &ToolCall) -> Result<bool> {
		Ok(true)
	}
}

async fn open(
	root: &Path,
	model: Arc<Script>,
	options: SessionEngineOptions,
) -> Result<SessionHandle> {
	SessionEngine::open(root, Path::new("session.jsonl"), None, model, Arc::new(Allow), options)
		.await
}

#[tokio::test]
async fn async_owner_resumes_the_current_compaction_context_instead_of_old_turns() -> Result<()> {
	let root = tempfile::tempdir()?;
	let rows = [
		serde_json::json!({"type":"session","version":3,"id":"context","timestamp":"2026-10-20T00:00:00Z","cwd":root.path()}),
		serde_json::json!({"type":"message","id":"old","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"superseded detail"}]}}),
		serde_json::json!({"type":"compaction","id":"summary","parentId":"old","summary":"Retained summary","firstKeptEntryId":"summary"}),
	];
	let mut bytes = Vec::new();
	for row in rows {
		serde_json::to_writer(&mut bytes, &row)?;
		bytes.push(b'\n');
	}
	std::fs::write(root.path().join("session.jsonl"), bytes)?;
	let model = Arc::new(Script::new([Reply::Answer(answer("resumed"))]));
	let session = open(root.path(), model.clone(), SessionEngineOptions::default()).await?;
	assert_eq!(session.submit("continue").await?.wait().await?, "resumed");
	session.shutdown().await?;
	let histories = model.histories.lock().unwrap();
	let sent = histories[0]
		.iter()
		.filter_map(|message| message.content.as_deref())
		.collect::<Vec<_>>()
		.join("\n");
	assert!(sent.contains("Retained summary"));
	assert!(!sent.contains("superseded detail"));
	Ok(())
}

#[tokio::test]
async fn queued_turns_commit_in_order_callbacks_can_reenter_and_shutdown_releases_the_writer()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Semaphore::new(0));
	let mut calls = answer("working");
	calls.tool_calls = Some(vec![
		ToolCall {
			id:       "write-1".into(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "write".into(),
				arguments: r#"{"path":"note","contents":"saved"}"#.into(),
			},
		},
		ToolCall {
			id:       "read-1".into(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "read".into(),
				arguments: r#"{"path":"note"}"#.into(),
			},
		},
	]);
	let model = Arc::new(Script::new([
		Reply::Wait {
			entered:     entered.clone(),
			release:     release.clone(),
			message:     calls,
			cooperative: true,
		},
		Reply::Answer(answer("first done")),
		Reply::Answer(answer("second done")),
		Reply::Answer(answer("third done")),
	]));
	let session = open(root.path(), model.clone(), SessionEngineOptions {
		follow_up_capacity: 2,
		..Default::default()
	})
	.await?;
	assert!(model.session.set(session.clone()).is_ok());
	let mut subscription = session.subscribe().await?;
	let first = session.submit("first").await?;
	entered.notified().await;
	let second = session.follow_up("second").await?;
	let third = session.follow_up("third").await?;
	assert!(matches!(session.submit("busy").await, Err(SessionError::Busy)));
	assert!(matches!(session.follow_up("overflow").await, Err(SessionError::QueueFull)));
	let snapshot = session.snapshot().await?;
	assert_eq!(snapshot.active, Some(first.handle.id()));
	assert_eq!(snapshot.queued, [second.handle.id(), third.handle.id()]);
	assert_eq!(snapshot.messages[0].content.as_deref(), Some("first"));
	release.add_permits(1);
	assert_eq!(first.wait().await?, "first done");
	assert_eq!(second.wait().await?, "second done");
	assert_eq!(third.wait().await?, "third done");
	let snapshot = session.snapshot().await?;
	assert_eq!(snapshot.state, SessionState::Idle);
	let prompts: Vec<_> = snapshot
		.messages
		.iter()
		.filter(|message| message.role == "user")
		.map(|message| message.content.as_deref().unwrap())
		.collect();
	assert_eq!(prompts, ["first", "second", "third"]);
	assert_eq!(std::fs::read_to_string(root.path().join("note"))?, "saved");
	let mut cursor = subscription.snapshot.sequence;
	let mut committed = 0;
	while let Ok(event) = subscription.events.try_recv() {
		assert_eq!(event.sequence, cursor + 1);
		cursor = event.sequence;
		if let SessionEngineEventKind::Committed { entry_id, .. } = &event.event {
			let document =
				SessionDocument::decode(&std::fs::read(root.path().join("session.jsonl"))?)?;
			assert!(
				document
					.entries()
					.iter()
					.any(|row| row["id"].get() == serde_json::to_string(entry_id).unwrap())
			);
			committed += 1;
		}
	}
	assert_eq!(committed, snapshot.messages.len());
	assert!(DurableSession::open(root.path(), Path::new("session.jsonl"), None).is_err());
	session.shutdown().await?;
	let next = Arc::new(Script::new([Reply::Answer(answer("remembered"))]));
	let reopened = open(root.path(), next.clone(), Default::default()).await?;
	assert_eq!(reopened.submit("recall").await?.wait().await?, "remembered");
	assert!(
		next.histories.lock().unwrap()[0]
			.iter()
			.any(|message| message.role == "tool"
				&& message.tool_call_id.as_deref() == Some("read-1")
				&& message.content.as_deref() == Some("saved"))
	);
	reopened.shutdown().await?;
	Ok(())
}

#[tokio::test]
async fn abort_parks_followups_and_old_handles_cannot_cancel_the_resumed_generation() -> Result<()>
{
	let root = tempfile::tempdir()?;
	let first_entered = Arc::new(Notify::new());
	let second_entered = Arc::new(Notify::new());
	let second_release = Arc::new(Semaphore::new(0));
	let model = Arc::new(Script::new([
		Reply::Wait {
			entered:     first_entered.clone(),
			release:     Arc::new(Semaphore::new(0)),
			message:     answer("discarded"),
			cooperative: true,
		},
		Reply::Wait {
			entered:     second_entered.clone(),
			release:     second_release.clone(),
			message:     answer("resumed"),
			cooperative: true,
		},
	]));
	let session = open(root.path(), model.clone(), Default::default()).await?;
	let first = session.submit("cancel").await?;
	let old = first.handle.clone();
	first_entered.notified().await;
	let second = session.follow_up("parked").await?;
	assert!(session.abort(&old).await?);
	assert_eq!(first.wait().await, Err(SessionError::Cancelled));
	let snapshot = session.snapshot().await?;
	assert!(snapshot.follow_ups_paused);
	assert_eq!(snapshot.state, SessionState::Idle);
	assert_eq!(snapshot.queued, [second.handle.id()]);
	assert_eq!(model.histories.lock().unwrap().len(), 1, "abort must not start queued work");
	assert!(session.resume_follow_ups().await?);
	second_entered.notified().await;
	old.abort();
	assert!(!session.abort(&old).await?);
	second_release.add_permits(1);
	assert_eq!(second.wait().await?, "resumed");
	assert!(
		!session
			.snapshot()
			.await?
			.messages
			.iter()
			.any(|message| message.content.as_deref() == Some("discarded"))
	);
	session.shutdown().await?;
	Ok(())
}

#[tokio::test]
async fn shutdown_rejects_queued_work_and_keeps_ownership_until_a_blocking_callback_drains()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Semaphore::new(0));
	let model = Arc::new(Script::new([Reply::Wait {
		entered:     entered.clone(),
		release:     release.clone(),
		message:     answer("too late"),
		cooperative: false,
	}]));
	let session = open(root.path(), model, Default::default()).await?;
	let active = session.submit("blocked callback").await?;
	entered.notified().await;
	let queued = session.follow_up("must not run").await?;
	let closing = session.clone();
	let mut shutdown = tokio::spawn(async move { closing.shutdown().await });
	assert_eq!(queued.wait().await, Err(SessionError::Shutdown));
	assert!(
		tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
			.await
			.is_err()
	);
	assert!(DurableSession::open(root.path(), Path::new("session.jsonl"), None).is_err());
	release.add_permits(1);
	assert_eq!(active.wait().await, Err(SessionError::Cancelled));
	tokio::time::timeout(Duration::from_secs(2), shutdown).await???;
	assert!(matches!(session.submit("closed").await, Err(SessionError::Closed)));
	let reopened = DurableSession::open(root.path(), Path::new("session.jsonl"), None)?;
	assert_eq!(reopened.messages().len(), 1);
	assert_eq!(reopened.messages()[0].content.as_deref(), Some("blocked callback"));
	Ok(())
}

#[tokio::test]
async fn lagged_subscribers_receive_an_error_and_can_resubscribe_to_committed_state() -> Result<()>
{
	let root = tempfile::tempdir()?;
	let model = Arc::new(Script::new([Reply::Answer(answer("finished"))]));
	let session =
		open(root.path(), model, SessionEngineOptions { event_capacity: 1, ..Default::default() })
			.await?;
	let mut observer = session.subscribe().await?;
	assert_eq!(session.submit("hello").await?.wait().await?, "finished");
	assert!(matches!(observer.events.recv().await, Err(broadcast::error::RecvError::Lagged(_))));
	let fresh = session.subscribe().await?;
	assert_eq!(fresh.snapshot.messages.len(), 2);
	assert_eq!(fresh.snapshot.messages[1].content.as_deref(), Some("finished"));
	assert_eq!(fresh.snapshot.state, SessionState::Idle);
	assert!(fresh.snapshot.sequence > observer.snapshot.sequence);
	session.shutdown().await?;
	Ok(())
}

#[tokio::test]
async fn cancellation_mid_tool_batch_keeps_completed_changes_and_pairs_every_skipped_call()
-> Result<()> {
	struct CancelSecond {
		calls:   std::sync::atomic::AtomicUsize,
		entered: Notify,
		release: Semaphore,
		run:     OnceLock<RunHandle>,
	}
	impl Approval for CancelSecond {
		fn approve(&self, _: &ToolCall) -> Result<bool> {
			let number = self
				.calls
				.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			if number == 0 {
				self.entered.notify_one();
				tokio::runtime::Handle::current()
					.block_on(self.release.acquire())?
					.forget();
			} else {
				self.run.get().unwrap().abort();
			}
			Ok(true)
		}
	}
	let root = tempfile::tempdir()?;
	let mut message = answer("write three files");
	message.tool_calls = Some(
		["first", "second", "third"]
			.into_iter()
			.map(|name| ToolCall {
				id:       name.into(),
				kind:     "function".into(),
				function: FunctionCall {
					name:      "write".into(),
					arguments: serde_json::json!({"path":name,"contents":"saved"}).to_string(),
				},
			})
			.collect(),
	);
	let model = Arc::new(Script::new([Reply::Answer(message)]));
	let approval = Arc::new(CancelSecond {
		calls:   std::sync::atomic::AtomicUsize::new(0),
		entered: Notify::new(),
		release: Semaphore::new(0),
		run:     OnceLock::new(),
	});
	let session = SessionEngine::open(
		root.path(),
		Path::new("session.jsonl"),
		None,
		model,
		approval.clone(),
		Default::default(),
	)
	.await?;
	let ticket = session.submit("write files").await?;
	assert!(approval.run.set(ticket.handle.clone()).is_ok());
	approval.entered.notified().await;
	approval.release.add_permits(1);
	assert_eq!(ticket.wait().await, Err(SessionError::Cancelled));
	assert_eq!(std::fs::read_to_string(root.path().join("first"))?, "saved");
	assert!(!root.path().join("second").exists());
	assert!(!root.path().join("third").exists());
	let snapshot = session.snapshot().await?;
	let results: Vec<_> = snapshot
		.messages
		.iter()
		.filter(|message| message.role == "tool")
		.collect();
	assert_eq!(
		results
			.iter()
			.map(|message| message.tool_call_id.as_deref().unwrap())
			.collect::<Vec<_>>(),
		["first", "second", "third"]
	);
	assert!(results[1].content.as_deref().unwrap().starts_with("error:"));
	assert!(results[2].content.as_deref().unwrap().starts_with("error:"));
	session.shutdown().await?;
	let reopened = DurableSession::open(root.path(), Path::new("session.jsonl"), None)?;
	assert_eq!(
		reopened.messages().len(),
		5,
		"cancelled batches remain resumable instead of orphaning results"
	);
	Ok(())
}

#[tokio::test]
async fn sdk_abort_interrupts_the_real_http_transport_while_a_partial_frame_is_pending()
-> Result<()> {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	let root = tempfile::tempdir()?;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let address = listener.local_addr()?;
	let ready = Arc::new(Notify::new());
	let server_ready = ready.clone();
	let server = tokio::spawn(async move {
		let (mut socket, _) = listener.accept().await?;
		let mut request = Vec::new();
		let mut buffer = [0; 2048];
		loop {
			let count = socket.read(&mut buffer).await?;
			if count == 0 {
				bail!("request ended early");
			}
			request.extend_from_slice(&buffer[..count]);
			if request.len() > 64 * 1024 {
				bail!("fixture request exceeds bound");
			}
			if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
				let headers = String::from_utf8_lossy(&request[..end]);
				let length: usize = headers
					.lines()
					.find_map(|line| {
						line
							.to_ascii_lowercase()
							.strip_prefix("content-length:")
							.map(|value| value.trim().parse().unwrap())
					})
					.unwrap();
				if request.len() >= end + 4 + length {
					break;
				}
			}
		}
		socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100000\r\n\r\ndata: {\"choices\":").await?;
		server_ready.notify_one();
		assert_eq!(socket.read(&mut buffer).await?, 0, "abort must close the pending connection");
		Ok::<(), anyhow::Error>(())
	});
	let model = Arc::new(HttpModel::new(Settings {
		model:     "fixture".into(),
		base_url:  format!("http://{address}/v1"),
		api_key:   String::new(),
		max_steps: 1,
	}));
	let session = SessionEngine::open(
		root.path(),
		Path::new("session.jsonl"),
		None,
		model,
		Arc::new(Allow),
		Default::default(),
	)
	.await?;
	let run = session.submit("cancel pending frame").await?;
	ready.notified().await;
	assert!(session.abort(&run.handle).await?);
	assert_eq!(
		tokio::time::timeout(Duration::from_secs(2), run.wait()).await?,
		Err(SessionError::Cancelled)
	);
	tokio::time::timeout(Duration::from_secs(2), server).await???;
	let snapshot = session.snapshot().await?;
	assert_eq!(snapshot.messages.len(), 1);
	assert_eq!(snapshot.state, SessionState::Idle);
	session.shutdown().await?;
	Ok(())
}
