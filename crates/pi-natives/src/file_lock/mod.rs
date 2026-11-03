//! Cross-process advisory locks backed by platform ownership primitives.
//!
//! Linux uses abstract Unix sockets and Windows uses named mutexes, so neither
//! platform leaves a filesystem artifact. Other Unix platforms use `flock(2)`
//! on a persistent sidecar because they lack a process-owned in-memory name
//! registry with automatic crash recovery.

use std::{io, path::Path};

use harvest_runtime::file_lock::FileLock as ResourceLock;
use napi::JsString;
use napi_derive::napi;

use crate::js;

/// Process-owned cross-platform advisory lock.
///
/// `tryAcquire()` is non-blocking; its returned handle reports whether it won
/// through `acquired`. Ownership ends on `release()`, garbage collection, or
/// process exit; `release()` is idempotent.
#[napi(js_name = "FileLock")]
pub struct FileLock {
	inner: ResourceLock,
}

#[napi]
impl FileLock {
	/// Try to acquire a lock directly from native Rust code.
	///
	/// This is crate-only so native subsystems share the same kernel-backed
	/// ownership primitive without exposing another JavaScript API.
	pub(crate) fn try_acquire_path(path: &Path) -> io::Result<Self> {
		ResourceLock::try_acquire_path(path).map(|inner| Self { inner })
	}

	/// Try to acquire `path` without blocking.
	#[napi(factory)]
	pub fn try_acquire(path: JsString) -> napi::Result<Self> {
		let path = js::utf8(path)?;
		Self::try_acquire_path(Path::new(&*path)).map_err(|error| {
			napi::Error::from_reason(format!(
				"Failed to acquire native file lock for {}: {error}",
				&*path
			))
		})
	}

	/// Whether this handle owns the requested lock.
	#[napi(getter)]
	#[allow(clippy::missing_const_for_fn, reason = "napi method signature")]
	pub fn acquired(&self) -> bool {
		self.is_acquired()
	}

	/// Whether this native handle owns the requested lock.
	pub(crate) const fn is_acquired(&self) -> bool {
		self.inner.is_acquired()
	}

	/// Release this handle's ownership without affecting a successor.
	#[napi]
	pub fn release(&mut self) -> napi::Result<()> {
		self.release_native().map_err(|error| {
			napi::Error::from_reason(format!("Failed to release native file lock: {error}"))
		})
	}

	/// Release native ownership while preserving the handle on failure.
	pub(crate) fn release_native(&mut self) -> io::Result<()> {
		self.inner.release()
	}
}
