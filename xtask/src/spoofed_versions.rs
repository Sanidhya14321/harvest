//! Check upstream client identity versions without a JavaScript runtime.
use std::{
	env,
	path::{Path, PathBuf},
	time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::{JailedFs, read_bounded};
use regex::Regex;
use reqwest::blocking::Client;
use serde::Deserialize;

const PROVIDER: &str = "packages/catalog/src/wire/gemini-headers.ts";
const RESPONSE_LIMIT: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Release {
	tag_name: String,
}

fn release_version(bytes: &[u8]) -> Result<String> {
	let release: Release =
		serde_json::from_slice(bytes).context("invalid GitHub release metadata")?;
	let pattern = Regex::new(r"\d+\.\d+\.\d+")?;
	let version = pattern
		.find(&release.tag_name)
		.context("release tag contains no semantic version")?
		.as_str();
	semver::Version::parse(version).context("release version exceeds semantic version limits")?;
	Ok(version.to_owned())
}

fn fetch_release(base: &str) -> Result<Vec<u8>> {
	let client = Client::builder()
		.timeout(Duration::from_secs(20))
		.redirect(reqwest::redirect::Policy::limited(3))
		.build()?;
	let response = client
		.get(format!("{base}/repos/google-gemini/gemini-cli/releases/latest"))
		.header("Accept", "application/vnd.github+json")
		.header("User-Agent", concat!("Harvest/", env!("CARGO_PKG_VERSION")))
		.send()
		.context("could not fetch latest Gemini CLI release")?
		.error_for_status()
		.context("GitHub release lookup failed")?;
	read_bounded(response, RESPONSE_LIMIT)
}

#[derive(Parser)]
#[command(name = "cargo xtask spoofed-versions")]
struct Arguments {
	#[arg(long)]
	root:         Option<PathBuf>,
	#[arg(long)]
	update:       bool,
	/// Use captured release metadata. This verifies the conversion/update
	/// contract, not live upstream freshness.
	#[arg(long)]
	release_json: Option<PathBuf>,
}

/// Check the source's client identity; prepare an atomic update only after
/// validating both the source and the release metadata.
pub fn check(fs: &JailedFs, release: &[u8], update: bool) -> Result<bool> {
	let latest = release_version(release)?;
	let source = String::from_utf8(fs.read(Path::new(PROVIDER), RESPONSE_LIMIT)?)?;
	let pattern = Regex::new(r#"PI_AI_GEMINI_CLI_VERSION\s*\|\|\s*"(\d+\.\d+\.\d+)""#)?;
	let mut matches = pattern.captures_iter(&source);
	let found = matches
		.next()
		.context("could not extract current Gemini CLI version from source")?;
	if matches.next().is_some() {
		bail!("ambiguous Gemini CLI version declarations; source was not changed");
	}
	let current = found.get(1).context("missing captured source version")?;
	if current.as_str() == latest {
		println!("[OK]   Gemini CLI: {latest} (up to date)");
		return Ok(false);
	}
	println!("[DRIFT] Gemini CLI: {} -> {latest}", current.as_str());
	if update {
		let range = current.range();
		let mut updated = source.clone();
		updated.replace_range(range, &latest);
		fs.atomic_write(Path::new(PROVIDER), updated.as_bytes())?;
		println!("Wrote updates to {PROVIDER}");
	}
	Ok(true)
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args =
		Arguments::try_parse_from(std::iter::once("spoofed-versions".into()).chain(arguments))?;
	let fs = JailedFs::open(&args.root.unwrap_or(env::current_dir()?))?;
	let release = if let Some(path) = args.release_json {
		println!("Release source: captured metadata (not a live freshness check)");
		fs.read(&path, RESPONSE_LIMIT)?
	} else {
		fetch_release("https://api.github.com")?
	};
	if check(&fs, &release, args.update)? && !args.update {
		bail!("Run with --update to apply version bumps.");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		net::TcpListener,
	};

	use super::*;

	#[test]
	fn upstream_release_uses_the_github_contract_and_rejects_error_statuses() -> Result<()> {
		for (status, body, succeeds) in
			[("200 OK", r#"{"tag_name":"v0.48.2"}"#, true), ("403 Forbidden", "denied", false)]
		{
			let server = TcpListener::bind("127.0.0.1:0")?;
			let address = server.local_addr()?;
			let worker = std::thread::spawn(move || -> Result<()> {
				let (mut stream, _) = server.accept()?;
				stream.set_read_timeout(Some(Duration::from_secs(5)))?;
				let mut input = Vec::new();
				let mut chunk = [0; 512];
				while !input.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
					let size = stream.read(&mut chunk)?;
					if size == 0 || input.len() + size > 4096 {
						bail!("invalid fixture HTTP request");
					}
					input.extend_from_slice(&chunk[..size]);
				}
				let request = std::str::from_utf8(&input)?.to_ascii_lowercase();
				assert!(
					request.starts_with("get /repos/google-gemini/gemini-cli/releases/latest http/1.1")
				);
				assert!(request.contains("accept: application/vnd.github+json"));
				assert!(request.contains("user-agent: harvest/"));
				write!(
					stream,
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)?;
				Ok(())
			});
			let result = fetch_release(&format!("http://{address}"));
			worker.join().expect("fixture server panicked")?;
			if succeeds {
				assert_eq!(release_version(&result?)?, "0.48.2");
			} else {
				assert!(result.is_err());
			}
		}
		Ok(())
	}
}
