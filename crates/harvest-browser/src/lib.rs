//! Native CDP control. Chrome owns its dedicated persistent data; this owner
//! holds the profile lock and reaps its child. No JavaScript is evaluated.
use std::{
	collections::VecDeque,
	io,
	net::{Ipv4Addr, SocketAddr, TcpStream},
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
	thread,
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, cancel::CancelToken, filesystem::RootLock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};

const MARKER: &str = "harvest-browser-profile.json";
const OWNER: &[u8] = b"{\"owner\":\"harvest-native-cdp\",\"version\":1}\n";
const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_REQUEST: usize = 128 * 1024;
const MAX_EVENTS: usize = 16_384;
const IO_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct Options {
	pub chrome:     PathBuf,
	pub profile:    PathBuf,
	pub headless:   bool,
	/// Explicit opt-in for constrained test containers, never the default.
	pub no_sandbox: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
	Tabs {},
	Open { url: String },
	Navigate { tab: String, url: String },
	Document { tab: String },
	Query { tab: String, selector: String },
	Click { tab: String, selector: String },
	Type { tab: String, selector: String, text: String },
	Screenshot { tab: String },
	Close { tab: String },
}

/// Chrome is killed/reaped before releasing the profile ownership lock.
struct Chrome {
	child:   Child,
	profile: JailedFs,
	_owner:  RootLock,
}

impl Drop for Chrome {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

struct Connection {
	socket:      WebSocket<TcpStream>,
	next_id:     u64,
	poisoned:    bool,
	load_events: VecDeque<Value>,
}

impl Connection {
	fn connect(address: SocketAddr, path: &str, cancel: &CancelToken) -> Result<Self> {
		cancel.heartbeat()?;
		if !path.starts_with("/devtools/browser/")
			|| path.len() > 256
			|| !path
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
		{
			bail!("Chrome published an invalid CDP endpoint");
		}
		let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
		stream.set_read_timeout(Some(Duration::from_secs(2)))?;
		stream.set_write_timeout(Some(Duration::from_secs(2)))?;
		let config = WebSocketConfig::default()
			.max_message_size(Some(MAX_FRAME))
			.max_frame_size(Some(MAX_FRAME))
			.write_buffer_size(128 * 1024)
			.max_write_buffer_size(MAX_FRAME + MAX_REQUEST);
		let (socket, _) = tungstenite::client::client_with_config(
			format!("ws://{address}{path}"),
			stream,
			Some(config),
		)
		.map_err(|_| anyhow::anyhow!("Chrome CDP handshake failed"))?;
		socket.get_ref().set_read_timeout(Some(IO_POLL))?;
		socket.get_ref().set_write_timeout(Some(IO_POLL))?;
		cancel.heartbeat()?;
		Ok(Self { socket, next_id: 0, poisoned: false, load_events: VecDeque::new() })
	}

	fn call(
		&mut self,
		method: &str,
		params: Value,
		session: Option<&str>,
		cancel: &CancelToken,
	) -> Result<Value> {
		if self.poisoned {
			bail!("CDP connection closed after an interrupted operation; reopen the browser");
		}
		cancel.heartbeat()?;
		self.next_id = self
			.next_id
			.checked_add(1)
			.context("CDP request IDs exhausted")?;
		let id = self.next_id;
		let mut request = json!({"id":id,"method":method,"params":params});
		if let Some(session) = session {
			request["sessionId"] = json!(session);
		}
		let request = serde_json::to_string(&request)?;
		if request.len() > MAX_REQUEST {
			bail!("CDP request exceeds 128 KiB");
		}
		// An uncertain write/read must never be replayed: click/type/navigation
		// can mutate browser state even if their response has not arrived.
		self.poisoned = true;
		self.socket.send(Message::Text(request.into()))?;
		for _ in 0..MAX_EVENTS {
			cancel.heartbeat()?;
			let frame = match self.socket.read() {
				Ok(frame) => frame,
				Err(tungstenite::Error::Io(error))
					if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
				{
					continue;
				},
				Err(_) => bail!("Chrome CDP connection closed"),
			};
			match frame {
				Message::Text(text) => {
					let message: Value = serde_json::from_str(&text).context("invalid CDP JSON")?;
					if message.get("id").and_then(Value::as_u64) != Some(id) {
						self.record_event(message);
						continue;
					}
					if message.get("sessionId").and_then(Value::as_str) != session {
						bail!("CDP response belongs to a different session");
					}
					if message.get("error").is_some() {
						self.poisoned = false;
						bail!("Chrome rejected CDP operation {method}");
					}
					let result = message
						.get("result")
						.cloned()
						.context("CDP response has no result")?;
					self.poisoned = false;
					return Ok(result);
				},
				Message::Ping(_) => self.socket.flush()?,
				Message::Pong(_) => {},
				Message::Close(_) => bail!("Chrome CDP connection closed"),
				_ => bail!("unexpected binary CDP frame"),
			}
		}
		bail!("CDP event drain exceeded its bound")
	}

	fn record_event(&mut self, event: Value) {
		if event["method"] == "Page.lifecycleEvent" && event["params"]["name"] == "load" {
			if self.load_events.len() == 128 {
				self.load_events.pop_front();
			}
			self.load_events.push_back(event);
		}
	}

	fn wait_load(&mut self, session: &str, loader: &str, cancel: &CancelToken) -> Result<()> {
		self.poisoned = true;
		for _ in 0..MAX_EVENTS {
			cancel.heartbeat()?;
			if let Some(index) = self.load_events.iter().position(|event| {
				event["sessionId"] == session && event["params"]["loaderId"] == loader
			}) {
				self.load_events.remove(index);
				self.poisoned = false;
				return Ok(());
			}
			match self.socket.read() {
				Ok(Message::Text(text)) => {
					self.record_event(serde_json::from_str(&text).context("invalid CDP event")?);
				},
				Ok(Message::Ping(_)) => self.socket.flush()?,
				Ok(Message::Pong(_)) => {},
				Err(tungstenite::Error::Io(error))
					if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {},
				_ => bail!("Chrome closed while waiting for navigation"),
			}
		}
		bail!("navigation event drain exceeded its bound")
	}
}

pub struct Browser {
	connection: Connection,
	chrome:     Chrome,
}

fn validate_url(url: &str) -> Result<()> {
	// No javascript:, file:, browser-internal URLs or user-data script payloads.
	// HTTP redirects remain browser navigation, governed by Chrome's origin rules.
	if url != "about:blank" && !url.starts_with("https://") && !url.starts_with("http://") {
		bail!("browser URLs must use http:// or https:// (or about:blank)");
	}
	if url.len() > 16 * 1024 || url.chars().any(char::is_control) {
		bail!("invalid browser URL");
	}
	Ok(())
}

impl Browser {
	pub fn launch(options: &Options, cancel: &CancelToken) -> Result<Self> {
		let budget = cancel.child_with_timeout(Duration::from_secs(15));
		let cancel = &budget;
		cancel.heartbeat()?;
		let profile = JailedFs::open_or_create(&options.profile)?;
		let owner = profile.lock(Path::new(".harvest-browser.lock"))?;
		match profile.read_optional(Path::new(MARKER), 1024)? {
			Some(marker) if marker == OWNER => {},
			Some(_) => bail!("profile is not owned by this Harvest browser protocol"),
			None => {
				if profile
					.entries(Path::new(""), 4)?
					.iter()
					.any(|e| e.name != ".harvest-browser.lock")
				{
					bail!(
						"refusing an existing unmarked browser profile; choose a new dedicated directory"
					);
				}
				profile.atomic_write(Path::new(MARKER), OWNER)?;
			},
		}
		if profile.exists(Path::new("DevToolsActivePort"))? {
			profile.remove(Path::new("DevToolsActivePort"))?;
		}
		let mut command = Command::new(&options.chrome);
		command
			.arg(format!("--user-data-dir={}", profile.root().display()))
			.args([
				"--remote-debugging-address=127.0.0.1",
				"--remote-debugging-port=0",
				"--no-first-run",
				"--no-default-browser-check",
			])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null());
		if options.headless {
			command.arg("--headless=new");
		}
		if options.no_sandbox {
			command.arg("--no-sandbox");
		}
		command.arg("about:blank");
		let child = command.spawn().context("could not start Chrome")?;
		let mut chrome = Chrome { child, profile, _owner: owner };
		let startup = Instant::now();
		let endpoint = loop {
			cancel.heartbeat()?;
			if chrome.child.try_wait()?.is_some() {
				bail!("Chrome exited during startup");
			}
			if startup.elapsed() > Duration::from_secs(15) {
				bail!("Chrome did not publish its CDP endpoint within 15 seconds");
			}
			if let Some(bytes) = chrome
				.profile
				.read_optional(Path::new("DevToolsActivePort"), 1024)?
			{
				let text = std::str::from_utf8(&bytes)?;
				let mut lines = text.lines();
				if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
					let port: u16 = port.parse().context("invalid Chrome debugging port")?;
					if port == 0 || lines.next().is_some() {
						bail!("invalid Chrome endpoint file");
					}
					break (SocketAddr::from((Ipv4Addr::LOCALHOST, port)), path.to_owned());
				}
			}
			thread::sleep(Duration::from_millis(20));
		};
		let connection = Connection::connect(endpoint.0, &endpoint.1, cancel)?;
		Ok(Self { connection, chrome })
	}

	fn call(
		&mut self,
		method: &str,
		params: Value,
		session: Option<&str>,
		cancel: &CancelToken,
	) -> Result<Value> {
		// A caller cannot accidentally create an unbounded browser operation.
		let combined = cancel.child_with_timeout(Duration::from_secs(30));
		self.connection.call(method, params, session, &combined)
	}

	fn attach(&mut self, tab: &str, cancel: &CancelToken) -> Result<String> {
		if tab.is_empty() || tab.len() > 256 {
			bail!("invalid browser tab ID");
		}
		let result = self.call(
			"Target.attachToTarget",
			json!({"targetId":tab,"flatten":true}),
			None,
			cancel,
		)?;
		Ok(result["sessionId"]
			.as_str()
			.context("Chrome did not return a session ID")?
			.to_owned())
	}

	fn node(&mut self, selector: &str, session: &str, cancel: &CancelToken) -> Result<Value> {
		if selector.is_empty() || selector.len() > 16 * 1024 {
			bail!("invalid DOM selector");
		}
		let document = self.call("DOM.getDocument", json!({"depth":0}), Some(session), cancel)?;
		let root = document["root"]["nodeId"]
			.as_i64()
			.context("missing DOM root")?;
		let found = self.call(
			"DOM.querySelector",
			json!({"nodeId":root,"selector":selector}),
			Some(session),
			cancel,
		)?;
		let node = found["nodeId"]
			.as_i64()
			.filter(|id| *id > 0)
			.context("DOM selector matched no element")?;
		Ok(json!(node))
	}

	pub fn perform(&mut self, action: Action, cancel: &CancelToken) -> Result<Value> {
		let budget = cancel.child_with_timeout(Duration::from_secs(30));
		let result = self.perform_with_budget(action, &budget);
		if budget.aborted() {
			// A multi-command action can be interrupted between mouse press and
			// release, even when each completed CDP response was received.
			self.connection.poisoned = true;
			budget.heartbeat()?;
		}
		result
	}

	fn perform_with_budget(&mut self, action: Action, cancel: &CancelToken) -> Result<Value> {
		cancel.heartbeat()?;
		match action {
			Action::Tabs {} => {
				let targets = self.call("Target.getTargets", json!({}), None, cancel)?;
				let pages: Vec<_> = targets["targetInfos"]
					.as_array()
					.context("missing target list")?
					.iter()
					.filter(|v| v["type"] == "page")
					.map(|v| json!({"tab":v["targetId"],"url":v["url"],"title":v["title"]}))
					.collect();
				if pages.len() > 128 {
					bail!("browser tab limit exceeded");
				}
				Ok(json!({"tabs":pages}))
			},
			Action::Open { url } => {
				validate_url(&url)?;
				let current = self.perform(Action::Tabs {}, cancel)?;
				if current["tabs"].as_array().context("missing tabs")?.len() >= 128 {
					bail!("browser tab limit reached");
				}
				let opened =
					self.call("Target.createTarget", json!({"url":"about:blank"}), None, cancel)?;
				let tab = opened["targetId"]
					.as_str()
					.context("Chrome did not return a tab ID")?
					.to_owned();
				if let Err(error) =
					self.perform(Action::Navigate { tab: tab.clone(), url: url.clone() }, cancel)
				{
					if !self.connection.poisoned {
						let _ = self.call("Target.closeTarget", json!({"targetId":tab}), None, cancel);
					}
					return Err(error);
				}
				Ok(json!({"tab":tab,"url":url}))
			},
			Action::Close { tab } => {
				let result = self.call("Target.closeTarget", json!({"targetId":tab}), None, cancel)?;
				if result["success"] != true {
					bail!("Chrome did not close the requested tab");
				}
				Ok(result)
			},
			action => {
				let tab = match &action {
					Action::Navigate { tab, .. }
					| Action::Document { tab }
					| Action::Query { tab, .. }
					| Action::Click { tab, .. }
					| Action::Type { tab, .. }
					| Action::Screenshot { tab } => tab.clone(),
					_ => unreachable!(),
				};
				let session = self.attach(&tab, cancel)?;
				let result = self.in_tab(action, &session, cancel);
				// Detach after every operation. On cancellation the uncertain
				// connection is poisoned, and owned Chrome is reaped on drop.
				if !self.connection.poisoned {
					let detached =
						self.call("Target.detachFromTarget", json!({"sessionId":session}), None, cancel);
					if result.is_ok() {
						detached?;
					}
				}
				result
			},
		}
	}

	fn in_tab(&mut self, action: Action, session: &str, cancel: &CancelToken) -> Result<Value> {
		let result = match action {
			Action::Navigate { url, .. } => {
				validate_url(&url)?;
				self.call("Page.enable", json!({}), Some(session), cancel)?;
				self.call(
					"Page.setLifecycleEventsEnabled",
					json!({"enabled":true}),
					Some(session),
					cancel,
				)?;
				self.connection.load_events.clear();
				let result = self.call("Page.navigate", json!({"url":url}), Some(session), cancel)?;
				if result.get("errorText").is_some() {
					bail!("Chrome navigation failed");
				}
				if let Some(loader) = result["loaderId"].as_str() {
					self.connection.wait_load(session, loader, cancel)?;
				}
				result
			},
			Action::Document { .. } => self.call(
				"DOM.getDocument",
				json!({"depth":-1,"pierce":true}),
				Some(session),
				cancel,
			)?,
			Action::Query { selector, .. } => {
				let node = self.node(&selector, session, cancel)?;
				self.call(
					"DOM.describeNode",
					json!({"nodeId":node,"depth":1}),
					Some(session),
					cancel,
				)?
			},
			Action::Type { selector, text, .. } => {
				if text.len() > 64 * 1024 {
					bail!("browser input exceeds 64 KiB");
				}
				let node = self.node(&selector, session, cancel)?;
				self.call("DOM.focus", json!({"nodeId":node}), Some(session), cancel)?;
				self.call("Input.insertText", json!({"text":text}), Some(session), cancel)?
			},
			Action::Click { selector, .. } => {
				let node = self.node(&selector, session, cancel)?;
				self.call(
					"DOM.scrollIntoViewIfNeeded",
					json!({"nodeId":node}),
					Some(session),
					cancel,
				)?;
				let model =
					self.call("DOM.getBoxModel", json!({"nodeId":node}), Some(session), cancel)?;
				let quad = model["model"]["content"]
					.as_array()
					.filter(|q| q.len() == 8)
					.context("element has no clickable box")?;
				let x = f64::midpoint(
					quad[0].as_f64().context("invalid box")?,
					quad[4].as_f64().context("invalid box")?,
				);
				let y = f64::midpoint(
					quad[1].as_f64().context("invalid box")?,
					quad[5].as_f64().context("invalid box")?,
				);
				for event in ["mousePressed", "mouseReleased"] {
					self.call(
						"Input.dispatchMouseEvent",
						json!({"type":event,"x":x,"y":y,"button":"left","clickCount":1}),
						Some(session),
						cancel,
					)?;
				}
				json!({"clicked":true})
			},
			Action::Screenshot { .. } => {
				let result = self.call(
					"Page.captureScreenshot",
					json!({"format":"png","captureBeyondViewport":false}),
					Some(session),
					cancel,
				)?;
				let data = result["data"].as_str().context("missing screenshot")?;
				if data.len() > 12 * 1024 * 1024 {
					bail!("screenshot exceeds 12 MiB");
				}
				json!({"mimeType":"image/png","data":data})
			},
			_ => unreachable!(),
		};
		Ok(result)
	}
}

impl Drop for Browser {
	fn drop(&mut self) {
		let cancel = CancelToken::with_timeout(Some(Duration::from_secs(2)));
		if !self.connection.poisoned {
			let _ = self
				.connection
				.call("Browser.close", json!({}), None, &cancel);
		}
		let until = Instant::now() + Duration::from_secs(2);
		while Instant::now() < until {
			if !matches!(self.chrome.child.try_wait(), Ok(None)) {
				break;
			}
			thread::sleep(Duration::from_millis(20));
		}
		// Chrome's Drop is the bounded kill/reap fallback, including failed CDP.
	}
}

#[cfg(test)]
mod tests {
	use std::net::TcpListener;

	use super::*;

	fn cdp_peer(
		handler: impl FnOnce(WebSocket<TcpStream>) + Send + 'static,
	) -> Result<(Connection, thread::JoinHandle<()>)> {
		let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
		let address = listener.local_addr()?;
		let worker = thread::spawn(move || {
			let (stream, _) = listener.accept().unwrap();
			stream
				.set_read_timeout(Some(Duration::from_secs(2)))
				.unwrap();
			stream
				.set_write_timeout(Some(Duration::from_secs(2)))
				.unwrap();
			handler(tungstenite::accept(stream).unwrap());
		});
		let connection =
			Connection::connect(address, "/devtools/browser/test", &CancelToken::default())?;
		Ok((connection, worker))
	}

	#[test]
	fn typed_commands_reject_code_and_non_network_urls() {
		for url in [
			"javascript:alert(1)",
			"file:///private",
			"data:text/html,test",
			"chrome://settings",
			"http://a\n",
		] {
			assert!(validate_url(url).is_err());
		}
		for url in ["about:blank", "https://example.test/path", "http://127.0.0.1:9000/"] {
			assert!(validate_url(url).is_ok());
		}
		for input in [r#"{"action":"run","code":"x"}"#, r#"{"action":"tabs","code":"x"}"#] {
			assert!(serde_json::from_str::<Action>(input).is_err());
		}
	}

	#[test]
	fn existing_unmarked_and_lockedprofiles_are_refused_before_spawning() -> Result<()> {
		let root = tempfile::tempdir()?;
		let options = Options {
			chrome:     root.path().join("no-such-chrome"),
			profile:    root.path().join("profile"),
			headless:   true,
			no_sandbox: false,
		};
		let fs = JailedFs::open_or_create(&options.profile)?;
		fs.atomic_write(Path::new("user-data"), b"keep")?;
		let error = Browser::launch(&options, &CancelToken::default())
			.err()
			.context("unmarked profile was accepted")?;
		assert!(error.to_string().contains("unmarked"));
		assert_eq!(fs.read(Path::new("user-data"), 32)?, b"keep");
		fs.remove(Path::new("user-data"))?;
		fs.atomic_write(Path::new(MARKER), OWNER)?;
		let _owner = fs.lock(Path::new(".harvest-browser.lock"))?;
		let error = Browser::launch(&options, &CancelToken::default())
			.err()
			.context("locked profile was accepted")?;
		assert!(error.to_string().contains("another writer"));
		Ok(())
	}

	#[test]
	fn correlated_responses_ignore_events_and_preserve_session_ownership() -> Result<()> {
		let (mut connection, peer) = cdp_peer(|mut socket| {
			let request: Value =
				serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
			assert_eq!(request["method"], "DOM.getDocument");
			assert_eq!(request["sessionId"], "owned");
			socket
				.send(Message::Text(
					json!({"method":"Page.event","sessionId":"owned"})
						.to_string()
						.into(),
				))
				.unwrap();
			socket
				.send(Message::Text(
					json!({"id":request["id"],"sessionId":"owned","result":{"root":{"nodeId":7}}})
						.to_string()
						.into(),
				))
				.unwrap();
		})?;
		let result =
			connection.call("DOM.getDocument", json!({}), Some("owned"), &CancelToken::default())?;
		assert_eq!(result["root"]["nodeId"], 7);
		assert!(!connection.poisoned);
		peer.join().unwrap();
		Ok(())
	}

	#[test]
	fn cancellation_poisoning_prevents_mutation_replay_and_wrong_sessions_fail() -> Result<()> {
		let (mut connection, peer) = cdp_peer(|mut socket| {
			socket.read().unwrap();
			thread::sleep(Duration::from_millis(150));
		})?;
		let result = connection.call(
			"Input.insertText",
			json!({"text":"once"}),
			Some("owned"),
			&CancelToken::with_timeout(Some(Duration::from_millis(30))),
		);
		assert!(result.is_err());
		assert!(connection.poisoned);
		assert!(
			connection
				.call("Input.insertText", json!({}), Some("owned"), &CancelToken::default())
				.is_err()
		);
		peer.join().unwrap();
		let (mut connection, peer) = cdp_peer(|mut socket| {
			let request: Value =
				serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
			socket
				.send(Message::Text(
					json!({"id":request["id"],"sessionId":"other","result":{}})
						.to_string()
						.into(),
				))
				.unwrap();
		})?;
		assert!(
			connection
				.call("DOM.getDocument", json!({}), Some("owned"), &CancelToken::default())
				.is_err()
		);
		assert!(connection.poisoned);
		peer.join().unwrap();
		Ok(())
	}

	#[test]
	fn malformed_closed_and_rejected_responses_cannot_be_successes() -> Result<()> {
		for response in ["not json", r#"{"id":1}"#, r#"{"id":1,"error":{"message":"private"}}"#] {
			let (mut connection, worker) = cdp_peer(move |mut socket| {
				socket.read().unwrap();
				socket.send(Message::Text(response.into())).unwrap();
			})?;
			let error = connection
				.call("Target.getTargets", json!({}), None, &CancelToken::default())
				.unwrap_err();
			assert!(!error.to_string().contains("private"));
			worker.join().unwrap();
		}
		let (mut connection, worker) = cdp_peer(|mut socket| {
			socket.read().unwrap();
			socket.close(None).unwrap();
		})?;
		assert!(
			connection
				.call("Target.getTargets", json!({}), None, &CancelToken::default())
				.is_err()
		);
		worker.join().unwrap();
		Ok(())
	}
}

#[cfg(test)]
mod navigation_tests {
	use super::*;

	#[test]
	fn load_event_arriving_before_navigation_response_is_not_lost() -> Result<()> {
		let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
		let address = listener.local_addr()?;
		let worker = thread::spawn(move || {
			let (stream, _) = listener.accept().unwrap();
			let mut socket = tungstenite::accept(stream).unwrap();
			let request: Value =
				serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
			for response in [
				json!({"method":"Page.lifecycleEvent","sessionId":"owned","params":{"name":"load","loaderId":"loader"}}),
				json!({"id":request["id"],"sessionId":"owned","result":{"loaderId":"loader"}}),
			] {
				socket
					.send(Message::Text(response.to_string().into()))
					.unwrap();
			}
		});
		let mut connection =
			Connection::connect(address, "/devtools/browser/test", &CancelToken::default())?;
		let result = connection.call(
			"Page.navigate",
			json!({"url":"about:blank"}),
			Some("owned"),
			&CancelToken::default(),
		)?;
		connection.wait_load(
			"owned",
			result["loaderId"].as_str().unwrap(),
			&CancelToken::default(),
		)?;
		assert!(!connection.poisoned);
		worker.join().unwrap();
		Ok(())
	}

	#[test]
	fn oversized_websocket_responses_are_rejected_at_the_transport_boundary() -> Result<()> {
		let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
		let address = listener.local_addr()?;
		let worker = thread::spawn(move || {
			let (stream, _) = listener.accept().unwrap();
			stream
				.set_write_timeout(Some(Duration::from_secs(2)))
				.unwrap();
			let mut socket = tungstenite::accept(stream).unwrap();
			socket.read().unwrap();
			let _ = socket.send(Message::Text("x".repeat(MAX_FRAME + 1).into()));
		});
		let mut connection =
			Connection::connect(address, "/devtools/browser/test", &CancelToken::default())?;
		assert!(
			connection
				.call("DOM.getDocument", json!({}), None, &CancelToken::default())
				.is_err()
		);
		assert!(connection.poisoned);
		worker.join().unwrap();
		Ok(())
	}
}
