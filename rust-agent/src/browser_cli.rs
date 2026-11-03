use std::{
	io::{self, BufRead, Read, Write},
	path::PathBuf,
	sync::mpsc::{self, RecvTimeoutError},
	thread,
	time::Duration,
};

use anyhow::{Context, Result, bail};
use harvest_browser::{Action, Browser, Options};
use harvest_runtime::cancel::{AbortReason, CancelToken};
use serde_json::json;

#[derive(Debug, clap::Args)]
pub(crate) struct BrowserArgs {
	#[arg(long, help = "Chrome/Chromium executable path")]
	chrome:     PathBuf,
	#[arg(long, help = "Dedicated persistent profile; must be empty or already marked by Harvest")]
	profile:    PathBuf,
	#[arg(long)]
	headless:   bool,
	#[arg(long, help = "Explicit Chrome sandbox opt-out for constrained test containers")]
	no_sandbox: bool,
}

pub(crate) fn execute(args: BrowserArgs) -> Result<u8> {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.worker_threads(1)
		.build()?;
	let mut cancel = CancelToken::default();
	let interrupt = cancel.emplace_abort_token();
	runtime.spawn(async move {
		if tokio::signal::ctrl_c().await.is_ok() {
			interrupt.abort(AbortReason::User);
		}
	});
	let options = Options {
		chrome:     args.chrome,
		profile:    args.profile,
		headless:   args.headless,
		no_sandbox: args.no_sandbox,
	};
	let mut browser = Browser::launch(&options, &cancel)?;
	// A bounded reader lets the single browser owner react to interrupts even
	// while stdin is idle. It cannot enqueue unbounded commands or share CDP.
	let (send, receive) = mpsc::sync_channel(1);
	thread::spawn(move || {
		let mut input = io::stdin().lock();
		loop {
			let mut line = Vec::new();
			let result = input
				.by_ref()
				.take(128 * 1024 + 1)
				.read_until(b'\n', &mut line);
			let done = result.is_err() || line.is_empty() || line.len() > 128 * 1024;
			if send.send(result.map(|_| line)).is_err() || done {
				break;
			}
		}
	});
	let mut output = io::stdout().lock();
	loop {
		cancel.heartbeat()?;
		let line = match receive.recv_timeout(Duration::from_millis(100)) {
			Ok(line) => line?,
			Err(RecvTimeoutError::Timeout) => continue,
			Err(RecvTimeoutError::Disconnected) => break,
		};
		if line.is_empty() {
			break;
		}
		if line.len() > 128 * 1024 {
			bail!("browser JSONL request exceeds 128 KiB");
		}
		let response = match serde_json::from_slice::<Action>(&line)
			.context("invalid browser action; use native structured actions, not JavaScript run/call")
		{
			Ok(action) => match browser.perform(action, &cancel) {
				Ok(result) => json!({"ok":true,"result":result}),
				Err(error) => json!({"ok":false,"error":error.to_string()}),
			},
			Err(error) => json!({"ok":false,"error":error.to_string()}),
		};
		serde_json::to_writer(&mut output, &response)?;
		output.write_all(b"\n")?;
		output.flush()?;
	}
	Ok(0)
}
