//! Verify a copied native application with language runtimes absent from PATH.
use std::{
	io::{BufRead, BufReader, Read, Write},
	path::{Path, PathBuf},
	process::{Child, ChildStdin, ChildStdout, Command, Stdio},
	thread,
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

struct Controller {
	child:  Child,
	input:  Option<ChildStdin>,
	output: BufReader<ChildStdout>,
}
impl Controller {
	fn launch(application: &Path, chrome: &Path, root: &Path) -> Result<Self> {
		let mut command = Command::new(application);
		command
			.args(["browser", "--chrome"])
			.arg(chrome)
			.arg("--profile")
			.arg(root.join("profile"))
			.arg("--headless")
			.env("PATH", root.join("empty-path"))
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit());
		if std::env::var_os("HARVEST_TEST_NO_SANDBOX").is_some() {
			command.arg("--no-sandbox");
		}
		let mut child = command.spawn()?;
		let input = child.stdin.take().context("missing controller input")?;
		let output = BufReader::new(child.stdout.take().context("missing controller output")?);
		Ok(Self { child, input: Some(input), output })
	}

	fn call(&mut self, request: Value) -> Result<Value> {
		let input = self.input.as_mut().context("controller already closed")?;
		serde_json::to_writer(&mut *input, &request)?;
		input.write_all(b"\n")?;
		input.flush()?;
		let mut response = Vec::new();
		self
			.output
			.by_ref()
			.take(16 * 1024 * 1024 + 1)
			.read_until(b'\n', &mut response)?;
		if response.is_empty() || response.len() > 16 * 1024 * 1024 {
			bail!("missing/oversized native response");
		}
		Ok(serde_json::from_slice(&response)?)
	}

	fn finish(&mut self) -> Result<()> {
		self.input.take();
		let deadline = Instant::now() + Duration::from_secs(10);
		loop {
			if let Some(status) = self.child.try_wait()? {
				assert!(status.success());
				return Ok(());
			}
			if Instant::now() >= deadline {
				bail!("controller did not drain after EOF");
			}
			thread::sleep(Duration::from_millis(20));
		}
	}
}
impl Drop for Controller {
	fn drop(&mut self) {
		self.input.take();
		if self.finish().is_err() {
			let _ = self.child.kill();
			let _ = self.child.wait();
		}
	}
}
fn main() -> Result<()> {
	let mut arguments = std::env::args_os().skip(1);
	let application = PathBuf::from(
		arguments
			.next()
			.context("pass native application and native Chrome executable")?,
	);
	let chrome = PathBuf::from(
		arguments
			.next()
			.context("pass native Chrome executable (not a shell wrapper)")?,
	);
	let root = tempfile::tempdir()?;
	std::fs::create_dir(root.path().join("empty-path"))?;
	let mut controller = Controller::launch(&application, &chrome, root.path())?;
	assert_eq!(controller.call(json!({"action":"run","code":"not executable"}))?["ok"], false);
	assert_eq!(controller.call(json!({"action":"tabs","code":"not executable"}))?["ok"], false);
	let opened = controller.call(json!({"action":"open","url":"about:blank"}))?;
	assert_eq!(opened["ok"], true, "{opened}");
	let tab = opened["result"]["tab"].as_str().context("missing tab")?;
	assert_eq!(controller.call(json!({"action":"document","tab":tab}))?["ok"], true);
	assert_eq!(controller.call(json!({"action":"query","tab":tab,"selector":"html"}))?["ok"], true);
	let screenshot = controller.call(json!({"action":"screenshot","tab":tab}))?;
	assert!(
		screenshot["result"]["data"]
			.as_str()
			.context("missing PNG")?
			.starts_with("iVBORw0KGgo")
	);
	assert_eq!(controller.call(json!({"action":"close","tab":tab}))?["ok"], true);
	controller.finish()?;
	let mut reopened = Controller::launch(&application, &chrome, root.path())?;
	assert_eq!(reopened.call(json!({"action":"tabs"}))?["ok"], true);
	reopened.finish()?;
	println!(
		"copied native browser CLI, JSONL errors/actions, PNG, EOF/reopen, no Bun/Node/Python on \
		 PATH: passed"
	);
	Ok(())
}
