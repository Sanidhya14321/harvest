//! Explicit imports must be complete, non-destructive and safely resumable.
use std::{
	path::Path,
	process::{Command, Output},
};

use anyhow::Result;
use harvest_omp::{
	chat::ChatMessage, durable_session::DurableSession, session_format::SessionDocument,
};

fn import(root: &Path, source: &str, destination: &str, leaf: Option<&str>) -> Result<Output> {
	let mut command = Command::new(env!("CARGO_BIN_EXE_harvest"));
	command
		.args(["session", "--workspace"])
		.arg(root)
		.args(["--import-v3", destination, source]);
	if let Some(leaf) = leaf {
		command.args(["--leaf", leaf]);
	}
	Ok(command.output()?)
}

#[test]
fn legacy_cli_import_keeps_original_bytes_and_resumes_a_separate_v3_session() -> Result<()> {
	let root = tempfile::tempdir()?;
	let source = r#"{"type":"session","id":"legacy"}
{"type":"message","message":{"role":"user","content":"question"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"answer"}]}}
"#;
	std::fs::write(root.path().join("legacy.jsonl"), source)?;
	let output = import(root.path(), "legacy.jsonl", "imports/current.jsonl", None)?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["importedTo"], "imports/current.jsonl");
	let mut session = DurableSession::open(root.path(), Path::new("imports/current.jsonl"), None)?;
	assert_eq!(
		session
			.messages()
			.iter()
			.map(|message| message.content.as_deref())
			.collect::<Vec<_>>(),
		[Some("question"), Some("answer")]
	);
	session.commit(
		&ChatMessage {
			role:         "user".into(),
			content:      Some("follow-up".into()),
			tool_calls:   None,
			tool_call_id: None,
		},
		"fixture",
	)?;
	drop(session);
	let resumed = DurableSession::open(root.path(), Path::new("imports/current.jsonl"), None)?;
	assert_eq!(
		resumed
			.messages()
			.last()
			.and_then(|message| message.content.as_deref()),
		Some("follow-up")
	);
	assert_eq!(std::fs::read(root.path().join("legacy.jsonl"))?, source.as_bytes());
	Ok(())
}

#[test]
fn importing_a_branched_v2_session_preserves_all_branches_and_unknown_raw_values() -> Result<()> {
	let root = tempfile::tempdir()?;
	let source = r#"{"type":"session","version":2,"id":"branched","unknown":1844674407370955161999}
{"type":"message","id":"left","parentId":null,"message":{"role":"user","content":"left"}}
{"type":"future_extension","id":"right","parentId":null,"signed":{"signature":"opaque\u0061","n":1844674407370955161999}}
"#;
	std::fs::write(root.path().join("source.jsonl"), source)?;
	assert!(
		import(root.path(), "source.jsonl", "imported.jsonl", None)?
			.status
			.success()
	);
	let bytes = std::fs::read(root.path().join("imported.jsonl"))?;
	let imported = SessionDocument::decode(&bytes)?;
	assert!(!imported.migrated());
	assert_eq!(imported.branch("left")?.len(), 1);
	assert_eq!(imported.branch("right")?.len(), 1);
	assert_eq!(imported.header()["unknown"].get(), "1844674407370955161999");
	assert_eq!(
		imported.entries()[1]["signed"].get(),
		r#"{"signature":"opaque\u0061","n":1844674407370955161999}"#
	);
	assert!(DurableSession::open(root.path(), Path::new("imported.jsonl"), Some("right")).is_err());
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source.as_bytes());
	Ok(())
}

#[test]
fn damaged_records_and_broken_unselected_branches_cannot_be_imported_as_clean_sessions()
-> Result<()> {
	let root = tempfile::tempdir()?;
	for (tail, expected_error) in [
		("truncated\n", "damaged session"),
		(
			r#"{"type":"message","id":"bad","parentId":"missing"}
"#,
			"missing branch entry",
		),
		(
			r#"{"type":"message","id":"bad","parentId":"bad"}
"#,
			"cyclic session parent chain",
		),
	] {
		let source = format!(
			r#"{{"type":"session","id":"s","version":3}}
{{"type":"message","id":"clean","parentId":null,"message":{{"role":"user","content":"clean"}}}}
{tail}"#
		);
		std::fs::write(root.path().join("damaged.jsonl"), &source)?;
		let output = import(root.path(), "damaged.jsonl", "new.jsonl", Some("clean"))?;
		assert!(!output.status.success());
		assert!(String::from_utf8_lossy(&output.stderr).contains(expected_error));
		assert!(!root.path().join("new.jsonl").exists());
		assert_eq!(std::fs::read(root.path().join("damaged.jsonl"))?, source.as_bytes());
	}
	Ok(())
}

#[test]
fn imports_never_overwrite_the_source_or_an_existing_destination() -> Result<()> {
	let root = tempfile::tempdir()?;
	let source = b"{\"type\":\"session\",\"version\":3,\"id\":\"s\"}";
	std::fs::write(root.path().join("source.jsonl"), source)?;
	std::fs::write(root.path().join("existing.jsonl"), b"keep")?;
	assert!(
		!import(root.path(), "source.jsonl", "source.jsonl", None)?
			.status
			.success()
	);
	assert!(
		!import(root.path(), "source.jsonl", "existing.jsonl", None)?
			.status
			.success()
	);
	assert!(
		!import(root.path(), "source.jsonl", "../outside.jsonl", None)?
			.status
			.success()
	);
	assert_eq!(std::fs::read(root.path().join("existing.jsonl"))?, b"keep");
	assert_eq!(std::fs::read(root.path().join("source.jsonl"))?, source);
	// A complete final record without its newline is imported with a delimiter,
	// so the first resumed append cannot corrupt that record.
	assert!(
		import(root.path(), "source.jsonl", "new.jsonl", None)?
			.status
			.success()
	);
	assert_eq!(std::fs::read(root.path().join("new.jsonl"))?, [source.as_slice(), b"\n"].concat());
	Ok(())
}
