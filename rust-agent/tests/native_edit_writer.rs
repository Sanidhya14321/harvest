//! Native publication exercises real jailed files; no transcript code runs.
use std::{fs, sync::Mutex};

use anyhow::Result;
use harvest_omp::{
	chat::ToolCall,
	edit::{NativeEditFailure, apply_with_approval},
	tools::{Approval, DenyMutations},
};
use harvest_runtime::{JailedFs, cancel::CancelToken};
use pi_edit::{EditMode, EditStore};
use serde_json::json;
#[derive(Default)]
struct Allow {
	requests: Mutex<Vec<String>>,
}
impl Approval for Allow {
	fn approve(&self, call: &ToolCall) -> Result<bool> {
		self
			.requests
			.lock()
			.unwrap()
			.push(call.function.arguments.clone());
		Ok(true)
	}
}
#[test]
fn multi_file_create_update_delete_move_use_one_final_approval() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::write(root.path().join("note.txt"), "old\n")?;
	fs::write(root.path().join("removed.txt"), "remove\n")?;
	fs::write(root.path().join("move.txt"), "move old\n")?;
	let input = "*** Begin Patch\n*** Update File: note.txt\n@@\n-old\n+new\n*** Add File: \
	             added.txt\n+created\n*** Delete File: removed.txt\n*** Update File: move.txt\n*** \
	             Move to: moved.txt\n@@\n-move old\n+move new\n*** End Patch";
	let allow = Allow::default();
	let report = apply_with_approval(
		root.path(),
		EditMode::ApplyPatch,
		&json!({"input":input}).to_string(),
		EditStore::default(),
		&allow,
		&CancelToken::default(),
	)?;
	assert_eq!(fs::read(root.path().join("note.txt"))?, b"new\n");
	assert_eq!(fs::read(root.path().join("added.txt"))?, b"created\n");
	assert_eq!(fs::read(root.path().join("moved.txt"))?, b"move new\n");
	assert!(!root.path().join("removed.txt").exists());
	assert!(!root.path().join("move.txt").exists());
	assert_eq!(report.outcome.files.len(), 4);
	let calls = allow.requests.lock().unwrap();
	assert_eq!(calls.len(), 1);
	let approved: serde_json::Value = serde_json::from_str(&calls[0])?;
	assert_eq!(approved["mode"], "apply_patch");
	assert_eq!(approved["arguments"]["input"], input);
	Ok(())
}
#[test]
fn ownership_conflict_and_denial_do_not_change_files() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::write(root.path().join("note.txt"), "old\n")?;
	let args = json!({"path":"note.txt","old_string":"old","new_string":"new"}).to_string();
	assert!(
		apply_with_approval(
			root.path(),
			EditMode::Replace,
			&args,
			EditStore::default(),
			&DenyMutations,
			&CancelToken::default()
		)
		.is_err()
	);
	let jailed = JailedFs::open(root.path())?;
	let ownership = jailed.resource_lock(std::path::Path::new("note.txt"))?;
	let error = apply_with_approval(
		root.path(),
		EditMode::Replace,
		&args,
		EditStore::default(),
		&Allow::default(),
		&CancelToken::default(),
	)
	.unwrap_err();
	assert!(
		error
			.downcast_ref::<NativeEditFailure>()
			.unwrap()
			.committed
			.is_empty()
	);
	assert_eq!(fs::read(root.path().join("note.txt"))?, b"old\n");
	drop(ownership);
	Ok(())
}
#[test]
fn hashline_sdk_requires_seen_lines_and_commits_real_snapshot() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("note.txt");
	fs::write(&path, "old\n")?;
	let path = path.canonicalize()?;
	let store = EditStore::default();
	let tag = store.record(&path, "old\n", Some(&[]));
	let args = json!({"input":format!("[note.txt#{tag}]\nPUT 1.=1:\n+new\n")}).to_string();
	assert!(
		apply_with_approval(
			root.path(),
			EditMode::Hashline,
			&args,
			store.clone(),
			&Allow::default(),
			&CancelToken::default()
		)
		.is_err()
	);
	assert_eq!(fs::read(&path)?, b"old\n");
	store.record_seen_lines(&path, &tag, &[1]);
	apply_with_approval(
		root.path(),
		EditMode::Hashline,
		&args,
		store.clone(),
		&Allow::default(),
		&CancelToken::default(),
	)?;
	assert_eq!(fs::read(&path)?, b"new\n");
	assert_eq!(store.head(&path).unwrap().text.as_ref(), "new\n");
	Ok(())
}
#[test]
fn synchronous_native_bridge_works_inside_existing_executor() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::write(root.path().join("note.txt"), "old\n")?;
	futures::executor::block_on(async {
		apply_with_approval(
			root.path(),
			EditMode::Replace,
			&json!({"path":"note.txt","old_string":"old","new_string":"new"}).to_string(),
			EditStore::default(),
			&Allow::default(),
			&CancelToken::default(),
		)
	})?;
	assert_eq!(fs::read(root.path().join("note.txt"))?, b"new\n");
	Ok(())
}
#[test]
fn occupied_move_destination_rejects_the_complete_set_before_writes() -> Result<()> {
	let root = tempfile::tempdir()?;
	for (name, text) in
		[("first.txt", "old\n"), ("from.txt", "source\n"), ("to.txt", "destination\n")]
	{
		fs::write(root.path().join(name), text)?;
	}
	let input = "*** Begin Patch\n*** Update File: first.txt\n@@\n-old\n+new\n*** Update File: \
	             from.txt\n*** Move to: to.txt\n@@\n-source\n+changed\n*** End Patch";
	assert!(
		apply_with_approval(
			root.path(),
			EditMode::ApplyPatch,
			&json!({"input":input}).to_string(),
			EditStore::default(),
			&Allow::default(),
			&CancelToken::default()
		)
		.is_err()
	);
	assert_eq!(fs::read(root.path().join("first.txt"))?, b"old\n");
	assert_eq!(fs::read(root.path().join("to.txt"))?, b"destination\n");
	assert_eq!(fs::read(root.path().join("from.txt"))?, b"source\n");
	Ok(())
}

#[test]
fn too_many_declared_targets_publish_nothing() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut input = "*** Begin Patch\n".to_owned();
	for index in 0..33 {
		input.push_str(&format!("*** Add File: file-{index}.txt\n+created\n"));
	}
	input.push_str("*** End Patch");
	let error = apply_with_approval(
		root.path(),
		EditMode::ApplyPatch,
		&json!({"input":input}).to_string(),
		EditStore::default(),
		&Allow::default(),
		&CancelToken::default(),
	)
	.unwrap_err();
	assert!(error.to_string().contains("targets"));
	assert_eq!(fs::read_dir(root.path())?.count(), 0);
	Ok(())
}
