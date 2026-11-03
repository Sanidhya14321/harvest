//! Native edit staging uses pi-edit; the application owns approvals and bytes.
use std::{
	path::Path,
	sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use pi_edit::{
	EditError, EditMode, EditResult, EditStore, PathPolicy, Resolved, Session,
	files::{FileRead, FileSource},
	session::SessionConfig,
};

use crate::tools::EditArgs;

/// Host-selected mode. Model arguments cannot switch the session's edit
/// contract.
#[derive(
	Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
	#[default]
	Replace,
	Patch,
	#[value(name = "apply_patch")]
	ApplyPatch,
	Hashline,
	Sloppy,
}
impl Mode {
	pub const fn engine(self) -> EditMode {
		match self {
			Self::Replace => EditMode::Replace,
			Self::Patch => EditMode::Patch,
			Self::ApplyPatch => EditMode::ApplyPatch,
			Self::Hashline => EditMode::Hashline,
			Self::Sloppy => EditMode::Sloppy,
		}
	}

	/// A supplied host flag wins even when a lower-priority environment value
	/// is malformed. The native file/profile resolver runs after this selection.
	pub fn select(explicit: Option<Self>, environment: Option<&str>) -> Result<Option<Self>> {
		if let Some(mode) = explicit {
			return Ok(Some(mode));
		}
		environment.map(Self::parse).transpose()
	}

	pub fn parse(value: &str) -> Result<Self> {
		serde_json::from_value(serde_json::Value::String(value.to_owned()))
			.context("unknown native edit mode")
	}
}

pub mod read;

struct WorkspaceFiles {
	fs:           Arc<JailedFs>,
	policy:       PathPolicy,
	cancellation: CancelToken,
	read_bytes:   AtomicUsize,
	read_count:   AtomicUsize,
}

impl WorkspaceFiles {
	fn load(&self, resolved: &Resolved) -> Result<Option<Arc<FileRead>>> {
		self.cancellation.heartbeat()?;
		if self.read_count.fetch_add(1, Ordering::Relaxed) >= 256 {
			bail!("edit source exceeds read-operation budget");
		}
		const READ_LIMIT: usize = 128 * 1024 * 1024;
		if self.read_bytes.load(Ordering::Relaxed) >= READ_LIMIT {
			bail!("edit source exceeds aggregate byte budget");
		}
		let Some(bytes) = self.fs.read_optional(&resolved.absolute, MAX_FILE_BYTES)? else {
			return Ok(None);
		};
		// Charge even a rejected read, so an overflow permanently exhausts
		// this call instead of allowing repeated uncharged preview reads.
		if self
			.read_bytes
			.fetch_add(bytes.len(), Ordering::Relaxed)
			.saturating_add(bytes.len())
			> READ_LIMIT
		{
			bail!("edit source exceeds aggregate byte budget");
		}

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

impl FileSource for WorkspaceFiles {
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
		self.cancellation.heartbeat().is_ok() && self.fs.file_type(absolute).is_ok()
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
	let fs = Arc::new(JailedFs::open(root)?);
	let policy = PathPolicy {
		cwd:                  fs.root().to_owned(),
		home_dir:             fs.root().to_owned(),
		local_sandbox_root:   None,
		vault_roots:          None,
		plan_active:          false,
		block_auto_generated: true,
	};
	let files = WorkspaceFiles {
		fs:           fs.clone(),
		policy:       policy.clone(),
		cancellation: cancellation.clone(),
		read_bytes:   AtomicUsize::new(0),
		read_count:   AtomicUsize::new(0),
	};
	let report = apply_authorized(
		fs,
		files,
		policy,
		EditMode::Replace,
		&serde_json::to_string(
			&serde_json::json!({"path":args.path,"old_string":args.old,"new_string":args.new}),
		)?,
		EditStore::default(),
		cancellation,
		true,
	)?;
	let file = report
		.outcome
		.files
		.first()
		.context("edit produced no file")?;
	Ok(crate::tools::ToolOutput {
		is_error: false,
		content:  format!(
			"{} {}",
			if file.op == pi_edit::FileOp::Noop {
				"unchanged"
			} else {
				"edited"
			},
			file.display
		),
		details:  report.details.into_iter().next(),
	})
}

mod writer;
use writer::NativeWriter;
pub use writer::{CommittedChange, NativeEditFailure};

/// Native SDK result; file snapshots and metadata describe actual engine
/// output.
#[derive(Debug)]
pub struct NativeEditReport {
	pub outcome: pi_edit::ApplyOutcome,
	pub details: Vec<serde_json::Value>,
}
/// Explicit host/SDK entrypoint using a supplied session store. Model hosts
/// normally use ToolRuntime to share read provenance and host-selected mode.
/// The approval policy sees the final mode plus complete argument object.
pub fn apply_with_approval(
	root: &Path,
	mode: EditMode,
	arguments: &str,
	store: EditStore,
	approval: &dyn crate::tools::Approval,
	cancellation: &CancelToken,
) -> Result<NativeEditReport> {
	cancellation.heartbeat()?;
	if arguments.len() > harvest_runtime::MAX_EVENT_BYTES {
		bail!("edit arguments exceed frame limit");
	}
	let arguments: serde_json::Value = serde_json::from_str(arguments)?;
	if !arguments.is_object() {
		bail!("edit arguments must be an object");
	}
	let approved = crate::chat::ToolCall {
		id:       "native-edit".into(),
		kind:     "function".into(),
		function: crate::chat::FunctionCall {
			name:      "edit".into(),
			arguments: serde_json::to_string(
				&serde_json::json!({"mode":mode.as_str(),"arguments":arguments}),
			)?,
		},
	};
	if !approval.approve(&approved)? {
		bail!("tool edit requires human approval");
	}
	cancellation.heartbeat()?;
	let fs = Arc::new(JailedFs::open(root)?);
	let policy = PathPolicy {
		cwd:                  fs.root().to_owned(),
		home_dir:             fs.root().to_owned(),
		local_sandbox_root:   None,
		vault_roots:          None,
		plan_active:          false,
		block_auto_generated: true,
	};
	let files = WorkspaceFiles {
		fs:           fs.clone(),
		policy:       policy.clone(),
		cancellation: cancellation.clone(),
		read_bytes:   AtomicUsize::new(0),
		read_count:   AtomicUsize::new(0),
	};
	apply_authorized(
		fs,
		files,
		policy,
		mode,
		&serde_json::to_string(&arguments)?,
		store,
		cancellation,
		false,
	)
}
/// Session-owned dispatch. Approval includes the host's mode and the exact
/// final model arguments; legacy replace aliases are translated only after
/// approval.
pub(crate) fn invoke(
	root: &Path,
	mode: Mode,
	call: &crate::chat::ToolCall,
	store: EditStore,
	approval: &dyn crate::tools::Approval,
	cancellation: &CancelToken,
) -> Result<crate::tools::ToolOutput> {
	let arguments: serde_json::Value = serde_json::from_str(&call.function.arguments)
		.context("edit arguments are not valid JSON")?;
	if !arguments.is_object() {
		bail!("edit arguments must be an object");
	}
	// Preserve original call identity and aliases at the approval boundary.
	struct FinalApproval<'a> {
		policy: &'a dyn crate::tools::Approval,
		call:   &'a crate::chat::ToolCall,
		mode:   Mode,
	}
	impl crate::tools::Approval for FinalApproval<'_> {
		fn approve(&self, _: &crate::chat::ToolCall) -> Result<bool> {
			let mut call = self.call.clone();
			call.function.arguments = serde_json::json!({
				 "mode":self.mode.engine().as_str(),
				 "arguments":serde_json::from_str::<serde_json::Value>(&self.call.function.arguments)?
			})
			.to_string();
			self.policy.approve(&call)
		}
	}
	let mut engine_arguments = arguments;
	if mode == Mode::Replace {
		let object = engine_arguments.as_object_mut().expect("object checked");
		for (legacy, native) in [("old", "old_string"), ("new", "new_string")] {
			if let Some(value) = object.remove(legacy) {
				if object.contains_key(native) {
					bail!("ambiguous replacement fields");
				}
				object.insert(native.into(), value);
			}
		}
	}
	match apply_with_approval(
		root,
		mode.engine(),
		&engine_arguments.to_string(),
		store,
		&FinalApproval { policy: approval, call, mode },
		cancellation,
	) {
		Ok(report) => {
			let content = if mode == Mode::Replace && report.outcome.files.len() == 1 {
				let file = &report.outcome.files[0];
				format!(
					"{} {}",
					if file.op == pi_edit::FileOp::Noop {
						"unchanged"
					} else {
						"edited"
					},
					file.display
				)
			} else {
				crate::tools::bounded(&report.outcome.text)
			};
			let mut details = if report.details.len() == 1 {
				report.details.into_iter().next().expect("one detail")
			} else {
				serde_json::json!({"noop":report.outcome.files.iter().all(|f| f.op == pi_edit::FileOp::Noop),
                    "files":report.details})
			};
			details["editMode"] = serde_json::to_value(mode)?;
			Ok(crate::tools::ToolOutput { is_error: false, content, details: Some(details) })
		},
		Err(error) => {
			if let Some(partial) = error.downcast_ref::<NativeEditFailure>() {
				return Ok(crate::tools::ToolOutput {
					is_error: true,
					content:  crate::tools::bounded(&format!("{error:#}")),
					details:  Some(serde_json::json!({"editMode":mode,"committed":partial.committed,
                        "progressUnavailable":partial.progress_unavailable})),
				});
			}
			Err(error)
		},
	}
}

fn apply_authorized(
	fs: Arc<JailedFs>,
	files: WorkspaceFiles,
	policy: PathPolicy,
	mode: EditMode,
	arguments: &str,
	store: EditStore,
	cancellation: &CancelToken,
	single: bool,
) -> Result<NativeEditReport> {
	let mut session = Session::with_file_source(
		SessionConfig {
			mode,
			policy,
			allow_fuzzy: false,
			fuzzy_threshold: 1.0,
			enforce_seen_lines: true,
			raw_input: false,
		},
		store,
		files,
	)?;
	if mode == EditMode::Hashline {
		session.require_read_provenance();
	}
	session.set_args_json(arguments);
	session.finish();
	let inspection = session.inspection();
	if inspection.paths.is_empty() || inspection.paths.len() > 32 || inspection.file_ops.len() > 32 {
		bail!("native edit must declare 1..32 targets");
	}
	let mut path_bytes = 0usize;
	for path in
		inspection
			.paths
			.iter()
			.map(String::as_str)
			.chain(inspection.file_ops.iter().flat_map(|op| match op {
				pi_edit::FileOpIntent::Delete { path } => vec![path.as_str()],
				pi_edit::FileOpIntent::Move { from, to } => vec![from.as_str(), to.as_str()],
			})) {
		if path.is_empty() || path.len() > 4096 {
			bail!("edit target path exceeds limit");
		}
		path_bytes += path.len();
		if path_bytes > 128 * 1024 {
			bail!("edit target paths exceed aggregate limit");
		}
	}
	let writer = NativeWriter::new(fs, cancellation.clone());
	let mut details = Vec::new();
	let result = futures::FutureExt::now_or_never(session.apply_with_preflight(
		Default::default(),
		&writer,
		|staged| {
			if single && staged.len() != 1 {
				return Err(EditError::apply("replace requires exactly one staged file"));
			}
			details = writer.prepare(staged).map_err(edit_error)?;
			Ok(())
		},
	));
	// NativeWriter performs only synchronous capability I/O; its futures never
	// yield. This checked adapter creates no executor or nested runtime.
	let result =
		result.unwrap_or_else(|| Err(EditError::apply("native edit writer unexpectedly yielded")));
	match result {
		Ok(outcome) => Ok(NativeEditReport { outcome, details }),
		Err(error) => Err(writer.failure(error).into()),
	}
}
