//! Bounded output for short local tooling probes.
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

pub(crate) fn capture(mut command: Command, limit: usize) -> Result<Vec<u8>> {
	let mut child = command
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()?;
	let output =
		harvest_runtime::read_bounded(child.stdout.take().context("missing process stdout")?, limit);
	let bytes = match output {
		Ok(bytes) => bytes,
		Err(error) => {
			let _ = child.kill();
			let _ = child.wait();
			return Err(error);
		},
	};
	let status = child.wait()?;
	if !status.success() {
		bail!("tooling probe failed with {status}");
	}
	Ok(bytes)
}
