//! Credential commands reuse the normal native shell/cancellation owner. Secret
//! stdout is bounded and never mixed with stderr, tool results or session logs.
use std::path::Path;

use anyhow::Result;
use harvest_runtime::cancel::CancelToken;

#[cfg(not(feature = "full"))]
pub(super) fn execute(
	_root: &Path,
	_command: &str,
	_cancellation: &CancelToken,
) -> Result<Option<String>> {
	anyhow::bail!("credential commands require the full native shell build")
}
#[cfg(feature = "full")]
pub(super) fn execute(
	root: &Path,
	command: &str,
	cancellation: &CancelToken,
) -> Result<Option<String>> {
	// A separate scoped thread permits synchronous SDK calls from inside an
	// existing Tokio runtime without nesting block_on on the caller's thread.
	std::thread::scope(|scope| scope.spawn(|| run(root, command, cancellation)).join())
		.map_err(|_| anyhow::anyhow!("credential command worker failed"))?
}
#[cfg(feature = "full")]
fn run(root: &Path, command: &str, parent: &CancelToken) -> Result<Option<String>> {
	use harvest_runtime::cancel::AbortReason;
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|_| anyhow::anyhow!("credential command runtime is unavailable"))?;
	let result = runtime.block_on(async {
		parent.heartbeat()?;
		let mut token = CancelToken::new(Some(10_000));
		let abort = token.emplace_abort_token();
		let output_abort = abort.clone();
		let caller = parent.clone();
		let bridge = tokio::spawn(async move {
			abort.abort(caller.wait().await);
		});
		let (sender, receiver) = flume::bounded(8);
		let streams = pi_shell::StreamSinks { stdout: Some(sender), stderr: None };
		let drain = tokio::spawn(async move {
			let mut bytes = Vec::new();
			let mut overflow = false;
			while let Ok(chunk) = receiver.recv_async().await {
				let chunk: &[u8] = chunk.as_ref();
				if chunk.len() > (64_usize * 1024).saturating_sub(bytes.len()) {
					overflow = true;
					output_abort.abort(AbortReason::User);
				} else if !overflow {
					bytes.extend_from_slice(chunk);
				}
			}
			(bytes, overflow)
		});
		let execution = pi_shell::execute_shell_streams(
			pi_shell::ShellExecuteOptions {
				command: command.into(),
				cwd: Some(root.to_string_lossy().into_owned()),
				timeout_ms: Some(10_000),
				..Default::default()
			},
			streams,
			token,
		)
		.await;
		bridge.abort();
		let (bytes, overflow) = drain
			.await
			.map_err(|_| anyhow::anyhow!("credential output owner failed"))?;
		parent.heartbeat()?;
		let Ok(execution) = execution else {
			return Ok(None);
		};
		if overflow || execution.cancelled || execution.timed_out || execution.exit_code != Some(0) {
			return Ok(None);
		}
		Ok(String::from_utf8(bytes)
			.ok()
			.map(|v| v.trim().to_owned())
			.filter(|v| !v.is_empty()))
	});
	runtime.shutdown_timeout(std::time::Duration::from_secs(1));
	result
}
