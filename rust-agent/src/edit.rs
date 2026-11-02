//! The `edit` tool: a `pi_edit` hashline session per call, sharing the
//! session-wide snapshot store with `read`. The engine stages every file in
//! memory and re-checks its read jail before the first write; the host writer
//! below persists bytes only through the workspace capability.
use std::{
	future::Future,
	path::{Path, PathBuf},
	pin::pin,
	sync::Arc,
	task::{Context as TaskContext, Poll, Wake, Waker},
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use pi_edit::{
	EditError, EditMode, EditResult, EditStore, FileOp, PathPolicy, Session,
	session::{ApplyRequest, EditWriter, SessionConfig, WriteRequest, WriteResponse},
};

/// Matches the legacy `edit.fuzzyMatch` / `edit.fuzzyThreshold` defaults.
const FUZZY_THRESHOLD: f64 = 0.95;

/// The policy every edit runs under: reads and writes stay inside `root`.
pub(crate) fn policy(root: &Path) -> PathPolicy {
	PathPolicy {
		cwd:                  root.to_owned(),
		home_dir:             root.to_owned(),
		local_sandbox_root:   None,
		vault_roots:          None,
		plan_active:          false,
		block_auto_generated: true,
	}
}

struct JailedWriter<'a> {
	fs:           JailedFs,
	cancellation: &'a CancelToken,
}

fn writer_error(error: impl std::fmt::Display) -> EditError {
	EditError::Writer(error.to_string())
}

impl JailedWriter<'_> {
	fn persist(&self, request: &WriteRequest) -> Result<String> {
		self.cancellation.heartbeat()?;
		if request.op == FileOp::Delete {
			self.fs.remove(&request.absolute)?;
			return Ok(String::new());
		}
		let Some(content) = &request.content else {
			bail!("Native edit {} request omitted content", request.op.as_str());
		};
		if content.len() > MAX_FILE_BYTES {
			bail!("edit exceeds file limit");
		}
		match &request.move_to {
			Some(destination) => {
				self.fs.relative(destination)?;
				self.fs.atomic_write(destination, content.as_bytes())?;
				self.fs.remove(&request.absolute)?;
			},
			None => self
				.fs
				.atomic_write(&request.absolute, content.as_bytes())?,
		}
		Ok(content.clone())
	}
}

#[async_trait]
impl EditWriter for JailedWriter<'_> {
	async fn write(&self, request: WriteRequest) -> EditResult<WriteResponse> {
		let written = self.persist(&request).map_err(writer_error)?;
		Ok(WriteResponse { written, diagnostics_json: None })
	}
}

/// Drive a future whose awaits all complete synchronously (the jailed writer
/// never yields), without pulling an async runtime into the core build.
fn block_on<F: Future>(future: F) -> F::Output {
	struct Unpark(std::thread::Thread);
	impl Wake for Unpark {
		fn wake(self: Arc<Self>) {
			self.0.unpark();
		}
	}
	let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
	let mut context = TaskContext::from_waker(&waker);
	let mut future = pin!(future);
	loop {
		match future.as_mut().poll(&mut context) {
			Poll::Ready(output) => return output,
			Poll::Pending => std::thread::park(),
		}
	}
}

/// Apply one hashline edit call (`{"input": "..."}`) and return the
/// model-facing result text.
pub(crate) fn hashline(
	root: &Path,
	arguments: &str,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<String> {
	cancellation.heartbeat()?;
	let fs = JailedFs::open(root)?;
	let root: PathBuf = fs.root().to_owned();
	let mut session = Session::new(
		SessionConfig {
			mode:               EditMode::Hashline,
			policy:             policy(&root),
			allow_fuzzy:        true,
			fuzzy_threshold:    FUZZY_THRESHOLD,
			enforce_seen_lines: false,
			raw_input:          false,
		},
		store.clone(),
	);
	session.set_args_json(arguments);
	session.finish();
	let writer = JailedWriter { fs, cancellation };
	let outcome = block_on(session.apply(ApplyRequest::default(), &writer))?;
	Ok(outcome.text)
}
