use std::{fs, path::Path};

use harvest_runtime::{JailedFs, read_bounded};

#[test]
fn new_parent_creation_and_atomic_replacement_preserve_contents() {
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	jail
		.atomic_write(Path::new("new/deep/note.md"), b"first")
		.unwrap();
	jail
		.atomic_write(Path::new("new/deep/note.md"), b"second")
		.unwrap();
	assert_eq!(jail.read(Path::new("new/deep/note.md"), 10).unwrap(), b"second");
	assert!(
		jail
			.atomic_write(Path::new("../outside.md"), b"escape")
			.is_err()
	);
}

#[test]
fn oversized_reads_and_appends_fail_before_unbounded_allocation() {
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	jail
		.append(Path::new("session.jsonl"), b"first\n", 10)
		.unwrap();
	assert!(
		jail
			.append(Path::new("session.jsonl"), b"second\n", 10)
			.is_err()
	);
	assert_eq!(jail.read(Path::new("session.jsonl"), 10).unwrap(), b"first\n");
	assert!(read_bounded(&b"too large"[..], 3).is_err());
}

#[test]
fn second_writer_cannot_acquire_a_live_root_lock() {
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let held = jail.lock(Path::new("state/write.lock")).unwrap();
	assert!(jail.lock(Path::new("state/write.lock")).is_err());
	drop(held);
	let acquired = jail.lock(Path::new("state/write.lock")).unwrap();
	drop(acquired);
}

#[cfg(unix)]
#[test]
fn symlinked_parents_and_leaves_cannot_redirect_mutations() {
	let root = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	fs::write(outside.path().join("target"), "original").unwrap();
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
	std::os::unix::fs::symlink(outside.path().join("target"), root.path().join("leaf")).unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	assert!(
		jail
			.atomic_write(Path::new("escape/missing/file"), b"bad")
			.is_err()
	);
	assert!(jail.append(Path::new("leaf"), b"bad", 100).is_err());
	assert!(jail.remove(Path::new("leaf")).is_err());
	assert_eq!(fs::read_to_string(outside.path().join("target")).unwrap(), "original");
	assert!(!outside.path().join("missing").exists());
}

#[cfg(unix)]
#[test]
fn session_append_cannot_mutate_an_outside_inode_through_a_hard_link() {
	let root = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	std::fs::write(outside.path().join("outside"), "original").unwrap();
	std::fs::hard_link(outside.path().join("outside"), root.path().join("alias")).unwrap();
	let fs = JailedFs::open(root.path()).unwrap();
	assert!(
		fs.append(std::path::Path::new("alias"), b"changed", 100)
			.is_err()
	);
	assert_eq!(std::fs::read(outside.path().join("outside")).unwrap(), b"original");
}
