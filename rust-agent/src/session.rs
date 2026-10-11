use std::path::Path;

use anyhow::{Context, Result};

use crate::chat::ChatMessage;

pub fn append(root: &Path, message: &ChatMessage) -> Result<()> {
	let filesystem = harvest_runtime::JailedFs::open(root)?;
	let mut line = serde_json::to_vec(message).context("failed to encode a session message")?;
	if line.len() > harvest_runtime::MAX_EVENT_BYTES {
		anyhow::bail!("session event exceeds frame limit");
	}
	line.push(b'\n');
	filesystem.append(Path::new(".omp/session.jsonl"), &line, 64 * 1024 * 1024)?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::append;
	use crate::chat::user;

	#[test]
	fn messages_append_as_json_lines() {
		let root = tempfile::tempdir().unwrap();
		append(root.path(), &user("one")).unwrap();
		append(root.path(), &user("two")).unwrap();
		let text = fs::read_to_string(root.path().join(".omp/session.jsonl")).unwrap();
		assert!(text.contains("\"content\":\"one\""));
		assert!(text.contains("\"content\":\"two\""));
		assert_eq!(text.lines().count(), 2);
	}
}
