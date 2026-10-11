//! Conservative, opaque repository identities. Uncertain configuration never
//! supports global promotion.
use std::path::Path;

use harvest_runtime::{JailedFs, content_hash};

/// Return the same identity for origin URLs differing only by SSH/HTTPS
/// transport.
///
/// Includes, missing origins, local repositories, and external
/// worktree metadata are deliberately unresolved rather than counted as
/// independent projects.
pub fn repository_identity(root: &JailedFs) -> Option<String> {
	let bytes = root.read(Path::new(".git/config"), 64 * 1024).ok()?;
	let config = std::str::from_utf8(&bytes).ok()?;
	let mut origin = false;
	let mut urls = Vec::new();
	for line in config
		.lines()
		.map(str::trim)
		.filter(|line| !line.starts_with(['#', ';']))
	{
		if line.starts_with('[') {
			let section = line.strip_suffix(']')?.to_ascii_lowercase();
			if section.starts_with("[include") {
				return None;
			}
			origin = section == "[remote \"origin\"";
		} else if origin
			&& let Some((key, value)) = line.split_once('=')
			&& key.trim().eq_ignore_ascii_case("url")
		{
			urls.push(value.trim());
		}
	}
	if urls.len() != 1 {
		return None;
	}
	canonical_remote(urls[0]).map(|canonical| format!("repo:{}", content_hash(canonical.as_bytes())))
}

fn canonical_remote(remote: &str) -> Option<String> {
	if remote.contains(['\\', '?', '#', '"']) || remote.chars().any(char::is_whitespace) {
		return None;
	}
	let (authority, path) = if let Some((scheme, rest)) = remote.split_once("://") {
		if !matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http" | "ssh" | "git") {
			return None;
		}
		rest.split_once('/')?
	} else {
		let (authority, path) = remote.split_once(':')?;
		if !authority.contains('@') {
			return None;
		}
		(authority, path)
	};
	let host = authority
		.rsplit('@')
		.next()?
		.split(':')
		.next()?
		.to_ascii_lowercase();
	let path = path
		.trim_matches('/')
		.strip_suffix(".git")
		.unwrap_or_else(|| path.trim_matches('/'))
		.to_ascii_lowercase();
	if host.is_empty()
		|| !host.contains('.')
		|| path.is_empty()
		|| path
			.split('/')
			.any(|part| part.is_empty() || part == "." || part == "..")
	{
		return None;
	}
	Some(format!("{host}/{path}"))
}
