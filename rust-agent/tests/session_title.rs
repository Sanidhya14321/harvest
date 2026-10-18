//! Native title changes must preserve opaque provider data and writer
//! ownership.
use std::{
	path::Path,
	process::{Command, Output},
};

use anyhow::Result;
use harvest_omp::{
	durable_session::DurableSession,
	session_format::{SessionDocument, TITLE_SLOT_BYTES, title_slot},
};

const HEADER: &[u8] =
	b"{\"type\":\"session\",\"version\":3,\"id\":\"titles\",\"title\":\"old header title\"}\n";
const MESSAGE: &[u8] = br#"{"type":"message","id":"opaque","parentId":null,"message":{"role":"assistant","content":[{"type":"thinking","thinking":"private","signature":"opaque"}],"providerPayload":{"large":900719925474099312345}}}
"#;

fn rename(root: &Path, title: &str) -> Result<Output> {
	Ok(Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["session", "--workspace"])
		.arg(root)
		.args(["--title", title, "source.jsonl"])
		.output()?)
}

#[test]
fn rename_preserves_every_non_title_byte_even_for_opaque_provider_messages() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("source.jsonl");
	let body = [HEADER, MESSAGE].concat();
	std::fs::write(&path, &body)?;
	let output = rename(root.path(), "Project review")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["header"]["title"], "Project review");
	assert_eq!(report["header"]["titleSource"], "user");
	let renamed = std::fs::read(&path)?;
	assert_eq!(&renamed[TITLE_SLOT_BYTES..], body);
	let long = "🦀\"\n".repeat(100);
	let output = rename(root.path(), &long)?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let bytes = std::fs::read(&path)?;
	assert_eq!(&bytes[TITLE_SLOT_BYTES..], body);
	let slot: serde_json::Value = serde_json::from_slice(&bytes[..TITLE_SLOT_BYTES])?;
	let actual = slot["title"].as_str().unwrap();
	assert!(long.starts_with(actual) && actual.len() < long.len());
	assert_eq!(bytes.iter().position(|byte| *byte == b'\n'), Some(TITLE_SLOT_BYTES - 1));
	let output = rename(root.path(), "")?;
	assert!(output.status.success());
	let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["header"]["title"], "");
	Ok(())
}

#[test]
fn variable_width_legacy_title_line_is_replaced_without_truncating_header() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut slot = title_slot("before", Some("auto"), "then")?;
	let mut value: serde_json::Value = serde_json::from_slice(&slot)?;
	value["pad"] = serde_json::json!("");
	slot = serde_json::to_vec(&value)?;
	slot.push(b'\n');
	std::fs::write(root.path().join("source.jsonl"), [slot.as_slice(), HEADER].concat())?;
	assert!(rename(root.path(), "after")?.status.success());
	let bytes = std::fs::read(root.path().join("source.jsonl"))?;
	assert_eq!(&bytes[TITLE_SLOT_BYTES..], HEADER);
	Ok(())
}

#[test]
fn malformed_legacy_unterminated_and_invalid_branch_sessions_are_never_rewritten() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("source.jsonl");
	for bytes in [
		b"{\"type\":\"session\",\"version\":1,\"id\":\"legacy\"}\n".to_vec(),
		[HEADER, b"bad record\n"].concat(),
		HEADER[..HEADER.len() - 1].to_vec(),
		[HEADER, b"{\"type\":\"message\",\"id\":\"bad\",\"parentId\":\"missing\"}\n"].concat(),
	] {
		std::fs::write(&path, &bytes)?;
		assert!(!rename(root.path(), "after")?.status.success());
		assert_eq!(std::fs::read(&path)?, bytes);
	}
	Ok(())
}

#[test]
fn title_changes_cannot_replace_a_session_held_by_an_active_rust_writer() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = Path::new("source.jsonl");
	std::fs::write(root.path().join(path), HEADER)?;
	let session = DurableSession::open(root.path(), path, None)?;
	assert!(!rename(root.path(), "blocked")?.status.success());
	assert_eq!(std::fs::read(root.path().join(path))?, HEADER);
	drop(session);
	assert!(rename(root.path(), "available")?.status.success());
	let session = DurableSession::open(root.path(), path, None)?;
	assert_eq!(session.id(), "titles");
	Ok(())
}

#[test]
fn writer_rejects_invalid_off_branch_entries_before_any_future_append() -> Result<()> {
	let root = tempfile::tempdir()?;
	let bytes = [HEADER, b"{\"type\":\"title_change\",\"id\":\"bad\",\"parentId\":\"missing\"}\n{\"type\":\"title_change\",\"id\":\"good\",\"parentId\":null}\n"].concat();
	std::fs::write(root.path().join("source.jsonl"), &bytes)?;
	assert!(DurableSession::open(root.path(), Path::new("source.jsonl"), Some("good")).is_err());
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, bytes);
	// Read-only inspection still exposes the valid historical branch.
	assert_eq!(SessionDocument::decode(&bytes)?.branch("good")?.len(), 1);
	Ok(())
}

#[cfg(unix)]
#[test]
fn title_mutations_cannot_escape_through_symlinked_session_directories() -> Result<()> {
	let root = tempfile::tempdir()?;
	let outside = tempfile::tempdir()?;
	std::fs::write(outside.path().join("source.jsonl"), HEADER)?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	let output = Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["session", "--workspace"])
		.arg(root.path())
		.args(["--title", "blocked", "escape/source.jsonl"])
		.output()?;
	assert!(!output.status.success());
	assert_eq!(std::fs::read(outside.path().join("source.jsonl"))?, HEADER);
	assert!(!outside.path().join("source.jsonl.rust-owner.lock").exists());
	Ok(())
}
