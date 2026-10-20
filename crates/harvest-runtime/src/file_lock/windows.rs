use std::{
	borrow::Cow,
	io,
	os::windows::io::{FromRawHandle, OwnedHandle},
	ptr,
};

use windows_sys::Win32::{
	Foundation::{ERROR_ALREADY_EXISTS, GetLastError, SetLastError},
	System::Threading::CreateMutexW,
};

/// Windows lease held by the exclusive lifetime of a named kernel object.
pub struct PlatformFileLock {
	handle: Option<OwnedHandle>,
}

pub fn try_acquire(path: &str) -> io::Result<Option<PlatformFileLock>> {
	let name = super::memory_lock_name(&path_identity(path));
	let wide_name: Vec<u16> = format!(r"Global\{name}")
		.encode_utf16()
		.chain(std::iter::once(0))
		.collect();

	// The name's existence, not thread-affine mutex ownership, is the lease.
	// A handle can therefore be released from a different native worker thread.
	// Contenders see ERROR_ALREADY_EXISTS and immediately close their handle.
	// SAFETY: SetLastError(0) safely resets the thread-local error status before
	// the call.
	unsafe { SetLastError(0) };
	// SAFETY: the attributes pointer is null, and `wide_name` is a live,
	// NUL-terminated UTF-16 string for the duration of the call.
	let raw_handle = unsafe { CreateMutexW(ptr::null(), 0, wide_name.as_ptr()) };
	if raw_handle.is_null() {
		return Err(io::Error::last_os_error());
	}
	// SAFETY: `CreateMutexW` returned a fresh owned handle. `OwnedHandle` closes
	// it exactly once on every return path.
	let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle) };
	// SAFETY: this immediately observes the last-error value set by
	// `CreateMutexW`; no intervening system call can overwrite it.
	if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
		return Ok(None);
	}
	Ok(Some(PlatformFileLock { handle: Some(handle) }))
}

// Rust canonical paths use verbatim prefixes while JS hosts can return ordinary
// drive/UNC paths. Their spelling must not create independent kernel owners.
fn path_identity(path: &str) -> Cow<'_, str> {
	let Some(rest) = path.strip_prefix(r"\\?\") else {
		return Cow::Borrowed(path);
	};
	if rest
		.get(..4)
		.is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
	{
		return Cow::Owned(format!(r"\\{}", &rest[4..]));
	}
	if rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
		&& rest.as_bytes().get(1) == Some(&b':')
		&& rest.as_bytes().get(2) == Some(&b'\\')
	{
		return Cow::Borrowed(rest);
	}
	Cow::Borrowed(path)
}

impl PlatformFileLock {
	#[allow(clippy::unnecessary_wraps, reason = "uniform cross-platform interface")]
	pub fn release(&mut self) -> io::Result<()> {
		drop(self.handle.take());
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use std::{
		thread,
		time::{SystemTime, UNIX_EPOCH},
	};

	use super::try_acquire;

	#[test]
	fn canonical_and_ordinary_windows_paths_cannot_create_separate_owners() {
		let nonce = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("clock")
			.as_nanos();
		for plain in [
			format!(r"c:\harvest-{}-{nonce}\session.lock", std::process::id()),
			format!(r"\\server\share\harvest-{}-{nonce}\session.lock", std::process::id()),
		] {
			let extended = if let Some(rest) = plain.strip_prefix(r"\\") {
				format!(r"\\?\UNC\{rest}")
			} else {
				format!(r"\\?\{plain}")
			};
			let mut legacy = try_acquire(&plain)
				.expect("legacy owner")
				.expect("exclusive");
			assert!(
				try_acquire(&extended)
					.expect("canonical contender")
					.is_none()
			);
			legacy.release().expect("handoff");
			let owner = try_acquire(&extended)
				.expect("canonical owner")
				.expect("exclusive");
			assert!(try_acquire(&plain).expect("legacy contender").is_none());
			drop(owner);
		}
	}

	#[test]
	fn releases_from_a_different_worker_thread() {
		let nonce = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("system clock")
			.as_nanos();
		let path = format!("oauth-lock-thread-{}-{nonce}", std::process::id());
		let mut lease = try_acquire(&path)
			.expect("acquire lease")
			.expect("exclusive lease");
		assert!(try_acquire(&path).expect("contend").is_none());
		thread::spawn(move || lease.release().expect("release on another worker"))
			.join()
			.expect("worker completed");
		let mut successor = try_acquire(&path)
			.expect("acquire successor")
			.expect("released lease");
		successor.release().expect("release successor");
	}
}
