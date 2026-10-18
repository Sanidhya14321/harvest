#[cfg(unix)]
use std::fs;
use std::path::Path;

use harvest_runtime::{JailedFs, read_bounded};

#[cfg(windows)]
#[test]
fn ordinary_absolute_windows_paths_share_the_canonical_jail_and_owner() {
	let root = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let ordinary = root.path().join("nested/session.jsonl");
	let relative = Path::new("nested/session.jsonl");
	jail.atomic_write(&ordinary, b"session").unwrap();
	assert_eq!(jail.read(relative, 100).unwrap(), b"session");
	assert_eq!(jail.resource_path(&ordinary).unwrap(), jail.resource_path(relative).unwrap());
	let owner = jail.resource_lock(&ordinary).unwrap();
	assert!(jail.resource_lock(relative).is_err());
	drop(owner);
	assert!(jail.resource_lock(relative).unwrap().is_acquired());
	assert!(
		jail
			.atomic_write(&outside.path().join("session.jsonl"), b"escape")
			.is_err()
	);
	assert!(!outside.path().join("session.jsonl").exists());
}

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

#[cfg(unix)]
#[test]
fn newly_appended_session_logs_are_private_even_with_a_permissive_umask() {
	use std::{os::unix::fs::PermissionsExt, process::Command};

	const CHILD_MARKER: &str = "HARVEST_PRIVATE_APPEND_TEST_CHILD";
	if std::env::var_os(CHILD_MARKER).is_none() {
		// Apply umask in a dedicated process; the parallel test runner must not
		// inherit a process-wide mutation from this contract test.
		let status = Command::new("/bin/sh")
			.args(["-c", "umask 0; exec \"$@\"", "harvest-private-append"])
			.arg(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"newly_appended_session_logs_are_private_even_with_a_permissive_umask",
				"--nocapture",
			])
			.env(CHILD_MARKER, "1")
			.status()
			.unwrap();
		assert!(status.success(), "private append contract failed in isolated child");
		return;
	}

	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let path = Path::new("sessions/private.jsonl");
	jail.append(path, b"private session data\n", 100).unwrap();
	let metadata = fs::metadata(root.path().join(path)).unwrap();
	assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
	assert_eq!(jail.read(path, 100).unwrap(), b"private session data\n");
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

#[test]
fn legacy_and_rust_session_owners_contend_and_release_without_stale_handles() {
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let path = Path::new("new.jsonl");
	let identity = root.path().canonicalize().unwrap().join(path);
	#[cfg(target_os = "windows")]
	let identity = identity.to_str().unwrap().to_lowercase();
	#[cfg(not(target_os = "windows"))]
	let identity = identity.display().to_string();
	let mut legacy = harvest_runtime::file_lock::FileLock::try_acquire_path(Path::new(&format!(
		"{identity}.lock"
	)))
	.unwrap();
	assert!(legacy.is_acquired());
	assert!(jail.resource_lock(path).is_err());
	legacy.release().unwrap();
	let owner = jail.resource_lock(path).unwrap();
	legacy.release().unwrap();
	assert!(jail.resource_lock(path).is_err());
	drop(owner);
	let successor = jail.resource_lock(path).unwrap();
	assert!(successor.is_acquired());
}

#[cfg(unix)]
#[test]
fn missing_session_leaves_under_symlinked_ancestors_share_one_owner() {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir(root.path().join("real")).unwrap();
	std::os::unix::fs::symlink("real", root.path().join("alias")).unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let alias = Path::new("alias/deep/new.jsonl");
	let real = Path::new("real/deep/new.jsonl");
	assert_eq!(jail.resource_path(alias).unwrap(), jail.resource_path(real).unwrap());
	let owner = jail.resource_lock(alias).unwrap();
	assert!(jail.resource_lock(real).is_err());
	drop(owner);
	assert!(jail.resource_lock(real).unwrap().is_acquired());
	let outside = tempfile::tempdir().unwrap();
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
	assert!(jail.resource_lock(Path::new("escape/new.jsonl")).is_err());
	assert!(!outside.path().join("new.jsonl.lock").exists());
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
