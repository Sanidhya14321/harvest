//! Native edit staging uses pi-edit; the application owns approvals and bytes.
use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use pi_edit::{
	ArgSnapshot, EditError, EditResult, EditStore, ModeEngine, PathPolicy, Resolved,
	files::{FileRead, FileSource},
	modes::replace::ReplaceEngine,
};

use crate::tools::EditArgs;

struct WorkspaceFiles<'a> {
	fs:           JailedFs,
	policy:       PathPolicy,
	cancellation: &'a CancelToken,
}

impl WorkspaceFiles<'_> {
	fn load(&self, resolved: &Resolved) -> Result<Option<Arc<FileRead>>> {
		self.cancellation.heartbeat()?;
		let Some(bytes) = self.fs.read_optional(&resolved.absolute, MAX_FILE_BYTES)? else {
			return Ok(None);
		};
		if bytes.contains(&0) {
			bail!("edit target is not a text file");
		}
		if let Some(message) = self
			.policy
			.auto_generated_message(&resolved.display, &bytes[..bytes.len().min(1024)])
		{
			bail!("{message}");
		}
		let raw = String::from_utf8(bytes).context("edit target is not UTF-8")?;
		Ok(Some(Arc::new(FileRead::from_raw(resolved.clone(), raw)?)))
	}
}

fn edit_error(error: anyhow::Error) -> EditError {
	EditError::apply(format!("{error:#}"))
}

impl FileSource for WorkspaceFiles<'_> {
	fn policy(&self) -> &PathPolicy {
		&self.policy
	}

	fn resolve(&mut self, authored: &str, _must_exist: bool) -> EditResult<Resolved> {
		let resolved = self.policy.resolve(authored)?;
		self.fs.relative(&resolved.absolute).map_err(edit_error)?;
		// No suffix recovery: approval covered this exact authored target.
		Ok(resolved)
	}

	fn exists(&mut self, absolute: &Path) -> bool {
		self
			.fs
			.read_optional(absolute, MAX_FILE_BYTES)
			.is_ok_and(|read| read.is_some())
	}

	fn read(&mut self, authored: &str) -> EditResult<Arc<FileRead>> {
		let resolved = self.resolve(authored, true)?;
		self
			.try_read(&resolved)?
			.ok_or_else(|| EditError::apply(format!("File not found: {}", resolved.display)))
	}

	fn try_read(&mut self, resolved: &Resolved) -> EditResult<Option<Arc<FileRead>>> {
		self.load(resolved).map_err(edit_error)
	}

	fn clear(&mut self) {}
}

pub(crate) fn replace(root: &Path, args: &EditArgs, cancellation: &CancelToken) -> Result<String> {
	Ok(replace_detailed(root, args, cancellation)?.content)
}

pub(crate) fn replace_detailed(
	root: &Path,
	args: &EditArgs,
	cancellation: &CancelToken,
) -> Result<crate::tools::ToolOutput> {
	let fs = JailedFs::open(root)?;
	let policy = PathPolicy {
		cwd:                  fs.root().to_owned(),
		home_dir:             fs.root().to_owned(),
		local_sandbox_root:   None,
		vault_roots:          None,
		plan_active:          false,
		block_auto_generated: true,
	};
	let mut files = WorkspaceFiles { fs, policy, cancellation };
	let snapshot = ArgSnapshot {
		path: Some(args.path.clone()),
		old_string: Some(args.old.clone()),
		new_string: Some(args.new.clone()),
		complete: true,
		..ArgSnapshot::default()
	};
	let engine = ReplaceEngine { allow_fuzzy: false, fuzzy_threshold: 1.0 };
	let staged = engine.stage(&snapshot, &mut files, &EditStore::default())?;
	let file = staged.first().context("edit produced no staged file")?;
	// Reuse the engine's actual staged diff, not the requested old/new strings.
	// Never publish truncated diffs as complete calibration data.
	let noop = file.op == pi_edit::FileOp::Noop;
	let diff_available = !noop && !file.diff.is_empty() && file.diff.len() <= 32 * 1024;
	let details = serde_json::json!({
		 "op":file.op.as_str(),"path":file.display,"noop":noop,
		 "diff":if diff_available {Some(file.diff.as_str())} else {None},
		 "diffUnavailable":if noop {Some("no_change")} else if file.diff.is_empty() {Some("not_reported")} else if !diff_available {Some("exceeds_limit")} else {None},
	});
	if serde_json::to_vec(&details)?.len() > crate::tools::MAX_TOOL_DETAILS_BYTES {
		bail!("edit metadata exceeds limit");
	}
	if noop {
		cancellation.heartbeat()?;
		return Ok(crate::tools::ToolOutput {
			is_error: false,
			content:  format!("unchanged {}", file.display),
			details:  Some(details),
		});
	}
	let content = file
		.persisted
		.as_deref()
		.context("edit produced no content")?;
	if content.len() > MAX_FILE_BYTES {
		bail!("edit exceeds file limit");
	}
	cancellation.heartbeat()?;
	files.fs.atomic_write(&file.absolute, content.as_bytes())?;
	Ok(crate::tools::ToolOutput {
		is_error: false,
		content:  format!("edited {}", file.display),
		details:  Some(details),
	})
}
