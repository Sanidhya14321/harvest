//! Exercise the public Rust SDK tool owner, including approved capture writes.
use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context, Result};
use harvest_omp::{
	browser::Options,
	chat::{FunctionCall, ToolCall},
	tools::{Approval, ToolRuntime},
};
use serde_json::{Value, json};

struct Host {
	approved: Mutex<Vec<ToolCall>>,
}
impl Approval for Host {
	fn approve(&self, call: &ToolCall) -> Result<bool> {
		self.approved.lock().unwrap().push(call.clone());
		Ok(true)
	}
}
fn main() -> Result<()> {
	let chrome = PathBuf::from(
		std::env::args_os()
			.nth(1)
			.context("pass native Chrome executable")?,
	);
	let root = tempfile::tempdir()?;
	let workspace = root.path().join("workspace");
	std::fs::create_dir(&workspace)?;
	let profile = root.path().join("profile");
	let mut tools = ToolRuntime::default();
	tools.configure_browser(Options {
		chrome,
		profile: profile.clone(),
		headless: true,
		no_sandbox: std::env::var_os("HARVEST_TEST_NO_SANDBOX").is_some(),
	});
	let host = Host { approved: Mutex::new(Vec::new()) };
	let mut expected = Vec::new();
	let mut call = |request: Value| -> Result<Value> {
		let tool_call = ToolCall {
			id:       expected.len().to_string(),
			kind:     "function".into(),
			function: FunctionCall { name: "browser".into(), arguments: request.to_string() },
		};
		expected.push(tool_call.clone());
		let output = tools.invoke(&workspace, &tool_call, &host, &Default::default())?;
		Ok(serde_json::from_str(&output)?)
	};
	let opened = call(json!({"action":"open","url":"about:blank"}))?;
	let tab = opened["tab"].as_str().context("missing tab")?;
	let found = call(json!({"action":"query","tab":tab,"selector":"html"}))?;
	assert_eq!(found["node"]["localName"], "html");
	let captured = call(json!({"action":"screenshot","tab":tab}))?;
	let path = captured["file"]
		.as_str()
		.context("missing screenshot path")?;
	assert!(path.starts_with(".harvest/browser-captures/"));
	assert_eq!(&std::fs::read(workspace.join(path))?[..8], b"\x89PNG\r\n\x1a\n");
	call(json!({"action":"close","tab":tab}))?;
	assert_eq!(*host.approved.lock().unwrap(), expected);
	drop(tools);
	assert!(profile.join("harvest-browser-profile.json").is_file());
	println!(
		"public Rust SDK native browser owner, final-argument approvals and jailed PNG capture: \
		 passed"
	);
	Ok(())
}
