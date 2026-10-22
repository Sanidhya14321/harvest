//! Real-browser workflow verification; requires an explicitly supplied Chrome.
//! Static HTML only: there is no injected or fixture JavaScript.
use std::{
	io::{Read, Write},
	net::{Ipv4Addr, TcpListener},
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	thread,
	time::{Duration, Instant},
};

use anyhow::{Context, Result};
use harvest_browser::{Action, Browser, Options};
use harvest_runtime::cancel::CancelToken;

fn main() -> Result<()> {
	let chrome = std::env::args_os()
		.nth(1)
		.context("pass the Chrome/Chromium executable")?;
	let profile = tempfile::tempdir()?;
	let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
	let url = format!("http://{}/", listener.local_addr()?);
	listener.set_nonblocking(true)?;
	let requests = Arc::new(Mutex::new(Vec::new()));
	let observed = Arc::clone(&requests);
	let stop = Arc::new(AtomicBool::new(false));
	let stopped = Arc::clone(&stop);
	let server = thread::spawn(move || -> Result<()> {
		while !stopped.load(Ordering::Acquire) {
			let (mut socket, _) = match listener.accept() {
				Ok(value) => value,
				Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
					thread::sleep(Duration::from_millis(10));
					continue;
				},
				Err(error) => return Err(error.into()),
			};
			socket.set_nonblocking(false)?;
			socket.set_read_timeout(Some(Duration::from_secs(2)))?;
			let mut request = Vec::new();
			let mut bytes = [0; 4096];
			while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 16 * 1024 {
				let count = match socket.read(&mut bytes) {
					Ok(0) | Err(_) => break,
					Ok(count) => count,
				};
				request.extend_from_slice(&bytes[..count]);
			}
			if request.is_empty() {
				continue;
			}
			let request = String::from_utf8(request)?;
			let submitted = request.starts_with("GET /submit?");
			observed.lock().unwrap().push(request);
			let body = if submitted {
				"<!doctype html><h1 id=result>Submitted Native Rust</h1>"
			} else {
				"<!doctype html><title>Native workflow</title><h1 id=title>Native browser</h1><form \
				 action=/submit><input id=answer name=answer><button id=send>Send</button></form>"
			};
			let response = format!(
				"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nSet-Cookie: \
				 persist=rust; Max-Age=3600; Path=/; SameSite=Lax\r\nConnection: close\r\n\r\n{body}",
				body.len()
			);
			let _ = socket.write_all(response.as_bytes());
		}
		Ok(())
	});
	let options = Options {
		chrome:     chrome.into(),
		profile:    profile.path().join("dedicated"),
		headless:   true,
		no_sandbox: std::env::var_os("HARVEST_TEST_NO_SANDBOX").is_some(),
	};
	let cancel = CancelToken::with_timeout(Some(Duration::from_secs(30)));
	let mut browser = Browser::launch(&options, &cancel)?;
	assert!(Browser::launch(&options, &cancel).is_err(), "profile owner lock was not exclusive");
	let opened = browser.perform(Action::Open { url: url.clone() }, &cancel)?;
	let tab = opened["tab"].as_str().context("missing tab")?.to_owned();
	let found =
		browser.perform(Action::Query { tab: tab.clone(), selector: "#title".into() }, &cancel)?;
	assert_eq!(found["node"]["localName"], "h1");
	browser.perform(
		Action::Type {
			tab:      tab.clone(),
			selector: "#answer".into(),
			text:     "Native Rust".into(),
		},
		&cancel,
	)?;
	browser.perform(Action::Click { tab: tab.clone(), selector: "#send".into() }, &cancel)?;
	let until = Instant::now() + Duration::from_secs(5);
	loop {
		if requests
			.lock()
			.unwrap()
			.iter()
			.any(|r| r.starts_with("GET /submit?answer=Native+Rust "))
		{
			break;
		}
		assert!(Instant::now() < until, "native input/click did not submit the form");
		thread::sleep(Duration::from_millis(20));
	}
	// Receiving a request precedes the browser committing its response. Observe
	// the submitted document through CDP before taking its screenshot.
	loop {
		if browser
			.perform(Action::Query { tab: tab.clone(), selector: "#result".into() }, &cancel)
			.is_ok()
		{
			break;
		}
		assert!(Instant::now() < until, "submitted DOM did not arrive");
		thread::sleep(Duration::from_millis(20));
	}
	let capture = browser.perform(Action::Screenshot { tab: tab.clone() }, &cancel)?;
	assert!(capture["data"].as_str().unwrap().starts_with("iVBORw0KGgo"), "capture is not PNG");
	browser.perform(Action::Close { tab }, &cancel)?;
	drop(browser);
	let before = requests.lock().unwrap().len();
	let mut browser = Browser::launch(&options, &CancelToken::default())?;
	browser.perform(Action::Open { url }, &CancelToken::default())?;
	assert!(
		requests.lock().unwrap()[before..]
			.iter()
			.any(|r| r.contains("Cookie: persist=rust")),
		"persistent authentication cookie did not survive restart"
	);
	drop(browser);
	stop.store(true, Ordering::Release);
	server.join().unwrap()?;
	println!(
		"native Chrome navigation, DOM, input/click, PNG, ownership and persistent profile: passed"
	);
	Ok(())
}
