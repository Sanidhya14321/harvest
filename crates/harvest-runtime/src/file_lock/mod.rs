//! Cross-process advisory locks backed by platform ownership primitives.
//!
//! Linux uses abstract Unix sockets and Windows uses named mutexes, so neither
//! platform leaves a filesystem artifact. Other Unix platforms use `flock(2)`
//! on a persistent sidecar because they lack a process-owned in-memory name
//! registry with automatic crash recovery.

use std::{io, path::Path};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(unix, not(target_os = "linux")))]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(all(unix, not(target_os = "linux")))]
use unix as platform;
#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!("Harvest file locks require Unix or Windows");

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn memory_lock_name(path: &str) -> String {
	const HIGH_SEED: u64 = 0x4f4d_502d_4c4f_434b;
	const LOW_SEED: u64 = 0x5049_2d46_494c_454c;
	let bytes = path.as_bytes();
	let high = xxhash_rust::xxh64::xxh64(bytes, HIGH_SEED);
	let low = xxhash_rust::xxh64::xxh64(bytes, LOW_SEED);
	format!("harvest-file-lock-{high:016x}{low:016x}")
}

/// Process-owned, nonblocking resource ownership shared by all application
/// hosts.
pub struct FileLock {
	inner: Option<platform::PlatformFileLock>,
}
impl FileLock {
	pub fn try_acquire_path(path: &Path) -> io::Result<Self> {
		let path = path.to_str().ok_or_else(|| {
			io::Error::new(io::ErrorKind::InvalidInput, "lock path is not valid UTF-8")
		})?;
		platform::try_acquire(path).map(|inner| Self { inner })
	}

	#[cfg(all(unix, not(target_os = "linux")))]
	pub(crate) fn try_acquire_file(file: std::fs::File) -> io::Result<Self> {
		platform::try_acquire_file(file).map(|inner| Self { inner })
	}

	pub const fn is_acquired(&self) -> bool {
		self.inner.is_some()
	}

	pub fn release(&mut self) -> io::Result<()> {
		let Some(mut inner) = self.inner.take() else {
			return Ok(());
		};
		if let Err(error) = inner.release() {
			self.inner = Some(inner);
			return Err(error);
		}
		Ok(())
	}
}
impl Drop for FileLock {
	fn drop(&mut self) {
		if let Some(inner) = self.inner.as_mut() {
			let _ = inner.release();
		}
	}
}
