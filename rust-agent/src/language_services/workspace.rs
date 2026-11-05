//! Ordered LSP workspace operations staged through the existing native writer.
//! A complete virtual preflight precedes approval; publication is sequential
//! and reports committed prefixes, with no whole-workspace rollback promise.

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use futures::FutureExt;
use harvest_runtime::{
	BoundedWriter, JailedFs, MAX_EVENT_BYTES, MAX_FILE_BYTES, cancel::CancelToken,
};
use pi_edit::{EditWriter, FileOp, Resolved, StagedFile, WriteRequest};
use serde::Deserialize;
use serde_json::{Value, json};

use super::edits::{self, ApplyResult, TextEdit, WorkspaceChange};
use crate::{
	chat::{FunctionCall, ToolCall},
	edit::NativeWriter,
	tools::Approval,
};

const MAX_OPERATIONS: usize = 256;
const MAX_PLAN_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
struct Target {
	uri:  String,
	path: PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Options {
	#[serde(default)]
	overwrite:            bool,
	#[serde(default)]
	ignore_if_exists:     bool,
	#[serde(default)]
	ignore_if_not_exists: bool,
	#[serde(default)]
	recursive:            bool,
}

enum Kind {
	Text(Target, Vec<TextEdit>),
	Create(Target, Options),
	Rename(Target, Target, Options),
	Delete(Target, Options),
}
struct Operation {
	index: usize,
	kind:  Kind,
}
struct PendingText {
	index:  usize,
	target: Target,
	edits:  Vec<TextEdit>,
}
struct Stage {
	index:       usize,
	file:        StagedFile,
	source:      Target,
	destination: Option<Target>,
	change:      WorkspaceChange,
}

fn target(fs: &JailedFs, value: &Value, aliases: &mut BTreeMap<PathBuf, String>) -> Result<Target> {
	let uri = value.as_str().context("LSP resource has no file URI")?;
	ensure!(!uri.is_empty() && uri.len() <= 4096, "LSP resource URI exceeds limit");
	let path = edits::uri_path(fs, uri)?;
	let key = pi_edit::path_policy::canonical_key(&path);
	if let Some(previous) = aliases.get(&key) {
		ensure!(previous == uri, "multiple LSP URI aliases target the same resource");
	} else {
		ensure!(aliases.len() < MAX_OPERATIONS, "too many LSP workspace resources");
		aliases.insert(key, uri.to_owned());
	}
	Ok(Target { uri: uri.to_owned(), path })
}

fn flush(
	pending: &mut Vec<PendingText>,
	operations: &mut Vec<Operation>,
	roots: &[&str],
	subtree: bool,
) {
	let mut retained = Vec::new();
	for text in pending.drain(..) {
		let matches = roots.is_empty()
			|| roots.iter().any(|root| {
				text.target.uri == *root
					|| (subtree
						&& text
							.target
							.uri
							.strip_prefix(root.trim_end_matches('/'))
							.is_some_and(|suffix| suffix.starts_with('/')))
			});
		if matches {
			operations
				.push(Operation { index: text.index, kind: Kind::Text(text.target, text.edits) });
		} else {
			retained.push(text);
		}
	}
	*pending = retained;
}

fn operations(
	fs: &JailedFs,
	edit: &Value,
	versions: &BTreeMap<String, i64>,
) -> Result<Vec<Operation>> {
	let mut aliases = BTreeMap::new();
	let mut operations = Vec::new();
	let mut edit_count = 0usize;
	// The modern ordered representation wins when both representations occur,
	// as in the retained TypeScript consumer.
	if let Some(changes) = edit.get("documentChanges") {
		let changes = changes.as_array().context("invalid LSP document changes")?;
		ensure!(changes.len() <= MAX_OPERATIONS, "too many LSP document changes");
		let mut pending: Vec<PendingText> = Vec::new();
		for (index, change) in changes.iter().enumerate() {
			ensure!(change.is_object(), "invalid LSP document change");
			if change.get("textDocument").is_some() {
				ensure!(change.get("kind").is_none(), "ambiguous LSP document/resource change");
				let document = &change["textDocument"];
				let target = target(fs, &document["uri"], &mut aliases)?;
				if let Some(version) = document.get("version").filter(|v| !v.is_null()) {
					ensure!(
						version
							.as_i64()
							.is_some_and(|v| versions.get(&target.uri) == Some(&v)),
						"stale or unknown LSP document version"
					);
				}
				let edits: Vec<TextEdit> =
					serde_json::from_value(change["edits"].clone()).context("invalid LSP text edits")?;
				edit_count += edits.len();
				ensure!(edit_count <= 10_000, "too many LSP text edits");
				if !edits.is_empty() {
					if let Some(previous) = pending.iter_mut().find(|p| p.target.uri == target.uri) {
						previous.edits.extend(edits);
					} else {
						pending.push(PendingText { index, target, edits });
					}
				}
				continue;
			}
			// Resolve every resource URI before branching on its operation, even
			// for ignored create/delete requests and unsupported resource kinds.
			let mut targets = BTreeMap::new();
			for key in ["uri", "oldUri", "newUri"] {
				if let Some(value) = change.get(key) {
					targets.insert(key, target(fs, value, &mut aliases)?);
				}
			}
			let options: Options = match change.get("options").filter(|v| !v.is_null()) {
				Some(value) => {
					serde_json::from_value(value.clone()).context("invalid LSP resource options")?
				},
				None => Options::default(),
			};
			let kind = match change["kind"].as_str() {
				Some("create") => {
					let target = targets.remove("uri").context("LSP create has no URI")?;
					flush(&mut pending, &mut operations, &[&target.uri], false);
					Kind::Create(target, options)
				},
				Some("rename") => {
					let source = targets
						.remove("oldUri")
						.context("LSP rename has no old URI")?;
					let destination = targets
						.remove("newUri")
						.context("LSP rename has no new URI")?;
					flush(&mut pending, &mut operations, &[&source.uri, &destination.uri], true);
					Kind::Rename(source, destination, options)
				},
				Some("delete") => {
					let target = targets.remove("uri").context("LSP delete has no URI")?;
					flush(&mut pending, &mut operations, &[&target.uri], true);
					Kind::Delete(target, options)
				},
				_ => bail!("unsupported LSP workspace resource operation"),
			};
			ensure!(targets.is_empty(), "unexpected URI fields in LSP resource operation");
			operations.push(Operation { index, kind });
		}
		flush(&mut pending, &mut operations, &[], false);
	} else if let Some(changes) = edit.get("changes") {
		let changes = changes.as_object().context("invalid LSP changes mapping")?;
		ensure!(changes.len() <= MAX_OPERATIONS, "too many workspace edit files");
		for (index, (uri, value)) in changes.iter().enumerate() {
			let target = target(fs, &Value::String(uri.clone()), &mut aliases)?;
			let edits: Vec<TextEdit> =
				serde_json::from_value(value.clone()).context("invalid LSP text edits")?;
			edit_count += edits.len();
			ensure!(edit_count <= 10_000, "too many LSP text edits");
			if !edits.is_empty() {
				operations.push(Operation { index, kind: Kind::Text(target, edits) });
			}
		}
	}
	ensure!(operations.len() <= MAX_OPERATIONS, "too many ordered workspace operations");
	Ok(operations)
}

struct Planner<'a> {
	fs:           &'a JailedFs,
	cancellation: &'a CancelToken,
	initial:      BTreeMap<PathBuf, Option<String>>,
	current:      BTreeMap<PathBuf, Option<String>>,
	bytes:        usize,
}
impl Planner<'_> {
	fn charge(&mut self, bytes: usize) -> Result<()> {
		self.bytes = self
			.bytes
			.checked_add(bytes)
			.context("workspace plan size overflow")?;
		ensure!(self.bytes <= MAX_PLAN_BYTES, "workspace edit plan exceeds byte limit");
		Ok(())
	}

	fn load(&mut self, target: &Target) -> Result<Option<String>> {
		self.cancellation.heartbeat()?;
		if let Some(value) = self.current.get(&target.path) {
			return Ok(value.clone());
		}
		let bytes = self
			.fs
			.read_optional(&target.path, MAX_FILE_BYTES)
			.with_context(|| {
				format!("workspace resource must be a bounded regular file: {}", target.uri)
			})?;
		let text = bytes
			.map(|bytes| -> Result<String> {
				let text = String::from_utf8(bytes)
					.context("binary LSP resource operations are not converted")?;
				ensure!(!text.contains('\0'), "LSP resource is not text");
				Ok(text)
			})
			.transpose()?;
		self.charge(text.as_ref().map_or(0, String::len) * 2)?;
		self.initial.insert(target.path.clone(), text.clone());
		self.current.insert(target.path.clone(), text.clone());
		Ok(text)
	}

	fn stage(&mut self, operation: Operation) -> Result<Option<Stage>> {
		let index = operation.index;
		let (source, destination, before, after, op, change) = match operation.kind {
			Kind::Text(target, edits) => {
				let before = self
					.load(&target)?
					.context("LSP text-edit target does not exist")?;
				let after = edits::apply_text_edits(&before, &edits)?;
				if before == after {
					return Ok(None);
				}
				let change = WorkspaceChange::Edit { path: target.path.clone() };
				(target, None, Some(before), Some(after), FileOp::Update, change)
			},
			Kind::Create(target, options) => {
				let before = self.load(&target)?;
				if before.is_some() && !options.overwrite {
					if options.ignore_if_exists {
						return Ok(None);
					}
					bail!("LSP create target already exists");
				}
				let op = if before.is_some() {
					FileOp::Update
				} else {
					FileOp::Create
				};
				if before.as_deref() == Some("") {
					return Ok(None);
				}
				let change = WorkspaceChange::Create { path: target.path.clone() };
				(target, None, before, Some(String::new()), op, change)
			},
			Kind::Delete(target, options) => {
				// Recursive files are valid; directory deletion remains an explicit
				// unsupported boundary of the shared file writer.
				let _recursive = options.recursive;
				let before = self.load(&target)?;
				if before.is_none() {
					if options.ignore_if_not_exists {
						return Ok(None);
					}
					bail!("LSP delete target does not exist");
				}
				let change = WorkspaceChange::Delete { path: target.path.clone() };
				(target, None, before, None, FileOp::Delete, change)
			},
			Kind::Rename(source, destination, options) => {
				let target_before = self.load(&destination)?;
				if source.path == destination.path {
					self
						.load(&source)?
						.context("LSP rename source does not exist")?;
					return Ok(None);
				}
				if target_before.is_some() {
					if !options.overwrite && options.ignore_if_exists {
						return Ok(None);
					}
					ensure!(
						!options.overwrite,
						"LSP rename overwrite requires destination recovery not yet provided by the \
						 shared writer; no edits applied"
					);
					bail!("LSP rename target already exists");
				}
				let before = self
					.load(&source)?
					.context("LSP rename source does not exist")?;
				let change = WorkspaceChange::Rename {
					old_path: source.path.clone(),
					new_path: destination.path.clone(),
				};
				(source, Some(destination), Some(before.clone()), Some(before), FileOp::Update, change)
			},
		};
		self
			.charge(before.as_ref().map_or(0, String::len) + after.as_ref().map_or(0, String::len))?;
		let mut file = StagedFile::new(source.uri.clone(), source.path.clone(), op);
		file.existed = before.is_some();
		file.before_raw = before;
		file.persisted = after.clone();
		if let Some(destination) = &destination {
			file.move_to = Some(Resolved {
				absolute: destination.path.clone(),
				display:  destination.uri.clone(),
			});
			self.current.insert(source.path.clone(), None);
			self.current.insert(destination.path.clone(), after);
		} else {
			self.current.insert(source.path.clone(), after);
		}
		Ok(Some(Stage { index, file, source, destination, change }))
	}
}

fn record(result: &mut ApplyResult, change: WorkspaceChange) {
	for path in change.paths() {
		if !result.changed_files.iter().any(|existing| existing == path) {
			result.changed_files.push(path.to_owned());
		}
	}
	result.executed_changes.push(change);
	result.synchronization_pending = true;
}

pub(super) fn apply(
	root: &Path,
	edit: &Value,
	versions: &BTreeMap<String, i64>,
	approval: &dyn Approval,
	cancellation: &CancelToken,
	context: Option<&Value>,
) -> Result<ApplyResult> {
	cancellation.heartbeat()?;
	ensure!(edit.is_object(), "workspace edit must be an object");
	let mut arguments = BoundedWriter::new(MAX_EVENT_BYTES);
	serde_json::to_writer(
		&mut arguments,
		&json!({"action":"apply_workspace_edit","edit":edit,"context":context}),
	)?;
	let arguments = String::from_utf8(arguments.into_inner())?;
	let fs = Arc::new(JailedFs::open(root)?);
	let mut planner = Planner {
		fs: &fs,
		cancellation,
		initial: BTreeMap::new(),
		current: BTreeMap::new(),
		bytes: 0,
	};
	let mut stages = Vec::new();
	for operation in operations(&fs, edit, versions)? {
		if let Some(stage) = planner.stage(operation)? {
			stages.push(stage);
		}
	}
	for path in planner.initial.keys() {
		ensure!(
			!planner
				.initial
				.keys()
				.any(|other| path != other && path.starts_with(other)),
			"workspace file operations have overlapping ancestor targets"
		);
	}
	// Reserve space for the complete committed-prefix report before any write.
	let mut report = BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES / 4);
	serde_json::to_writer(
		&mut report,
		&json!({"changedFiles":planner.initial.keys().collect::<Vec<_>>(),
		"executedChanges":stages.iter().map(|stage| &stage.change).collect::<Vec<_>>()}),
	)?;
	let mut result = ApplyResult {
		applied:                 true,
		synchronization_pending: false,
		changed_files:           Vec::new(),
		failure_reason:          None,
		failed_file_index:       None,
		failed_change_index:     None,
		executed_changes:        Vec::new(),
	};
	if stages.is_empty() {
		return Ok(result);
	}
	static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
	let id = ID
		.try_update(
			std::sync::atomic::Ordering::Relaxed,
			std::sync::atomic::Ordering::Relaxed,
			|id| id.checked_add(1),
		)
		.map_err(|_| anyhow::anyhow!("LSP approval IDs exhausted"))?;
	let call = ToolCall {
		id:       format!("lsp-workspace-edit-{id}"),
		kind:     "function".into(),
		function: FunctionCall { name: "lsp".into(), arguments },
	};
	if !approval.approve(&call)? {
		result.applied = false;
		result.failure_reason = Some("host_approval_denied");
		return Ok(result);
	}
	cancellation.heartbeat()?;
	for (path, before) in &planner.initial {
		ensure!(
			fs.read_optional(path, MAX_FILE_BYTES)?.as_deref()
				== before.as_ref().map(String::as_bytes),
			"workspace edit target changed during approval"
		);
	}
	for (index, stage) in stages.into_iter().enumerate() {
		let writer = NativeWriter::new(fs.clone(), cancellation.clone());
		let write = (|| -> Result<()> {
			cancellation.heartbeat()?;
			for target in std::iter::once(&stage.source).chain(stage.destination.iter()) {
				ensure!(
					edits::uri_path(&fs, &target.uri)? == target.path,
					"LSP resource resolution changed during approval"
				);
			}
			writer.prepare(std::slice::from_ref(&stage.file))?;
			writer
				.write(WriteRequest {
					absolute:     stage.file.absolute.clone(),
					display:      stage.file.display.clone(),
					op:           stage.file.op,
					move_to:      stage.file.move_to.as_ref().map(|p| p.absolute.clone()),
					content:      stage.file.persisted.clone(),
					flush_lsp:    false,
					lsp_batch_id: None,
				})
				.now_or_never()
				.context("native workspace writer unexpectedly yielded")??;
			Ok(())
		})();
		if write.is_err() {
			let failure =
				writer.failure(pi_edit::EditError::apply("LSP workspace publication failed"));
			for committed in failure.committed {
				let change = match committed.operation {
					"delete" | "move_source_removed" => WorkspaceChange::Delete { path: committed.path },
					"create" | "move_destination" => WorkspaceChange::Create { path: committed.path },
					_ => WorkspaceChange::Edit { path: committed.path },
				};
				record(&mut result, change);
			}
			result.applied = false;
			result.failed_file_index = Some(index);
			result.failed_change_index = Some(stage.index);
			result.failure_reason = Some("workspace_edit_write_failed_or_cancelled");
			break;
		}
		record(&mut result, stage.change);
	}
	Ok(result)
}
