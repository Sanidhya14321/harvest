//! A host source must remain authoritative across preview/stage/apply, rather
//! than falling back to ambient files or trusting stale preview bytes.
mod common;
use std::{
	path::Path,
	sync::{Arc, Mutex},
};

use common::{DiskWriter, Workspace};
use pi_edit::{
	ApplyRequest, EditError, EditMode, EditResult, EditStore, PathPolicy, Resolved, Session,
	files::{FileRead, FileSource},
};
#[derive(Default)]
struct State {
	text:  Option<String>,
	reads: usize,
}
struct HostSource {
	policy: PathPolicy,
	state:  Arc<Mutex<State>>,
	cached: Option<Arc<FileRead>>,
}
impl FileSource for HostSource {
	fn policy(&self) -> &PathPolicy {
		&self.policy
	}

	fn resolve(&mut self, path: &str, _must_exist: bool) -> EditResult<Resolved> {
		self.policy.resolve(path)
	}

	fn exists(&mut self, _path: &Path) -> bool {
		self.state.lock().unwrap().text.is_some()
	}

	fn read(&mut self, path: &str) -> EditResult<Arc<FileRead>> {
		let resolved = self.resolve(path, true)?;
		self
			.try_read(&resolved)?
			.ok_or_else(|| EditError::apply("host capability revoked"))
	}

	fn try_read(&mut self, resolved: &Resolved) -> EditResult<Option<Arc<FileRead>>> {
		if let Some(file) = &self.cached {
			return Ok(Some(file.clone()));
		}
		let mut state = self.state.lock().unwrap();
		state.reads += 1;
		let Some(text) = &state.text else {
			return Ok(None);
		};
		let file = Arc::new(FileRead::from_raw(resolved.clone(), text.clone())?);
		self.cached = Some(file.clone());
		Ok(Some(file))
	}

	fn clear(&mut self) {
		self.cached = None;
	}
}
fn session(ws: &Workspace, state: Arc<Mutex<State>>) -> Session {
	let source = HostSource { policy: ws.config.policy.clone(), state, cached: None };
	let mut session =
		Session::with_file_source(ws.config.clone(), ws.store.clone(), source).unwrap();
	session.set_args_json(r#"{"path":"file.txt","old_string":"alpha","new_string":"beta"}"#);
	session.finish();
	session
}
#[tokio::test]
async fn injected_reads_stage_without_writes_and_apply_rereads_after_preview() {
	let mut ws = Workspace::new(EditMode::Replace);
	ws.config.allow_fuzzy = false;
	ws.write("file.txt", "ambient disk contents\n");
	let state =
		Arc::new(Mutex::new(State { text: Some("alpha\nkeep\n".into()), ..Default::default() }));
	let mut session = session(&ws, state.clone());
	let preview = session.preview();
	assert!(preview.files.iter().all(|f| f.error.is_none()));
	let staged = session.stage().unwrap();
	assert_eq!(staged[0].persisted.as_deref(), Some("beta\nkeep\n"));
	assert_eq!(ws.read("file.txt").as_deref(), Some("ambient disk contents\n"));
	state.lock().unwrap().text = Some("alpha\nexternal update\n".into());
	let writer = DiskWriter::default();
	session
		.apply(ApplyRequest::default(), &writer)
		.await
		.unwrap();
	assert_eq!(ws.read("file.txt").as_deref(), Some("beta\nexternal update\n"));
}
#[tokio::test]
async fn revoked_host_read_never_falls_back_to_existing_ambient_file() {
	let ws = Workspace::new(EditMode::Replace);
	ws.write("file.txt", "alpha\n");
	let state = Arc::new(Mutex::new(State { text: Some("alpha\n".into()), ..Default::default() }));
	let mut session = session(&ws, state.clone());
	session.preview();
	state.lock().unwrap().text = None;
	let writer = DiskWriter::default();
	assert!(
		session
			.apply(ApplyRequest::default(), &writer)
			.await
			.is_err()
	);
	assert_eq!(ws.read("file.txt").as_deref(), Some("alpha\n"));
}
#[test]
fn conflicting_host_policy_is_rejected_before_any_read() {
	let ws = Workspace::new(EditMode::Replace);
	let state = Arc::new(Mutex::new(State::default()));
	let mut policy = ws.config.policy.clone();
	policy.plan_active = !policy.plan_active;
	let source = HostSource { policy, state: state.clone(), cached: None };
	assert!(Session::with_file_source(ws.config.clone(), EditStore::default(), source).is_err());
	assert_eq!(state.lock().unwrap().reads, 0);
}
#[test]
fn incomplete_arguments_are_rejected_by_public_staging() {
	let ws = Workspace::new(EditMode::Replace);
	ws.write("file.txt", "alpha");
	let mut session = ws.session();
	session.push(r#"{"path":"file.txt","old_string":"alpha","new_string":"beta""#);
	assert!(session.stage().is_err());
	assert_eq!(ws.read("file.txt").as_deref(), Some("alpha"));
}
// The existing native addon owns Session behind a Tokio mutex. Preserve its
// Send/Sync bridge contract as a compile-time requirement, without a runtime
// test.
const _: fn() = || {
	fn require<T: Send + Sync>() {}
	require::<Session>();
};
