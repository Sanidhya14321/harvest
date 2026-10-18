use std::{
	collections::VecDeque,
	path::Path,
	sync::{Arc, Mutex},
	time::Duration,
};

use anyhow::{Result, bail};
use harvest_omp::{
	chat::{ChatMessage, FunctionCall, ToolCall, ToolSpec},
	durable_session::DurableSession,
	provider::Model,
	rpc::{RpcOptions, serve},
	session_engine::{SessionEngine, SessionEngineOptions, SessionHandle},
	session_format::SessionDocument,
	tools::DenyMutations,
};
use harvest_runtime::{MAX_EVENT_BYTES, cancel::CancelToken};
use serde_json::{Value, json};
use tokio::{
	io::{
		AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
	},
	sync::{Notify, Semaphore},
	task::JoinHandle,
};

fn answer(text: &str) -> ChatMessage {
	ChatMessage {
		role:         "assistant".into(),
		content:      Some(text.into()),
		tool_calls:   None,
		tool_call_id: None,
	}
}
struct Reply {
	message: ChatMessage,
	gate:    Option<(Arc<Notify>, Arc<Semaphore>)>,
}
struct Script(Mutex<VecDeque<Reply>>);
impl Model for Script {
	fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ChatMessage> {
		bail!("fixture requires streaming")
	}

	fn stream(
		&self,
		_: &[ChatMessage],
		_: &[ToolSpec],
		cancellation: &CancelToken,
		sink: &mut dyn FnMut(&str) -> Result<()>,
	) -> Result<ChatMessage> {
		let reply = self
			.0
			.lock()
			.unwrap()
			.pop_front()
			.ok_or_else(|| anyhow::anyhow!("fixture exhausted"))?;
		if let Some(text) = &reply.message.content {
			sink(text)?;
		}
		if let Some((entered, release)) = reply.gate {
			entered.notify_one();
			tokio::runtime::Handle::current().block_on(async {
				tokio::select! {
					_ = cancellation.wait() => bail!("request cancelled"),
					permit = release.acquire() => { permit?.forget(); },
				}
				Ok::<(), anyhow::Error>(())
			})?;
		}
		Ok(reply.message)
	}
}
fn immediate(text: &str) -> Reply {
	Reply { message: answer(text), gate: None }
}

struct Connection {
	input:  WriteHalf<DuplexStream>,
	output: BufReader<ReadHalf<DuplexStream>>,
	task:   JoinHandle<Result<()>>,
	frames: Vec<Value>,
}
impl Connection {
	async fn open(root: &Path, replies: Vec<Reply>) -> Result<Self> {
		let session = SessionEngine::open(
			root,
			Path::new("rpc.jsonl"),
			None,
			Arc::new(Script(Mutex::new(replies.into()))),
			Arc::new(DenyMutations),
			SessionEngineOptions { event_capacity: 256, ..Default::default() },
		)
		.await?;
		Self::with_session(root, session).await
	}

	async fn with_session(root: &Path, session: SessionHandle) -> Result<Self> {
		let (client, server) = tokio::io::duplex(8192);
		let (input, output) = tokio::io::split(server);
		let options =
			RpcOptions { session_file: root.join("rpc.jsonl"), model: "fixture".into() };
		let task = tokio::spawn(serve(session, input, output, options));
		let (read, write) = tokio::io::split(client);
		let mut connection =
			Self { input: write, output: BufReader::new(read), task, frames: Vec::new() };
		let ready = connection.next().await?;
		assert_eq!(ready["type"], "ready");
		assert_eq!(ready["supportedProtocolVersions"], json!([1]));
		Ok(connection)
	}

	async fn send(&mut self, value: Value) -> Result<()> {
		let mut bytes = serde_json::to_vec(&value)?;
		bytes.push(b'\n');
		self.input.write_all(&bytes).await?;
		Ok(())
	}

	async fn next(&mut self) -> Result<Value> {
		let mut bytes = Vec::new();
		let count =
			tokio::time::timeout(Duration::from_secs(10), self.output.read_until(b'\n', &mut bytes))
				.await??;
		if count == 0 {
			bail!("RPC output closed before expected frame");
		}
		assert!(bytes.len() <= MAX_EVENT_BYTES, "client receives bounded physical frames");
		let frame: Value = serde_json::from_slice(&bytes)?;
		self.frames.push(frame.clone());
		Ok(frame)
	}

	async fn response(&mut self, id: &str) -> Result<Value> {
		loop {
			let frame = self.next().await?;
			if frame["type"] == "response" && frame["id"] == id {
				return Ok(frame);
			}
		}
	}

	async fn terminal(&mut self) -> Result<Value> {
		loop {
			let frame = self.next().await?;
			if frame["type"] == "agent_end" && frame["isTerminal"] == true {
				return Ok(frame);
			}
		}
	}

	async fn close(mut self) -> Result<Vec<Value>> {
		self.input.shutdown().await?;
		let mut tail = Vec::new();
		tokio::time::timeout(Duration::from_secs(10), self.output.read_to_end(&mut tail)).await??;
		for line in tail
			.split(|byte| *byte == b'\n')
			.filter(|line| !line.is_empty())
		{
			self.frames.push(serde_json::from_slice(line)?);
		}
		tokio::time::timeout(Duration::from_secs(10), self.task).await???;
		Ok(self.frames)
	}
}

#[tokio::test]
async fn rpc_queues_text_turns_preserves_wire_records_and_denies_mutations_without_host_approval()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Semaphore::new(0));
	let mut calling = answer("working");
	calling.tool_calls = Some(vec![ToolCall {
		id:       "write-1".into(),
		kind:     "function".into(),
		function: FunctionCall {
			name:      "write".into(),
			arguments: r#"{"path":"denied","contents":"must not be written"}"#.into(),
		},
	}]);
	let mut client = Connection::open(root.path(), vec![
		Reply { message: calling, gate: Some((entered.clone(), release.clone())) },
		immediate("first done"),
		immediate("second done"),
	])
	.await?;
	client
		.send(json!({"id":"p1","type":"prompt","message":"first"}))
		.await?;
	assert_eq!(client.response("p1").await?["data"]["agentInvoked"], true);
	entered.notified().await;
	client
		.send(json!({"id":"p2","type":"follow_up","message":"second"}))
		.await?;
	assert_eq!(client.response("p2").await?["success"], true);
	client
		.send(json!({"id":"busy","type":"prompt","message":"must not overlap"}))
		.await?;
	assert_eq!(client.response("busy").await?["success"], false);
	// A partial UTF-8 input frame survives concurrent streamed output and
	// completed engine events before its remaining bytes arrive.
	let state = serde_json::to_vec(&json!({"id":"state🦀","type":"get_state"}))?;
	let split = state.iter().position(|byte| *byte == 0xf0).unwrap() + 2;
	client.input.write_all(&state[..split]).await?;
	release.add_permits(1);
	client.terminal().await?;
	client.input.write_all(&state[split..]).await?;
	client.input.write_all(b"\n").await?;
	let state = client.response("state🦀").await?;
	assert_eq!(state["data"]["messageCount"], 6);
	assert_eq!(state["data"]["isStreaming"], false);
	client
		.send(json!({"id":"history","type":"get_messages"}))
		.await?;
	let history = client.response("history").await?;
	let messages = history["data"]["messages"].as_array().unwrap();
	assert_eq!(
		messages
			.iter()
			.map(|message| message["role"].as_str().unwrap())
			.collect::<Vec<_>>(),
		["user", "assistant", "toolResult", "assistant", "user", "assistant"]
	);
	assert_eq!(messages[2]["toolName"], "write");
	assert_eq!(messages[2]["toolCallId"], "write-1");
	assert_eq!(messages[2]["isError"], true);
	assert!(!root.path().join("denied").exists());
	let frames = client.close().await?;
	let ends: Vec<_> = frames
		.iter()
		.filter(|frame| frame["type"] == "agent_end")
		.collect();
	assert_eq!(
		ends
			.iter()
			.map(|frame| frame["isTerminal"].as_bool().unwrap())
			.collect::<Vec<_>>(),
		[false, true]
	);
	let deltas: Vec<_> = frames
		.iter()
		.filter(|frame| frame["assistantMessageEvent"]["type"] == "text_delta")
		.map(|frame| frame["assistantMessageEvent"]["delta"].as_str().unwrap())
		.collect();
	assert_eq!(deltas, ["working", "first done", "second done"]);
	let document = SessionDocument::decode(&std::fs::read(root.path().join("rpc.jsonl"))?)?;
	let stored: Vec<Value> = document
		.entries()
		.iter()
		.map(|entry| serde_json::from_str(entry["message"].get()).unwrap())
		.collect();
	assert_eq!(messages, &stored, "RPC history returns durable timestamps and tool metadata");
	let reopened = DurableSession::open(root.path(), Path::new("rpc.jsonl"), None)?;
	assert_eq!(reopened.messages().len(), 6);
	Ok(())
}

#[tokio::test]
async fn rpc_stale_abort_cannot_cancel_current_work_and_explicit_resume_unparks_followups()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Semaphore::new(0));
	let mut client = Connection::open(root.path(), vec![
		Reply { message: answer("provisional"), gate: Some((entered.clone(), release)) },
		immediate("resumed"),
	])
	.await?;
	client
		.send(json!({"id":"first","type":"prompt","message":"cancel me"}))
		.await?;
	client.response("first").await?;
	entered.notified().await;
	client
		.send(json!({"id":"queued","type":"follow_up","message":"keep queued"}))
		.await?;
	client.response("queued").await?;
	client
		.send(json!({"id":"state","type":"get_state"}))
		.await?;
	let generation = client.response("state").await?["data"]["runGeneration"]
		.as_u64()
		.unwrap();
	client
		.send(json!({"id":"malformed-generation","type":"abort","runGeneration":"invalid"}))
		.await?;
	assert_eq!(client.response("malformed-generation").await?["success"], false);
	client
		.send(json!({"id":"stale","type":"abort","runGeneration":generation-1}))
		.await?;
	assert_eq!(client.response("stale").await?["success"], true);
	client.send(json!({"id":"live","type":"get_state"})).await?;
	assert_eq!(client.response("live").await?["data"]["isStreaming"], true);
	client
		.send(json!({"id":"cancel","type":"abort","runGeneration":generation}))
		.await?;
	assert_eq!(client.response("cancel").await?["success"], true);
	client
		.send(json!({"id":"parked","type":"get_state"}))
		.await?;
	let parked = client.response("parked").await?;
	assert_eq!(parked["data"]["isStreaming"], false);
	assert_eq!(parked["data"]["queuedMessageCount"], 1);
	assert_eq!(parked["data"]["messageCount"], 1);
	client
		.send(json!({"id":"resume","type":"resume_follow_ups"}))
		.await?;
	assert_eq!(client.response("resume").await?["data"]["resumed"], true);
	let terminal = client.terminal().await?;
	assert_eq!(terminal["messages"][1]["content"][0]["text"], "resumed");
	let frames = client.close().await?;
	assert!(
		frames
			.iter()
			.any(|frame| frame["assistantMessageEvent"]["type"] == "error"
				&& frame["assistantMessageEvent"]["reason"] == "aborted")
	);
	let reopened = DurableSession::open(root.path(), Path::new("rpc.jsonl"), None)?;
	assert_eq!(
		reopened
			.messages()
			.iter()
			.map(|message| message.content.as_deref().unwrap())
			.collect::<Vec<_>>(),
		["cancel me", "keep queued", "resumed"]
	);
	Ok(())
}

#[tokio::test]
async fn rpc_recovers_from_malformed_and_oversized_frames_and_never_acknowledges_unported_features()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let mut client = Connection::open(root.path(), vec![]).await?;
	client.input.write_all(b"not JSON\n").await?;
	let parse = client.next().await?;
	assert_eq!(parse["command"], "parse");
	assert_eq!(parse["success"], false);
	assert!(parse.get("id").is_none());
	let mut oversized = vec![b'x'; MAX_EVENT_BYTES + 1];
	oversized.push(b'\n');
	client.input.write_all(&oversized).await?;
	let parse = client.next().await?;
	assert_eq!(parse["command"], "parse");
	assert_eq!(parse["success"], false);
	for request in [
		json!({"id":"feature","type":"compact"}),
		json!({"id":"images","type":"prompt","message":"image","images":[{"type":"image","data":"AA==","mimeType":"image/png"}]}),
		json!({"id":"steer","type":"prompt","message":"steer","streamingBehavior":"steer"}),
		json!({"id":"slash","type":"prompt","message":"/clear"}),
		json!({"id":"v2","type":"negotiate_protocol","protocolVersion":2}),
	] {
		let id = request["id"].as_str().unwrap().to_owned();
		client.send(request).await?;
		assert_eq!(client.response(&id).await?["success"], false);
	}
	client
		.send(json!({"id":"untrusted-id","type":"unknown"}))
		.await?;
	let unknown = client.next().await?;
	assert_eq!(unknown["command"], "unknown");
	assert!(unknown.get("id").is_none(), "unknown command IDs are not trusted correlations");
	client
		.send(json!({"id":"after-errors","type":"get_state"}))
		.await?;
	let state = client.response("after-errors").await?;
	assert_eq!(state["success"], true);
	assert_eq!(state["data"]["messageCount"], 0);
	client.close().await?;
	Ok(())
}

#[tokio::test]
async fn rpc_refuses_oversized_history_without_truncation_and_keeps_serving_state() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut durable = DurableSession::open(root.path(), Path::new("rpc.jsonl"), None)?;
	for _ in 0..2 {
		durable.commit(&harvest_omp::chat::user("x".repeat(MAX_EVENT_BYTES / 2 + 1)), "fixture")?;
	}
	drop(durable);
	let before = std::fs::read(root.path().join("rpc.jsonl"))?;
	let mut client = Connection::open(root.path(), vec![]).await?;
	client
		.send(json!({"id":"large","type":"get_messages"}))
		.await?;
	let history = client.response("large").await?;
	assert_eq!(history["success"], false);
	assert!(history.get("data").is_none());
	client
		.send(json!({"id":"count","type":"get_state"}))
		.await?;
	assert_eq!(client.response("count").await?["data"]["messageCount"], 2);
	client.close().await?;
	assert_eq!(std::fs::read(root.path().join("rpc.jsonl"))?, before);
	Ok(())
}

#[tokio::test]
async fn rpc_disconnect_cancels_pending_transport_and_releases_the_writer_before_returning()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Semaphore::new(0));
	let mut client = Connection::open(root.path(), vec![Reply {
		message: answer("pending"),
		gate:    Some((entered.clone(), release)),
	}])
	.await?;
	client
		.send(json!({"id":"p","type":"prompt","message":"disconnect"}))
		.await?;
	client.response("p").await?;
	entered.notified().await;
	client.close().await?;
	let reopened = DurableSession::open(root.path(), Path::new("rpc.jsonl"), None)?;
	assert_eq!(reopened.messages().len(), 1);
	assert_eq!(reopened.messages()[0].content.as_deref(), Some("disconnect"));
	Ok(())
}
