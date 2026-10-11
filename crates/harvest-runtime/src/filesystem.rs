//! Capability-relative I/O: no check-then-reopen on an ambient lexical path.

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
	fs::File,
	io::Write,
	path::{Component, Path, PathBuf},
	sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::{
	ambient_authority,
	fs::{Dir, OpenOptions},
};
use sha2::{Digest, Sha256};

use crate::read_bounded;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct JailedFs {
	root:      PathBuf,
	directory: Dir,
}

/// A lock is tied to the authorized open file, not a pathname reopened later.
#[derive(Debug)]
pub struct RootLock {
	_file: File,
}

#[derive(Debug, Clone)]
pub struct DirectoryEntry {
	pub name:         String,
	pub is_directory: bool,
	pub is_file:      bool,
	pub is_symlink:   bool,
	pub size:         u64,
}

impl JailedFs {
	pub fn open(root: &Path) -> Result<Self> {
		let root = root
			.canonicalize()
			.context("failed to resolve filesystem root")?;
		let directory = Dir::open_ambient_dir(&root, ambient_authority())?;
		Ok(Self { root, directory })
	}

	/// Create an explicitly authorized root through its deepest existing parent.
	pub fn open_or_create(root: &Path) -> Result<Self> {
		if root.is_dir() {
			return Self::open(root);
		}
		let absolute = if root.is_absolute() {
			root.to_path_buf()
		} else {
			std::env::current_dir()?.join(root)
		};
		let mut ancestor = absolute.as_path();
		loop {
			match std::fs::symlink_metadata(ancestor) {
				Ok(meta) => {
					if meta.file_type().is_symlink() || !meta.is_dir() {
						bail!("root ancestor must be a real directory");
					}
					break;
				},
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
					ancestor = ancestor.parent().context("root has no existing ancestor")?;
				},
				Err(error) => return Err(error.into()),
			}
		}
		let base = Self::open(ancestor)?;
		let suffix = absolute.strip_prefix(ancestor)?;
		let relative = base.relative(suffix)?;
		base.directory.create_dir_all(&relative)?;
		let directory = base.directory.open_dir(&relative)?;
		let root = base.root.join(base.directory.canonicalize(&relative)?);
		Ok(Self { root, directory })
	}

	pub fn root(&self) -> &Path {
		&self.root
	}

	/// Normalize paths lexically; the directory capability enforces real access.
	pub fn relative(&self, path: &Path) -> Result<PathBuf> {
		let path = if path.is_absolute() {
			path
				.strip_prefix(&self.root)
				.context("path escapes the authorized root")?
		} else {
			path
		};
		let mut result = PathBuf::new();
		for component in path.components() {
			match component {
				Component::Normal(name) => result.push(name),
				Component::CurDir => {},
				Component::ParentDir if result.pop() => {},
				_ => bail!("path escapes the authorized root"),
			}
		}
		Ok(result)
	}

	fn parent(&self, path: &Path, create: bool) -> Result<(Dir, PathBuf)> {
		let relative = self.relative(path)?;
		let name = relative
			.file_name()
			.context("operation requires a file, not the root")?;
		let parent = relative.parent().unwrap_or_else(|| Path::new(""));
		if create && !parent.as_os_str().is_empty() {
			self.directory.create_dir_all(parent)?;
		}
		let directory = if parent.as_os_str().is_empty() {
			self.directory.try_clone()?
		} else {
			self.directory.open_dir(parent)?
		};
		Ok((directory, PathBuf::from(name)))
	}

	pub fn read(&self, path: &Path, limit: usize) -> Result<Vec<u8>> {
		let relative = self.relative(path)?;
		let file = self
			.directory
			.open(&relative)
			.with_context(|| format!("failed to read {}", relative.display()))?;
		if !file.metadata()?.is_file() {
			bail!("expected a regular file");
		}
		read_bounded(file, limit)
	}

	pub fn read_optional(&self, path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
		let relative = self.relative(path)?;
		match self.directory.open(&relative) {
			Ok(file) => {
				if !file.metadata()?.is_file() {
					bail!("expected a regular file");
				}
				Ok(Some(read_bounded(file, limit)?))
			},
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
			Err(error) => Err(error.into()),
		}
	}

	/// Pin the parent, stage a fresh inode and replace atomically within it.
	pub fn atomic_write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
		let (directory, name) = self.parent(path, true)?;
		let old_permissions = match directory.symlink_metadata(&name) {
			Ok(metadata) => {
				if metadata.file_type().is_symlink() || !metadata.is_file() {
					bail!("refusing to replace a symlink or non-file");
				}
				Some(metadata.permissions())
			},
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
			Err(error) => return Err(error.into()),
		};
		let temp = PathBuf::from(format!(
			".harvest-{}-{}.tmp",
			std::process::id(),
			TEMP_ID.fetch_add(1, Ordering::Relaxed)
		));
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		options.mode(0o600);
		let mut file = directory.open_with(&temp, &options)?;
		let result = (|| -> Result<()> {
			file.write_all(bytes)?;
			if let Some(permissions) = old_permissions {
				file.set_permissions(permissions)?;
			}
			file.sync_all()?;
			drop(file);
			directory.rename(&temp, &directory, &name)?;
			#[cfg(unix)]
			directory.open(".")?.sync_all()?;
			Ok(())
		})();
		if result.is_err() {
			let _cleanup = directory.remove_file(&temp);
		}
		result
	}

	pub fn append(&self, path: &Path, bytes: &[u8], limit: usize) -> Result<()> {
		let (directory, name) = self.parent(path, true)?;
		if directory
			.symlink_metadata(&name)
			.is_ok_and(|meta| meta.file_type().is_symlink())
		{
			bail!("refusing to append through a symlink");
		}
		let mut options = OpenOptions::new();
		options.create(true).append(true);
		let mut file = directory.open_with(&name, &options)?.into_std();
		file.lock()?;
		let metadata = file.metadata()?;
		if !metadata.is_file()
			|| metadata.len().saturating_add(u64::try_from(bytes.len())?) > u64::try_from(limit)?
		{
			bail!("append exceeds the {limit}-byte limit");
		}
		#[cfg(unix)]
		{
			if metadata.nlink() != 1 {
				bail!("refusing to append to a multiply linked file");
			}
		}
		let length = metadata.len();
		if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
			file.set_len(length)?;
			return Err(error.into());
		}
		Ok(())
	}

	pub fn lock(&self, path: &Path) -> Result<RootLock> {
		let (directory, name) = self.parent(path, true)?;
		if directory
			.symlink_metadata(&name)
			.is_ok_and(|meta| meta.file_type().is_symlink())
		{
			bail!("lock path is a symlink");
		}
		let mut options = OpenOptions::new();
		options.read(true).write(true).create(true);
		let file = directory.open_with(&name, &options)?.into_std();
		if !file.metadata()?.is_file() {
			bail!("lock path is not a regular file");
		}
		file.try_lock().context("another writer owns this root")?;
		Ok(RootLock { _file: file })
	}

	/// Hold a shared snapshot lock without creating anything during reads.
	pub fn lock_existing(&self, path: &Path) -> Result<Option<RootLock>> {
		let relative = self.relative(path)?;
		match self.directory.symlink_metadata(&relative) {
			Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {},
			Ok(_) => bail!("snapshot lock is not a regular file"),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
			Err(error) => return Err(error.into()),
		}
		let file = self.directory.open(&relative)?.into_std();
		file
			.try_lock_shared()
			.context("another writer owns this root")?;
		Ok(Some(RootLock { _file: file }))
	}

	pub fn remove(&self, path: &Path) -> Result<()> {
		let (directory, name) = self.parent(path, false)?;
		if directory.symlink_metadata(&name)?.file_type().is_symlink() {
			bail!("refusing to remove a symlink");
		}
		directory.remove_file(name)?;
		Ok(())
	}

	pub fn entries(&self, path: &Path, limit: usize) -> Result<Vec<DirectoryEntry>> {
		let path = self.relative(path)?;
		let mut entries = Vec::new();
		let iter = if path.as_os_str().is_empty() {
			self.directory.entries()?
		} else {
			self.directory.read_dir(path)?
		};
		for entry in iter.take(limit.saturating_add(1)) {
			let entry = entry?;
			let file_type = entry.file_type()?;
			let size = if file_type.is_file() {
				entry.metadata()?.len()
			} else {
				0
			};
			if let Some(name) = entry.file_name().to_str() {
				entries.push(DirectoryEntry {
					name: name.to_owned(),
					is_directory: file_type.is_dir(),
					is_file: file_type.is_file(),
					is_symlink: file_type.is_symlink(),
					size,
				});
			}
		}
		if entries.len() > limit {
			bail!("directory exceeds the {limit}-entry traversal limit");
		}
		entries.sort_by(|a, b| a.name.cmp(&b.name));
		Ok(entries)
	}

	pub fn exists(&self, path: &Path) -> Result<bool> {
		let path = self.relative(path)?;
		if path.as_os_str().is_empty() {
			return Ok(true);
		}
		match self.directory.symlink_metadata(path) {
			Ok(metadata) => Ok(!metadata.file_type().is_symlink()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
			Err(error) => Err(error.into()),
		}
	}
}

/// Stable content fingerprints; never use process-dependent runtime hashes.
pub fn content_hash(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}
