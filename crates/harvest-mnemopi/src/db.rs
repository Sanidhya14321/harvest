//! Connection setup mirroring `db.ts` (0700 dir, 0600 file, FK + WAL + busy
//! timeout).

use std::path::Path;

use harvest_runtime::JailedFs;
use rusqlite::{Connection, OpenFlags, limits::Limit};

use crate::Result;

const MIN_PAGE: i64 = 512;
const MAX_PAGE: i64 = 65536;

fn valid_page_size(n: i64) -> bool {
	(MIN_PAGE..=MAX_PAGE).contains(&n) && (n & (n - 1)) == 0
}

fn configured_page_size() -> Option<i64> {
	let raw = std::env::var("MNEMOPI_DB_PAGE_SIZE").ok()?;
	let n: i64 = raw.trim().parse().ok()?;
	valid_page_size(n).then_some(n)
}

#[cfg(unix)]
fn chmod(path: &Path, mode: u32) -> Result<()> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
	Ok(())
}
#[cfg(not(unix))]
fn chmod(_path: &Path, _mode: u32) -> Result<()> {
	Ok(())
}

/// Open (creating) a Mnemopi database at `path`, or in-memory when `None`.
pub fn open_database(path: Option<&Path>) -> Result<Connection> {
	let conn = match path {
		None => Connection::open_in_memory()?,
		Some(p) => {
			// The database root is an explicit host capability, never a tool-call
			// argument. Resolve it through the shared filesystem owner before any
			// SQLite mutation; reopen only the resolved leaf, with NOFOLLOW.
			let dir = p
				.parent()
				.filter(|d| !d.as_os_str().is_empty())
				.unwrap_or_else(|| Path::new("."));
			let existed = dir.is_dir();
			let root = JailedFs::open_or_create(dir)
				.map_err(|e| crate::MnemopiError::Invalid(e.to_string()))?;
			if !existed {
				chmod(root.root(), 0o700)?;
			}
			let name = p
				.file_name()
				.ok_or_else(|| crate::MnemopiError::Invalid("database needs a file name".into()))?;
			let path = root
				.resource_path(Path::new(name))
				.map_err(|e| crate::MnemopiError::Invalid(e.to_string()))?;
			if root
				.entry_exists(Path::new(name))
				.map_err(|e| crate::MnemopiError::Invalid(e.to_string()))?
				&& !root
					.file_type(Path::new(name))
					.map_err(|e| crate::MnemopiError::Invalid(e.to_string()))?
					.is_file()
			{
				return Err(crate::MnemopiError::Invalid("database must be a regular file".into()));
			}
			let c = Connection::open_with_flags(
				&path,
				OpenFlags::SQLITE_OPEN_READ_WRITE
					| OpenFlags::SQLITE_OPEN_CREATE
					| OpenFlags::SQLITE_OPEN_NO_MUTEX
					| OpenFlags::SQLITE_OPEN_NOFOLLOW,
			)?;
			chmod(&path, 0o600)?;
			c
		},
	};
	conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, 4 * 1024 * 1024)?;
	conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024)?;
	conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
	if path.is_some() {
		if let Some(size) = configured_page_size() {
			conn.execute_batch(&format!("PRAGMA page_size={size}"))?;
		}
		let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
	}
	Ok(conn)
}

/// Run `f` inside a deferred transaction (or inline when already in one).
pub fn transaction<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
	if !conn.is_autocommit() {
		return f();
	}
	conn.execute_batch("BEGIN DEFERRED")?;
	match f() {
		Ok(v) => match conn.execute_batch("COMMIT") {
			Ok(()) => Ok(v),
			Err(error) => {
				let _ = conn.execute_batch("ROLLBACK");
				Err(error.into())
			},
		},
		Err(e) => {
			let _ = conn.execute_batch("ROLLBACK");
			Err(e)
		},
	}
}
