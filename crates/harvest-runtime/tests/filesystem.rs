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

#[test]
fn atomic_create_publishes_complete_bytes_and_never_overwrites_existing_entries() {
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let path = Path::new("imports/session.jsonl");
	jail.atomic_create(path, b"complete\n").unwrap();
	assert!(jail.atomic_create(path, b"replacement\n").is_err());
	assert_eq!(jail.read(path, 100).unwrap(), b"complete\n");
	let entries = jail.entries(Path::new("imports"), 10).unwrap();
	assert_eq!(
		entries
			.iter()
			.map(|entry| entry.name.as_str())
			.collect::<Vec<_>>(),
		["session.jsonl"]
	);
	assert!(
		jail
			.atomic_create(Path::new("../escape"), b"outside")
			.is_err()
	);
}

#[cfg(unix)]
#[test]
fn atomic_create_cannot_escape_through_a_symlinked_parent_or_replace_a_symlink() {
	let root = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	std::fs::write(outside.path().join("target"), b"original").unwrap();
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
	std::os::unix::fs::symlink(outside.path().join("target"), root.path().join("alias")).unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	assert!(
		jail
			.atomic_create(Path::new("escape/new/deep"), b"bad")
			.is_err()
	);
	assert!(jail.atomic_create(Path::new("alias"), b"bad").is_err());
	assert_eq!(std::fs::read(outside.path().join("target")).unwrap(), b"original");
	assert!(!outside.path().join("new").exists());
}

#[test]
fn concurrent_atomic_creators_publish_one_winner_without_overwriting_it() {
	let root = tempfile::tempdir().unwrap();
	let barrier = std::sync::Barrier::new(2);
	let results = std::thread::scope(|scope| {
		let first = scope.spawn(|| {
			let jail = JailedFs::open(root.path()).unwrap();
			barrier.wait();
			jail.atomic_create(Path::new("winner"), b"first").is_ok()
		});
		let second = scope.spawn(|| {
			let jail = JailedFs::open(root.path()).unwrap();
			barrier.wait();
			jail.atomic_create(Path::new("winner"), b"second").is_ok()
		});
		(first.join().unwrap(), second.join().unwrap())
	});
	assert_ne!(results.0, results.1);
	assert_eq!(
		fs::read(root.path().join("winner")).unwrap(),
		if results.0 {
			b"first".as_slice()
		} else {
			b"second".as_slice()
		}
	);
	assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn file_reads_optional_reads_and_hashes_reject_fifos_without_waiting_for_a_writer() {
	use std::{
		ffi::CString,
		os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
		sync::mpsc,
		thread,
		time::Duration,
	};
	let root = tempfile::tempdir().unwrap();
	let path = root.path().join("pipe");
	let encoded = CString::new(path.as_os_str().as_bytes()).unwrap();
	// SAFETY: encoded owns a NUL-terminated path for the duration of mkfifo.
	assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);
	for operation in 0..3 {
		let jail = JailedFs::open(root.path()).unwrap();
		let (sender, receiver) = mpsc::channel();
		let worker = thread::spawn(move || {
			let rejected = match operation {
				0 => jail.read(Path::new("pipe"), 10).is_err(),
				1 => jail.read_optional(Path::new("pipe"), 10).is_err(),
				_ => jail.hash_file(Path::new("pipe"), 10).is_err(),
			};
			sender.send(rejected).unwrap();
		});
		let result = receiver.recv_timeout(Duration::from_secs(2));
		// Unblock a regressed blocking open before failing, so the test itself
		// cannot leave a stuck worker behind.
		let release = result.is_err().then(|| {
			fs::OpenOptions::new()
				.read(true)
				.write(true)
				.custom_flags(libc::O_NONBLOCK)
				.open(&path)
				.unwrap()
		});
		worker.join().unwrap();
		drop(release);
		assert_eq!(result, Ok(true));
	}
}
