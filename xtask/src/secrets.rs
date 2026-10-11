//! Credential checks for tracked source, with bounded input and redacted
//! diagnostics.
use std::{
	io::{self, Write},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, read_bounded};
use regex::Regex;

const FILE_LIMIT: usize = 512 * 1024;
const TOTAL_LIMIT: usize = 64 * 1024 * 1024;
const MANIFEST_LIMIT: usize = 8 * 1024 * 1024;
const FINDING_LIMIT: usize = 256;
const SKIP_SUFFIXES: &[&str] =
	&[".png", ".jpg", ".jpeg", ".gif", ".ico", ".pdf", ".woff", ".woff2", ".zip"];

struct Rule {
	name:            &'static str,
	pattern:         Regex,
	placeholders_ok: bool,
}

/// A finding identifies its location and rule, never the credential itself.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
	pub path: PathBuf,
	pub line: usize,
	pub rule: &'static str,
}

pub struct Scanner {
	rules:       Vec<Rule>,
	placeholder: Regex,
}

impl Scanner {
	pub fn new() -> Result<Self> {
		let specifications = [
			("private key block", r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----", false),
			("github token", r"\bgh[pousr]_[A-Za-z0-9]{20,}", true),
			("github fine-grained token", r"\bgithub_pat_[A-Za-z0-9_]{20,}", true),
			("aws access key id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", true),
			("slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}", true),
			("google api key", r"\bAIza[0-9A-Za-z_-]{35}\b", true),
			("openai key", r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}", true),
			("anthropic key", r"\bsk-ant-[A-Za-z0-9_-]{20,}", true),
			("crates.io token", r"\bcio[A-Za-z0-9]{28,}\b", true),
			("npm token", r"\bnpm_[A-Za-z0-9]{30,}\b", true),
			("stripe key", r"\b[sr]k_live_[A-Za-z0-9]{20,}\b", true),
			(
				"generic secret assignment",
				r#"(?ix)\b(?:api[_-]?key|secret|password|passwd|token|private[_-]?key)\s*["']?\s*[:=]\s*["'][A-Za-z0-9+/_-]{24,}={0,2}["']"#,
				true,
			),
			("url with inline credentials", r"\b[a-z][a-z0-9+.-]*://[^/\s:@]+:[^/\s:@]+@", true),
		];
		let rules = specifications
			.into_iter()
			.map(|(name, pattern, placeholders_ok)| {
				Ok(Rule { name, pattern: Regex::new(pattern)?, placeholders_ok })
			})
			.collect::<Result<Vec<_>>>()?;
		Ok(Self {
			rules,
			placeholder: Regex::new(
				r"(?ix)XXXX|EXAMPLE|PLACEHOLDER|REDACTED|CHANGEME|DUMMY|\*{3}|<[A-Za-z_-]+>|\.\.\.|\$\{|\{\{|:(?:secret|password|passwd|pass|token|deploy-key)@",
			)?,
		})
	}

	pub fn scan(&self, text: &str, path: &Path) -> Result<Vec<Finding>> {
		if text.len() > FILE_LIMIT {
			bail!("source exceeds file limit");
		}
		if text.contains('\0') {
			return Ok(Vec::new());
		}
		let mut findings = Vec::new();
		for rule in &self.rules {
			for hit in rule.pattern.find_iter(text) {
				if rule.placeholders_ok && self.placeholder.is_match(hit.as_str()) {
					continue;
				}
				if findings.len() == FINDING_LIMIT {
					bail!("too many credential findings");
				}
				findings.push(Finding {
					path: path.to_owned(),
					line: text[..hit.start()]
						.chars()
						.filter(|character| *character == '\n')
						.count() + 1,
					rule: rule.name,
				});
			}
		}
		Ok(findings)
	}
}

/// Scan the NUL-delimited output of `git ls-files -z` relative to the checkout.
///
/// Unreadable, escaping and oversized files fail closed rather than
/// disappearing from the security check. Binary files retain the legacy
/// exclusion.
pub fn scan_files(fs: &JailedFs, manifest: &[u8]) -> Result<(usize, Vec<Finding>)> {
	if manifest.len() > MANIFEST_LIMIT {
		bail!("tracked-file manifest exceeds limit");
	}
	if !manifest.is_empty() && !manifest.ends_with(&[0]) {
		bail!("expected a NUL-delimited tracked-file manifest");
	}
	let scanner = Scanner::new()?;
	let mut scanned = 0;
	let mut total = 0usize;
	let mut findings = Vec::new();
	let mut listed = 0;
	for bytes in manifest
		.split(|byte| *byte == 0)
		.filter(|name| !name.is_empty())
	{
		listed += 1;
		if listed > 10_000 {
			bail!("too many tracked files");
		}
		#[cfg(unix)]
		let path = {
			use std::os::unix::ffi::OsStrExt;
			Path::new(std::ffi::OsStr::from_bytes(bytes)).to_owned()
		};
		#[cfg(not(unix))]
		let path = PathBuf::from(std::str::from_utf8(bytes).context("invalid tracked filename")?);
		// Validate even excluded names so an untrusted manifest cannot hide an escape.
		fs.relative(&path)?;
		if path == Path::new("rust-agent/scripts/check_secrets.py") {
			continue;
		}
		if SKIP_SUFFIXES
			.iter()
			.any(|suffix| path.to_string_lossy().ends_with(suffix))
		{
			continue;
		}
		let content = fs
			.read(&path, FILE_LIMIT)
			.with_context(|| format!("cannot scan {}", path.display().to_string().escape_debug()))?;
		total = total
			.checked_add(content.len())
			.context("source size overflow")?;
		if total > TOTAL_LIMIT {
			bail!("source aggregate exceeds limit");
		}
		let Ok(text) = std::str::from_utf8(&content) else {
			continue;
		};
		scanned += 1;
		findings.extend(scanner.scan(text, &path)?);
		if findings.len() > FINDING_LIMIT {
			bail!("too many credential findings");
		}
	}
	Ok((scanned, findings))
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	if arguments == ["--help"] {
		println!(
			"git ls-files -z -- rust-agent crates/harvest-runtime crates/harvest-brain xtask | cargo \
			 xtask secrets --files-stdin"
		);
		return Ok(());
	}
	if arguments != ["--files-stdin"] {
		bail!("use secrets --files-stdin with git ls-files -z from the checkout root");
	}
	let manifest = read_bounded(io::stdin().lock(), MANIFEST_LIMIT)?;
	let fs = JailedFs::open(&std::env::current_dir()?)?;
	let (scanned, findings) = scan_files(&fs, &manifest)?;
	if !findings.is_empty() {
		let mut stderr = io::stderr().lock();
		for finding in &findings {
			writeln!(
				stderr,
				"{}:{}: {}",
				finding.path.to_string_lossy().escape_debug(),
				finding.line,
				finding.rule
			)?;
		}
		bail!("{} potential secret(s) found", findings.len());
	}
	println!("check_secrets: {scanned} tracked file(s) clean");
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn each_supported_credential_shape_reports_redacted_location() -> Result<()> {
		let scanner = Scanner::new()?;
		let cases = [
			(format!("{}OPENSSH {}", "-----BEGIN ", "PRIVATE KEY-----"), "private key block"),
			(format!("ghp_{}", "a".repeat(36)), "github token"),
			(format!("github_pat_{}", "a".repeat(30)), "github fine-grained token"),
			(format!("AKIA{}", "A".repeat(16)), "aws access key id"),
			(format!("xoxb-{}", "a".repeat(12)), "slack token"),
			(format!("AIza{}", "a".repeat(35)), "google api key"),
			(format!("sk-proj-{}", "a".repeat(24)), "openai key"),
			(format!("sk-ant-{}", "a".repeat(24)), "anthropic key"),
			(format!("cio{}", "a".repeat(28)), "crates.io token"),
			(format!("npm_{}", "a".repeat(30)), "npm token"),
			(format!("sk_live_{}", "a".repeat(24)), "stripe key"),
			(format!("api_key: \"{}\"", "b".repeat(32)), "generic secret assignment"),
			(
				format!("{}deploy:{}@example.com", "https://", "a".repeat(24)),
				"url with inline credentials",
			),
		];
		for (credential, rule) in cases {
			let findings =
				scanner.scan(&format!("header\n{credential}\n"), Path::new("settings.txt"))?;
			assert!(
				findings
					.iter()
					.any(|finding| finding.rule == rule && finding.line == 2)
			);
			assert!(!format!("{findings:?}").contains(&credential));
		}
		Ok(())
	}

	#[test]
	fn placeholders_and_binary_data_are_excluded_but_private_keys_are_not() -> Result<()> {
		let scanner = Scanner::new()?;
		for sample in [
			"aws = AKIAIOSFODNN7EXAMPLE",
			"api_key: \"${OMP_TOKEN}\"",
			"https://user:secret@github.com/repo",
			"https://example.com/path",
		] {
			assert!(scanner.scan(sample, Path::new("fixture"))?.is_empty());
		}
		let key = format!("{}EXAMPLE {}", "-----BEGIN ", "PRIVATE KEY-----");
		assert_eq!(scanner.scan(&key, Path::new("fixture"))?.len(), 1);
		assert!(
			scanner
				.scan(&format!("\0{key}"), Path::new("fixture"))?
				.is_empty()
		);
		assert!(
			scanner
				.scan(&format!("ghp_{}", "a".repeat(19)), Path::new("fixture"))?
				.is_empty()
		);
		Ok(())
	}

	#[test]
	fn tracked_scan_fails_closed_on_missing_escaping_oversized_or_malformed_inputs() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		fs.atomic_write(Path::new("settings.txt"), format!("\nghp_{}", "a".repeat(36)).as_bytes())?;
		let (scanned, findings) = scan_files(&fs, b"settings.txt\0")?;
		assert_eq!(scanned, 1);
		assert_eq!(findings[0].line, 2);
		assert_eq!(findings[0].path, Path::new("settings.txt"));
		assert!(scan_files(&fs, b"missing.txt\0").is_err());
		assert!(scan_files(&fs, b"../outside\0").is_err());
		assert!(scan_files(&fs, b"settings.txt").is_err());
		fs.atomic_write(Path::new("large.txt"), &vec![b'a'; FILE_LIMIT + 1])?;
		assert!(scan_files(&fs, b"large.txt\0").is_err());
		Ok(())
	}

	#[test]
	fn excessive_matches_surface_an_error_instead_of_unbounded_diagnostics() -> Result<()> {
		let scanner = Scanner::new()?;
		let credential = format!("ghp_{}\n", "a".repeat(24));
		assert!(
			scanner
				.scan(&credential.repeat(FINDING_LIMIT + 1), Path::new("fixture"))
				.is_err()
		);
		Ok(())
	}
}
