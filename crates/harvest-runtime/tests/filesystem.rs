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
fn file_consumers_reject_fifos_without_waiting_for_a_writer() {
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
	for operation in 0..4 {
		let jail = JailedFs::open(root.path()).unwrap();
		let (sender, receiver) = mpsc::channel();
		let worker = thread::spawn(move || {
			let rejected = match operation {
				0 => jail.read(Path::new("pipe"), 10).is_err(),
				1 => jail.read_optional(Path::new("pipe"), 10).is_err(),
				2 => jail.hash_file(Path::new("pipe"), 10).is_err(),
				_ => jail.open_read(Path::new("pipe"), 10).is_err(),
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

#[test]
fn child_descriptors_are_read_only_and_reject_oversized_or_directory_inputs() {
	use std::io::{Read, Write};
	let root = tempfile::tempdir().unwrap();
	fs::write(root.path().join("input"), b"original").unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	assert!(jail.open_read(Path::new("input"), 7).is_err());
	assert!(jail.open_read(Path::new("."), 100).is_err());
	let mut input = jail.open_read(Path::new("input"), 8).unwrap();
	assert!(input.write_all(b"changed").is_err());
	let mut content = String::new();
	input.read_to_string(&mut content).unwrap();
	assert_eq!(content, "original");
}

#[cfg(unix)]
#[test]
fn authorized_child_descriptor_stays_pinned_after_a_path_is_replaced() {
	use std::io::Read;
	let root = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	fs::write(root.path().join("input"), b"authorized").unwrap();
	fs::write(outside.path().join("secret"), b"outside").unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let mut input = jail.open_read(Path::new("input"), 100).unwrap();
	fs::rename(root.path().join("input"), root.path().join("moved")).unwrap();
	std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("input")).unwrap();
	assert!(jail.open_read(Path::new("input"), 100).is_err());
	let mut content = String::new();
	input.read_to_string(&mut content).unwrap();
	assert_eq!(content, "authorized");
}

#[test]
fn streamed_replacement_keeps_the_old_file_on_reader_failure_or_limit_overflow() {
	struct Fails {
		emitted: bool,
	}
	impl std::io::Read for Fails {
		fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
			if bytes.is_empty() {
				return Ok(0);
			}
			if self.emitted {
				return Err(std::io::ErrorKind::Other.into());
			}
			self.emitted = true;
			bytes[0] = b'x';
			Ok(1)
		}
	}
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let path = Path::new("output");
	jail.atomic_write(path, b"original").unwrap();
	assert!(
		jail
			.atomic_write_from(path, Fails { emitted: false }, 100)
			.is_err()
	);
	assert_eq!(jail.read(path, 100).unwrap(), b"original");
	assert!(jail.atomic_write_from(path, &b"too large"[..], 4).is_err());
	assert_eq!(jail.read(path, 100).unwrap(), b"original");
	assert_eq!(jail.atomic_write_from(path, &b"complete"[..], 8).unwrap(), 8);
	assert_eq!(jail.read(path, 100).unwrap(), b"complete");
}

#[test]
fn a_panicking_stream_cannot_leave_partial_payload_files_or_replace_the_original() {
	struct Panics {
		emitted: bool,
	}
	impl std::io::Read for Panics {
		fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
			if bytes.is_empty() {
				return Ok(0);
			}
			assert!(!self.emitted, "reader failed");
			self.emitted = true;
			let count = bytes.len().min(7);
			bytes[..count].copy_from_slice(&b"private"[..count]);
			Ok(count)
		}
	}
	let root = tempfile::tempdir().unwrap();
	let jail = JailedFs::open(root.path()).unwrap();
	let path = Path::new("output");
	jail.atomic_write(path, b"original").unwrap();
	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		jail.atomic_write_from(path, Panics { emitted: false }, 100)
	}));
	assert!(result.is_err());
	assert_eq!(jail.read(path, 100).unwrap(), b"original");
	for entry in fs::read_dir(root.path()).unwrap() {
		let entry = entry.unwrap();
		if entry.file_type().unwrap().is_file() {
			assert!(
				!fs::read(entry.path())
					.unwrap()
					.windows(7)
					.any(|window| window == b"private")
			);
		}
	}
}

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

#[cfg(unix)]
#[test]
fn explicitly_opened_root_alias_remains_bound_to_its_original_directory() {
	use std::os::unix::fs::symlink;
	let holder = tempfile::tempdir().unwrap();
	let actual = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	let alias = holder.path().join("alias");
	symlink(actual.path(), &alias).unwrap();
	let jail = JailedFs::open(&alias).unwrap();
	jail
		.atomic_write(&alias.join("nested/session.jsonl"), b"original")
		.unwrap();
	assert_eq!(jail.read(Path::new("nested/session.jsonl"), 100).unwrap(), b"original");
	std::fs::remove_file(&alias).unwrap();
	symlink(outside.path(), &alias).unwrap();
	jail
		.atomic_write(&alias.join("nested/session.jsonl"), b"still authorized")
		.unwrap();
	assert_eq!(
		std::fs::read(actual.path().join("nested/session.jsonl")).unwrap(),
		b"still authorized"
	);
	assert!(!outside.path().join("nested/session.jsonl").exists());
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
