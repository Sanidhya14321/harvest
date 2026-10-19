//! Shipping target identities shared by CI, package renderers and archives.
use std::{
	io::{Cursor, Write},
	path::Path,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, content_hash, read_bounded};
use serde::Serialize;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const BINARY_LIMIT: usize = 256 * 1024 * 1024;
const ARCHIVE_LIMIT: usize = 2 * BINARY_LIMIT + 8 * 1024 * 1024;
const LEGAL_FILES: &[&str] = &["LICENSE", "THIRD-PARTY-NOTICES.txt"];
pub const CHECKSUMS: &str = "SHA256SUMS.txt";

#[derive(Serialize)]
pub struct Target {
	pub os:           &'static str,
	pub target:       &'static str,
	pub artifact:     &'static str,
	pub binary:       &'static str,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub bazel_target: Option<&'static str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub deb_arch:     Option<&'static str>,
}

pub static TARGETS: &[Target] = &[
	Target {
		os:           "ubuntu-24.04",
		target:       "x86_64-unknown-linux-gnu",
		artifact:     "omp-linux-x64",
		binary:       "omp",
		bazel_target: Some("app-linux-x64"),
		deb_arch:     Some("amd64"),
	},
	Target {
		os:           "ubuntu-24.04-arm",
		target:       "aarch64-unknown-linux-gnu",
		artifact:     "omp-linux-arm64",
		binary:       "omp",
		bazel_target: Some("app-linux-arm64"),
		deb_arch:     Some("arm64"),
	},
	Target {
		os:           "ubuntu-24.04",
		target:       "x86_64-unknown-linux-musl",
		artifact:     "omp-linux-musl-x64",
		binary:       "omp",
		bazel_target: None,
		deb_arch:     None,
	},
	Target {
		os:           "ubuntu-24.04-arm",
		target:       "aarch64-unknown-linux-musl",
		artifact:     "omp-linux-musl-arm64",
		binary:       "omp",
		bazel_target: None,
		deb_arch:     None,
	},
	Target {
		os:           "macos-15-intel",
		target:       "x86_64-apple-darwin",
		artifact:     "omp-darwin-x64",
		binary:       "omp",
		bazel_target: None,
		deb_arch:     None,
	},
	Target {
		os:           "macos-15",
		target:       "aarch64-apple-darwin",
		artifact:     "omp-darwin-arm64",
		binary:       "omp",
		bazel_target: None,
		deb_arch:     None,
	},
	Target {
		os:           "windows-latest",
		target:       "x86_64-pc-windows-msvc",
		artifact:     "omp-windows-x64.exe",
		binary:       "omp.exe",
		bazel_target: None,
		deb_arch:     None,
	},
	Target {
		os:           "windows-11-arm",
		target:       "aarch64-pc-windows-msvc",
		artifact:     "omp-windows-arm64.exe",
		binary:       "omp.exe",
		bazel_target: None,
		deb_arch:     None,
	},
];

impl Target {
	pub fn archive(&self) -> String {
		format!("{}.zip", self.artifact.strip_suffix(".exe").unwrap_or(self.artifact))
	}

	fn alias(&self) -> &'static str {
		if self.binary == "omp.exe" {
			"harvest.exe"
		} else {
			"harvest"
		}
	}
}

pub fn target(name: &str) -> Result<&'static Target> {
	TARGETS
		.iter()
		.find(|target| target.target == name)
		.context("unsupported shipping target")
}

pub fn matrix() -> Result<String> {
	Ok(serde_json::to_string(&serde_json::json!({"include":TARGETS}))?)
}

/// Archive both real aliases and legal notices, with deterministic metadata.
pub fn archive(fs: &JailedFs, target: &Target, binary: &Path, output_dir: &Path) -> Result<()> {
	let output = fs.relative(&output_dir.join(target.archive()))?;
	if output == fs.relative(binary)? {
		bail!("archive output cannot replace the input binary")
	}
	let bytes = fs.read(binary, BINARY_LIMIT)?;
	if bytes.is_empty() {
		bail!("release binary cannot be empty")
	}
	let legal = LEGAL_FILES
		.iter()
		.map(|name| Ok((*name, fs.read(Path::new(name), 4 * 1024 * 1024)?)))
		.collect::<Result<Vec<_>>>()?;
	let entries = [target.binary, target.alias()]
		.into_iter()
		.map(|name| (name, bytes.as_slice(), 0o755))
		.chain(
			legal
				.iter()
				.map(|(name, content)| (*name, content.as_slice(), 0o644)),
		);
	let bytes = zip_bytes(entries)?;
	fs.atomic_write(&output, &bytes)
}

pub(crate) fn zip_bytes<'a>(
	entries: impl IntoIterator<Item = (&'a str, &'a [u8], u32)>,
) -> Result<Vec<u8>> {
	let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
	for (name, bytes, permissions) in entries {
		writer.start_file(
			name,
			SimpleFileOptions::default()
				.last_modified_time(zip::DateTime::default())
				.compression_method(CompressionMethod::Deflated)
				.unix_permissions(permissions),
		)?;
		writer.write_all(bytes)?;
	}
	let bytes = writer.finish()?.into_inner();
	if bytes.len() > ARCHIVE_LIMIT {
		bail!("release archive exceeds size limit")
	}
	Ok(bytes)
}

pub(crate) fn read_zip(
	fs: &JailedFs,
	path: &Path,
	limit: usize,
) -> Result<ZipArchive<Cursor<Vec<u8>>>> {
	let mut archive = ZipArchive::new(Cursor::new(fs.read(path, limit)?))?;
	archive.validate_header_consistency()?;
	Ok(archive)
}

/// Check every promised raw target and verify archive aliases against raw
/// bytes.
pub fn verify(fs: &JailedFs, directory: &Path) -> Result<()> {
	let legal = LEGAL_FILES
		.iter()
		.map(|name| Ok((*name, fs.read(Path::new(name), 4 * 1024 * 1024)?)))
		.collect::<Result<Vec<_>>>()?;
	for target in TARGETS {
		let raw = fs.read(&directory.join(target.artifact), BINARY_LIMIT)?;
		if raw.is_empty() {
			bail!("release binary cannot be empty");
		}
		let digest = content_hash(&raw);
		drop(raw);
		let mut archive = read_zip(fs, &directory.join(target.archive()), ARCHIVE_LIMIT)?;
		if archive.len() != 2 + LEGAL_FILES.len() {
			bail!("release archive has unexpected entries")
		}
		for name in [target.binary, target.alias()] {
			let file = archive.by_name(name)?;
			if !file.is_file() {
				bail!("release archive alias must be a regular file")
			}
			if file
				.unix_mode()
				.is_none_or(|mode| mode & 0o170777 != 0o100755)
			{
				bail!("release archive executable mode is incorrect")
			}
			if content_hash(&read_bounded(file, BINARY_LIMIT)?) != digest {
				bail!("release archive alias does not match the raw binary");
			}
		}
		for (name, expected) in &legal {
			let file = archive.by_name(name)?;
			if !file.is_file() {
				bail!("release archive legal notice must be a regular file")
			}
			if &read_bounded(file, 4 * 1024 * 1024)? != expected {
				bail!("release archive legal notice does not match source");
			}
		}
	}
	Ok(())
}
