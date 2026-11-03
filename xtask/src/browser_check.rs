//! Production replay checks run through Chrome's bounded loopback CDP
//! transport.
use std::{
	env, fs,
	path::Path,
	process::{Child, Command, Stdio},
	thread,
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use harvest_protocol::display::ReplayProjection;
use harvest_runtime::{JailedFs, read_bounded};
use serde_json::{Value, json};
use tempfile::TempDir;
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

struct Browser {
	child:   Child,
	profile: TempDir,
}
impl Drop for Browser {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

struct Connection {
	socket:           WebSocket<MaybeTlsStream<std::net::TcpStream>>,
	sequence:         u64,
	external_fetches: Vec<String>,
}

impl Connection {
	fn request(&mut self, method: &str, params: Value) -> Result<Value> {
		self.sequence += 1;
		let id = self.sequence;
		self.socket.send(Message::Text(
			json!({"id":id,"method":method,"params":params})
				.to_string()
				.into(),
		))?;
		let deadline = Instant::now() + Duration::from_secs(15);
		let mut events = 0;
		loop {
			if Instant::now() > deadline || events > 10_000 {
				bail!("browser command exceeded its deadline/event bound: {method}");
			}
			events += 1;
			let message = self.socket.read()?;
			let Message::Text(text) = message else {
				continue;
			};
			if text.len() > 2 * 1024 * 1024 {
				bail!("browser response exceeds limit");
			}
			let mut response: Value = serde_json::from_str(&text)?;
			if response["method"] == "Network.requestWillBeSent"
				&& let Some(url) = response["params"]["request"]["url"].as_str()
				&& matches!(
					reqwest::Url::parse(url)
						.map(|url| url.scheme().to_owned())
						.as_deref(),
					Ok("http" | "https")
				) {
				if self.external_fetches.len() > 100 {
					bail!("unexpected browser network request flood");
				}
				self.external_fetches.push(url.to_owned());
			}
			if response["id"].as_u64() == Some(id) {
				if !response["error"].is_null() {
					bail!("browser rejected command {method}");
				}
				return Ok(response["result"].take());
			}
		}
	}

	fn evaluate(&mut self, expression: &str) -> Result<Value> {
		let value = self.request(
			"Runtime.evaluate",
			json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
		)?;
		if !value["exceptionDetails"].is_null() {
			bail!("browser evaluation failed");
		}
		Ok(value["result"]["value"].clone())
	}
}

pub fn verify(fs: &JailedFs, html: &Path, projection: &ReplayProjection) -> Result<()> {
	let profile = TempDir::new()?;
	let mut profile_argument = std::ffi::OsString::from("--user-data-dir=");
	profile_argument.push(profile.path());
	let child =
		Command::new(env::var_os("HARVEST_REPLAY_BROWSER").unwrap_or_else(|| "google-chrome".into()))
			.args([
				"--headless=new",
				"--no-sandbox",
				"--disable-gpu",
				"--disable-background-networking",
				"--no-first-run",
				"--remote-debugging-address=127.0.0.1",
				"--remote-debugging-port=0",
			])
			.arg(profile_argument)
			.arg("about:blank")
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.context(
				"headless Chrome is required for --check-browser (or set HARVEST_REPLAY_BROWSER)",
			)?;
	let mut browser = Browser { child, profile };
	let deadline = Instant::now() + Duration::from_secs(15);
	let port = loop {
		if let Ok(data) = fs::read_to_string(browser.profile.path().join("DevToolsActivePort")) {
			break data
				.lines()
				.next()
				.context("missing browser port")?
				.parse::<u16>()?;
		}
		if Instant::now() > deadline || browser.child.try_wait()?.is_some() {
			bail!("headless browser failed to start");
		}
		thread::sleep(Duration::from_millis(25));
	};
	let client = reqwest::blocking::Client::builder()
		.no_proxy()
		.timeout(Duration::from_secs(15))
		.build()?;
	let response = client
		.put(format!("http://127.0.0.1:{port}/json/new?about:blank"))
		.send()?
		.error_for_status()?;
	let target: Value = serde_json::from_slice(&read_bounded(response, 64 * 1024)?)?;
	let websocket = target["webSocketDebuggerUrl"]
		.as_str()
		.context("missing browser socket")?;
	let socket_url = reqwest::Url::parse(websocket)?;
	if socket_url.scheme() != "ws"
		|| socket_url.host_str() != Some("127.0.0.1")
		|| socket_url.port() != Some(port)
	{
		bail!("browser returned a non-loopback debugger endpoint");
	}
	let config = tungstenite::protocol::WebSocketConfig::default()
		.max_message_size(Some(2 * 1024 * 1024))
		.max_frame_size(Some(2 * 1024 * 1024));
	let (mut socket, _) = tungstenite::client::connect_with_config(websocket, Some(config), 3)?;
	if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
		stream.set_read_timeout(Some(Duration::from_secs(15)))?;
		stream.set_write_timeout(Some(Duration::from_secs(15)))?;
	}
	let mut connection = Connection { socket, sequence: 0, external_fetches: Vec::new() };
	connection.request("Page.enable", json!({}))?;
	connection.request("Network.enable", json!({}))?;
	let url = reqwest::Url::from_file_path(fs.resource_path(html)?)
		.map_err(|()| anyhow::anyhow!("invalid replay file URL"))?;
	connection.request("Page.navigate", json!({"url":url.as_str()}))?;
	let deadline = Instant::now() + Duration::from_secs(15);
	loop {
		if connection.evaluate("!!document.querySelector('main')")? == true {
			break;
		}
		if Instant::now() > deadline {
			bail!("production WASM viewer did not mount");
		}
		thread::sleep(Duration::from_millis(25));
	}
	let state = connection.evaluate(
		"({ids:[...document.querySelectorAll('article.entry')].map(e=>e.id),handlers:document.\
		 querySelectorAll('[onerror],[onload]').length,scripts:document.querySelectorAll('script').\
		 length,injected:globalThis.harvestInjected===1})",
	)?;
	let expected: Vec<_> = projection
		.entries
		.iter()
		.filter(|entry| entry.is_visible())
		.map(|entry| format!("entry-{}", entry.id))
		.collect();
	if state["ids"] != json!(expected)
		|| state["handlers"] != 0
		|| state["scripts"] != 2
		|| state["injected"] != false
	{
		bail!(
			"browser history visibility, ordering or snapshot escaping differs from the Rust \
			 projection"
		);
	}
	connection.evaluate(
		"document.querySelector('input').value='42';document.querySelector('input').\
		 dispatchEvent(new Event('input',{'bubbles':true}));",
	)?;
	let expected: Vec<_> = projection
		.entries
		.iter()
		.filter(|entry| entry.is_visible() && harvest_web::searchable_text(entry).contains("42"))
		.map(|entry| format!("entry-{}", entry.id))
		.collect();
	let deadline = Instant::now() + Duration::from_secs(2);
	loop {
		let actual = connection.evaluate(
			"[...document.querySelectorAll('article.entry')].filter(e=>!e.hidden).map(e=>e.id)",
		)?;
		if actual == json!(expected) {
			break;
		}
		if Instant::now() > deadline {
			bail!("browser search did not apply the Rust filter");
		}
		thread::sleep(Duration::from_millis(25));
	}
	if !connection.external_fetches.is_empty() {
		bail!("self-contained export fetched external resources");
	}
	println!(
		"Browser contract passed: selected branch/order, safe snapshot, Rust search and zero \
		 external fetches"
	);
	Ok(())
}
