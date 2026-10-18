//! Recovery copies a validated prefix and never silently changes the source.
use std::{
	path::Path,
	process::{Command, Output},
};

use anyhow::Result;
use harvest_omp::{chat::ChatMessage, durable_session::DurableSession, session_format::title_slot};
use harvest_runtime::content_hash;

const PREFIX: &[u8] = br#"{"type":"session","version":3,"id":"recover"}
{"type":"message","id":"user","parentId":null,"message":{"role":"user","content":"original"}}
"#;
const PARTIAL: &[u8] = br#"{"type":"message","id":"partial""#;

fn recover(root: &Path, destination: &str) -> Result<Output> {
	Ok(Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["session", "--workspace"])
		.arg(root)
		.args(["--recover-to", destination, "source.jsonl"])
		.output()?)
}

#[test]
fn recovered_prefix_can_resume_and_receipt_identifies_every_excluded_byte() -> Result<()> {
	let root = tempfile::tempdir()?;
	let source = [PREFIX, PARTIAL].concat();
	std::fs::write(root.path().join("source.jsonl"), &source)?;
	let output = recover(root.path(), "recovered.jsonl")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["recoveredTo"], "recovered.jsonl");
	assert_eq!(report["recovery"]["sourceHash"], content_hash(&source));
	assert_eq!(report["recovery"]["recoveredBytes"], PREFIX.len());
	assert_eq!(report["recovery"]["excludedTailBytes"], PARTIAL.len());
	assert_eq!(std::fs::read(root.path().join("recovered.jsonl"))?, PREFIX);
	assert!(DurableSession::open(root.path(), Path::new("source.jsonl"), None).is_err());
	let mut session = DurableSession::open(root.path(), Path::new("recovered.jsonl"), None)?;
	assert_eq!(session.messages()[0].content.as_deref(), Some("original"));
	session.commit(
		&ChatMessage {
			role:         "assistant".into(),
			content:      Some("resumed".into()),
			tool_calls:   None,
			tool_call_id: None,
		},
		"fixture",
	)?;
	drop(session);
	let session = DurableSession::open(root.path(), Path::new("recovered.jsonl"), None)?;
	assert_eq!(
		session
			.messages()
			.last()
			.and_then(|message| message.content.as_deref()),
		Some("resumed")
	);
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	Ok(())
}

#[test]
fn recovery_preserves_title_slots_and_opaque_payloads_byte_for_byte() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut prefix = title_slot("漢字 recovery", Some("user"), "2026-10-18T00:00:00Z")?;
	prefix.extend_from_slice(br#"{"type":"session","version":3,"id":"s","unknown":1844674407370955161999}
{"type":"future_extension","id":"signed","parentId":null,"payload":{"signature":"opaque\u0061","n":1844674407370955161999}}
"#);
	let source = [prefix.as_slice(), PARTIAL].concat();
	std::fs::write(root.path().join("source.jsonl"), &source)?;
	let output = recover(root.path(), "recovered.jsonl")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert_eq!(std::fs::read(root.path().join("recovered.jsonl"))?, prefix);
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	Ok(())
}

#[test]
fn an_interrupted_utf8_sequence_is_retained_in_the_original_and_counted_in_the_receipt()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let tail = [br#"{"type":"message","text":""#.as_slice(), &[0xf0, 0x9f]].concat();
	let source = [PREFIX, &tail].concat();
	std::fs::write(root.path().join("source.jsonl"), &source)?;
	let output = recover(root.path(), "recovered.jsonl")?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["recovery"]["excludedTailBytes"], tail.len());
	assert_eq!(std::fs::read(root.path().join("recovered.jsonl"))?, PREFIX);
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	Ok(())
}

#[test]
fn complete_records_interior_damage_and_invalid_trees_cannot_be_discarded_as_a_partial_tail()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let complete = br#"{"type":"message","id":"complete","parentId":"user","message":{"role":"user","content":"keep me"}}"#;
	let malformed = br#"{"type":invalid}"#;
	let broken = br#"{"type":"message","id":"broken","parentId":"missing"}
"#;
	for (source, expected) in [
		([PREFIX, complete].concat(), "final record is complete"),
		([PREFIX, PARTIAL, b"\n"].concat(), "no unterminated final record"),
		([PREFIX, b"bad interior row\n", PARTIAL].concat(), "damaged session"),
		([PREFIX, malformed].concat(), "malformed rather than truncated"),
		([PREFIX, broken, PARTIAL].concat(), "missing branch entry"),
		(
			[
				br#"{"type":"session","version":2,"id":"legacy"}
"#
				.as_slice(),
				PARTIAL,
			]
			.concat(),
			"requires a v3 session",
		),
	] {
		std::fs::write(root.path().join("source.jsonl"), &source)?;
		let output = recover(root.path(), "recovered.jsonl")?;
		assert!(!output.status.success());
		assert!(
			String::from_utf8_lossy(&output.stderr).contains(expected),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
		assert!(!root.path().join("recovered.jsonl").exists());
		assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	}
	Ok(())
}

#[test]
fn recovery_keeps_tool_calls_and_refuses_resume_when_their_result_was_interrupted() -> Result<()> {
	let root = tempfile::tempdir()?;
	let call = br#"{"type":"message","id":"assistant","parentId":"user","message":{"role":"assistant","content":[{"type":"toolCall","id":"tool-1","name":"write","arguments":{"path":"note.txt","contents":"must not replay"}}]}}
"#;
	let prefix = [PREFIX, call].concat();
	std::fs::write(root.path().join("source.jsonl"), [prefix.as_slice(), PARTIAL].concat())?;
	assert!(recover(root.path(), "recovered.jsonl")?.status.success());
	let Err(error) = DurableSession::open(root.path(), Path::new("recovered.jsonl"), None) else {
		anyhow::bail!("unresolved tool call was accepted for replay");
	};
	assert!(error.to_string().contains("unresolved tool calls"));
	assert_eq!(std::fs::read(root.path().join("recovered.jsonl"))?, prefix);
	assert!(!root.path().join("note.txt").exists());
	Ok(())
}

#[test]
fn recovery_never_overwrites_an_existing_destination_and_conflicting_modes_write_nothing()
-> Result<()> {
	let root = tempfile::tempdir()?;
	let source = [PREFIX, PARTIAL].concat();
	std::fs::write(root.path().join("source.jsonl"), &source)?;
	std::fs::write(root.path().join("existing.jsonl"), b"keep")?;
	assert!(!recover(root.path(), "existing.jsonl")?.status.success());
	assert!(!recover(root.path(), "source.jsonl")?.status.success());
	let output = Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["session", "--workspace"])
		.arg(root.path())
		.args(["--recover-to", "recovered.jsonl", "--import-v3", "imported.jsonl", "source.jsonl"])
		.output()?;
	assert!(!output.status.success());
	assert!(!root.path().join("recovered.jsonl").exists());
	assert!(!root.path().join("imported.jsonl").exists());
	assert_eq!(std::fs::read(root.path().join("existing.jsonl"))?, b"keep");
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	Ok(())
}

#[test]
fn import_and_recovery_wait_for_the_cooperating_rust_writer_to_release_ownership() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("source.jsonl"), PREFIX)?;
	let owner = DurableSession::open(root.path(), Path::new("source.jsonl"), None)?;
	let output = Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["session", "--workspace"])
		.arg(root.path())
		.args(["--import-v3", "imported.jsonl", "source.jsonl"])
		.output()?;
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("another writer"));
	assert!(!root.path().join("imported.jsonl").exists());
	let source = [PREFIX, PARTIAL].concat();
	std::fs::write(root.path().join("source.jsonl"), &source)?;
	let output = recover(root.path(), "recovered.jsonl")?;
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("another writer"));
	assert!(!root.path().join("recovered.jsonl").exists());
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	drop(owner);
	assert!(recover(root.path(), "recovered.jsonl")?.status.success());
	assert_eq!(std::fs::read(root.path().join("recovered.jsonl"))?, PREFIX);
	Ok(())
}
