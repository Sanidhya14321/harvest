//! Native publication uses existing jailed atomic I/O and resource locks. All
//! targets are preflighted; commits remain sequential, with explicit progress.
use std::{
	collections::{BTreeSet, VecDeque},
	path::PathBuf,
	sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use harvest_runtime::{
	JailedFs, MAX_FILE_BYTES, cancel::CancelToken, content_hash, file_lock::FileLock,
};
use pi_edit::{EditError, EditResult, EditWriter, FileOp, StagedFile, WriteRequest, WriteResponse};
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct CommittedChange {
	pub path:      PathBuf,
	pub operation: &'static str,
}
#[derive(Debug)]
pub struct NativeEditFailure {
	pub committed:            Vec<CommittedChange>,
	pub progress_unavailable: bool,
	source:                   EditError,
}
impl std::fmt::Display for NativeEditFailure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if self.progress_unavailable {
			write!(
				f,
				"native edit progress is unavailable; inspect all requested files: {}",
				self.source
			)
		} else if self.committed.is_empty() {
			std::fmt::Display::fmt(&self.source, f)
		} else {
			write!(
				f,
				"native edit stopped after {} committed filesystem changes; re-read affected files: {}",
				self.committed.len(),
				self.source
			)
		}
	}
}
impl std::error::Error for NativeEditFailure {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		Some(&self.source)
	}
}
struct Planned {
	absolute:             PathBuf,
	source:               PathBuf,
	destination:          Option<PathBuf>,
	authored_destination: Option<PathBuf>,
	op:                   FileOp,
	before:               Option<String>,
	content:              Option<String>,
}
#[derive(Default)]
struct State {
	pending:   VecDeque<Planned>,
	committed: Vec<CommittedChange>,
	locks:     Vec<FileLock>,
}
pub(super) struct NativeWriter {
	fs:           Arc<JailedFs>,
	cancellation: CancelToken,
	state:        Mutex<State>,
}
impl NativeWriter {
	pub fn new(fs: Arc<JailedFs>, cancellation: CancelToken) -> Self {
		Self { fs, cancellation, state: Mutex::new(State::default()) }
	}

	pub fn failure(&self, source: EditError) -> NativeEditFailure {
		let (committed, progress_unavailable) = match self.state.lock() {
			Ok(state) => (state.committed.clone(), false),
			Err(poisoned) => (poisoned.into_inner().committed.clone(), true),
		};
		NativeEditFailure { committed, progress_unavailable, source }
	}

	fn fingerprint(&self, path: &std::path::Path) -> Result<Option<String>> {
		self.cancellation.heartbeat()?;
		Ok(self
			.fs
			.read_optional(path, MAX_FILE_BYTES)?
			.map(|bytes| content_hash(&bytes)))
	}

	pub fn prepare(&self, staged: &[StagedFile]) -> Result<Vec<serde_json::Value>> {
		ensure!(!staged.is_empty() && staged.len() <= 32, "native edit must stage 1..32 files");
		let mut paths = BTreeSet::new();
		let mut plans = VecDeque::new();
		let mut details = Vec::new();
		let mut bytes = 0usize;
		for file in staged {
			self.cancellation.heartbeat()?;
			let source = self.fs.resource_path(&file.absolute)?;
			// Retain the resolved target for reads, locks and mutation. Never
			// validate one path then reopen its unvalidated authored alias.
			ensure!(paths.insert(source.clone()), "overlapping or aliased edit targets");
			let destination = file
				.move_to
				.as_ref()
				.map(|m| self.fs.resource_path(&m.absolute))
				.transpose()?;
			if let Some(destination) = &destination {
				ensure!(paths.insert(destination.clone()), "overlapping move destination");
			}
			bytes = bytes
				.checked_add(file.before_raw.as_ref().map_or(0, String::len))
				.and_then(|n| n.checked_add(file.persisted.as_ref().map_or(0, String::len)))
				.context("edit byte budget overflow")?;
			ensure!(
				bytes <= 16 * 1024 * 1024
					&& file
						.persisted
						.as_ref()
						.is_none_or(|s| s.len() <= MAX_FILE_BYTES),
				"native edit exceeds byte budget"
			);
			let noop = file.op == FileOp::Noop;
			let diff = (!noop && !file.diff.is_empty() && file.diff.len() <= 32 * 1024)
				.then_some(file.diff.as_str());
			details.push(serde_json::json!({"op":file.op.as_str(),"path":file.display,"noop":noop,"diff":diff,"diffUnavailable":if noop{Some("no_change")}else if file.diff.is_empty(){Some("not_reported")}else if diff.is_none(){Some("exceeds_limit")}else{None},"moveTo":file.move_to.as_ref().map(|m|m.display.as_str())}));
			plans.push_back(Planned {
				absolute: file.absolute.clone(),
				source,
				destination,
				authored_destination: file.move_to.as_ref().map(|m| m.absolute.clone()),
				op: file.op,
				before: file.before_raw.as_ref().map(|s| content_hash(s.as_bytes())),
				content: file.persisted.clone(),
			});
		}
		let mut output = harvest_runtime::BoundedWriter::new(crate::tools::MAX_TOOL_DETAILS_BYTES);
		serde_json::to_writer(&mut output, &details)?;
		let mut locks = Vec::new();
		for path in paths {
			self.cancellation.heartbeat()?;
			locks.push(self.fs.resource_lock(&path)?);
		}
		// Recheck every preimage/destination after ownership, before any write.
		for plan in &plans {
			ensure!(
				self.fingerprint(&plan.source)? == plan.before,
				"edit target changed after staging"
			);
			if let Some(destination) = &plan.destination {
				ensure!(self.fingerprint(destination)?.is_none(), "move destination already exists");
			}
		}
		let mut state = self
			.state
			.lock()
			.map_err(|_| anyhow::anyhow!("edit writer state unavailable"))?;
		ensure!(state.pending.is_empty() && state.locks.is_empty(), "edit writer already prepared");
		state.pending = plans;
		state.locks = locks;
		// Noop stages have no writer callback. Remove them only after their
		// preimages were validated and their resource ownership was retained.
		state.pending.retain(|p| p.op != FileOp::Noop);
		Ok(details)
	}

	fn commit(&self, request: WriteRequest) -> Result<WriteResponse> {
		self.cancellation.heartbeat()?;
		let mut state = self
			.state
			.lock()
			.map_err(|_| anyhow::anyhow!("edit writer state unavailable"))?;
		let plan = state.pending.pop_front().context("unplanned edit write")?;
		ensure!(
			request.absolute == plan.absolute
				&& request.move_to == plan.authored_destination
				&& request.op == plan.op
				&& request.content == plan.content,
			"writer request differs from approved staging"
		);
		ensure!(
			self.fs.resource_path(&request.absolute)? == plan.source,
			"edit target resolution changed"
		);
		ensure!(self.fingerprint(&plan.source)? == plan.before, "edit target changed before write");
		if let Some(destination) = &plan.destination {
			ensure!(
				self.fs.resource_path(
					request
						.move_to
						.as_deref()
						.context("missing move destination")?
				)? == *destination,
				"move destination resolution changed"
			);
			ensure!(self.fingerprint(destination)?.is_none(), "move destination changed before write");
		}
		self.cancellation.heartbeat()?;
		if plan.op == FileOp::Delete {
			self.fs.remove(&plan.source)?;
			state
				.committed
				.push(CommittedChange { path: plan.source, operation: "delete" });
			return Ok(WriteResponse::default());
		}
		let content = plan.content.context("write content missing")?;
		if let Some(destination) = plan.destination {
			self.fs.atomic_create(&destination, content.as_bytes())?;
			state
				.committed
				.push(CommittedChange { path: destination, operation: "move_destination" });
			// Do not cancel between a landed destination and source cleanup.
			// Cleanup failure remains an explicit partial move, never rollback.
			self.fs.remove(&plan.source)?;
			state
				.committed
				.push(CommittedChange { path: plan.source, operation: "move_source_removed" });
		} else {
			if plan.op == FileOp::Create {
				self.fs.atomic_create(&plan.source, content.as_bytes())?;
			} else {
				self.fs.atomic_write(&plan.source, content.as_bytes())?;
			}
			state
				.committed
				.push(CommittedChange { path: plan.source, operation: "write" });
		}
		Ok(WriteResponse { written: content, diagnostics_json: None })
	}
}
#[async_trait::async_trait]
impl EditWriter for NativeWriter {
	async fn write(&self, request: WriteRequest) -> EditResult<WriteResponse> {
		self.commit(request).map_err(super::edit_error)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn later_preimage_change_preserves_and_reports_the_committed_prefix() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = Arc::new(JailedFs::open(root.path())?);
		let mut staged = Vec::new();
		for name in ["first.txt", "second.txt"] {
			fs.atomic_create(std::path::Path::new(name), b"old\n")?;
			let mut file = StagedFile::new(name, fs.root().join(name), FileOp::Update);
			file.before_raw = Some("old\n".into());
			file.persisted = Some("new\n".into());
			staged.push(file);
		}
		let writer = NativeWriter::new(fs.clone(), CancelToken::default());
		writer.prepare(&staged)?;
		let request = |file: &StagedFile| WriteRequest {
			absolute:     file.absolute.clone(),
			display:      file.display.clone(),
			op:           file.op,
			move_to:      None,
			content:      file.persisted.clone(),
			flush_lsp:    false,
			lsp_batch_id: None,
		};
		writer.commit(request(&staged[0]))?;
		// Simulate a non-cooperating external writer after the all-file preflight.
		fs.atomic_write(std::path::Path::new("second.txt"), b"external\n")?;
		let error = writer.commit(request(&staged[1])).unwrap_err();
		let error = writer.failure(super::super::edit_error(error));
		assert_eq!(error.committed.len(), 1);
		assert_eq!(error.committed[0].path, fs.root().join("first.txt"));
		assert_eq!(fs.read(std::path::Path::new("first.txt"), 32)?, b"new\n");
		assert_eq!(fs.read(std::path::Path::new("second.txt"), 32)?, b"external\n");
		Ok(())
	}
}
