use anyhow::Result;
use harvest_omp::session_format::{SessionDocument, TITLE_SLOT_BYTES, title_slot};

#[test]
fn v3_preserves_exact_bytes_signed_payloads_unknown_fields_and_selected_branch() -> Result<()> {
	let mut body = title_slot("current 漢字", Some("user"), "2026-10-11T00:00:00Z")?;
	let records = concat!(
		"{\"type\":\"session\",\"version\":3,\"id\":\"s\",\"title\":\"old\",\"unknown\":\
		 1844674407370955161999}\n",
		"{\"type\":\"message\",\"id\":\"a\",\"parentId\":null,\"message\":{\"role\":\"assistant\",\"\
		 content\":[{\"type\":\"thinking\",\"signature\":\"opaque\\u0061\",\"encrypted\":{\"n\":\
		 1844674407370955161999}}]}}\n",
		"{\"type\":\"message\",\"id\":\"b\",\"parentId\":\"a\",\"message\":{\"role\":\"user\",\"\
		 content\":\"left\"}}\n",
		"{\"type\":\"future_extension\",\"id\":\"c\",\"parentId\":\"a\",\"data\":{\"blob\":\"blob://\
		 opaque\"}}\n"
	);
	body.extend_from_slice(records.as_bytes());
	let document = SessionDocument::decode(&body)?;
	assert_eq!(document.export_v3()?, body);
	assert_eq!(document.header()["title"].get(), "\"current 漢字\"");
	assert_eq!(document.header()["titleSource"].get(), "\"user\"");
	assert_eq!(document.header()["unknown"].get(), "1844674407370955161999");
	let branch = document.branch("c")?;
	assert_eq!(branch.iter().map(|row| row["id"].get()).collect::<Vec<_>>(), ["\"a\"", "\"c\""]);
	assert_eq!(
		branch[0]["message"].get(),
		r#"{"role":"assistant","content":[{"type":"thinking","signature":"opaque\u0061","encrypted":{"n":1844674407370955161999}}]}"#
	);
	Ok(())
}

#[test]
fn legacy_migration_rebuilds_tree_compaction_and_custom_roles_without_losing_payloads() -> Result<()>
{
	let source = concat!(
		"{\"type\":\"session\",\"id\":\"legacy\"}\n",
		"{\"type\":\"message\",\"message\":{\"role\":\"hookMessage\",\"content\":\"lesson\",\"\
		 unknown\":1844674407370955161999}}\n",
		"{\"type\":\"compaction\",\"firstKeptEntryIndex\":1,\"summary\":\"kept\"}\n"
	);
	let document = SessionDocument::decode(source.as_bytes())?;
	assert_eq!(document.original_bytes(), source.as_bytes());
	let imported = SessionDocument::decode(&document.export_v3()?)?;
	let rows = imported.entries();
	let leaf: String = serde_json::from_str(rows[1]["id"].get())?;
	assert_eq!(imported.branch(&leaf)?.len(), 2);
	assert_eq!(rows[1]["firstKeptEntryId"].get(), rows[0]["id"].get());
	assert!(!rows[1].contains_key("firstKeptEntryIndex"));
	let message: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
		serde_json::from_str(rows[0]["message"].get())?;
	assert_eq!(message["role"].get(), "\"custom\"");
	assert_eq!(message["unknown"].get(), "1844674407370955161999");
	// V2 keeps existing branch IDs rather than reconstructing a linear history.
	let v2 = br#"{"type":"session","version":2,"id":"v2"}
{"type":"message","id":"branch","parentId":null,"message":{"role":"hookMessage","content":"old"}}
"#;
	let document = SessionDocument::decode(v2)?;
	assert_eq!(document.entries()[0]["id"].get(), "\"branch\"");
	assert!(
		document.entries()[0]["message"]
			.get()
			.contains("\"custom\"")
	);
	Ok(())
}

#[test]
fn invalid_headers_future_versions_duplicate_ids_and_broken_branches_surface_errors() -> Result<()>
{
	assert!(SessionDocument::decode(br#"{"role":"user","content":"prototype"}"#).is_err());
	assert!(SessionDocument::decode(br#"{"type":"session","id":"s","version":4}"#).is_err());
	assert!(
		SessionDocument::decode(
			br#"{"type":"session","id":"s","version":3}
{"type":"message","id":"a","parentId":null}
{"type":"message","id":"a","parentId":null}
"#
		)
		.is_err()
	);
	let cycle = SessionDocument::decode(
		br#"{"type":"session","id":"s","version":3}
{"type":"message","id":"a","parentId":"b"}
{"type":"message","id":"b","parentId":"a"}
"#,
	)?;
	assert!(cycle.branch("a").is_err());
	assert!(cycle.branch("missing").is_err());
	assert!(SessionDocument::decode(&vec![b'x'; harvest_runtime::MAX_EVENT_BYTES + 1]).is_err());
	let damaged = b"{\"type\":\"session\",\"id\":\"s\",\"version\":3}\ntruncated garbage\n";
	let document = SessionDocument::decode(damaged)?;
	assert_eq!(document.malformed_records(), 1);
	assert_eq!(document.export_v3()?, damaged);
	Ok(())
}

#[test]
fn title_slots_remain_256_bytes_with_unicode_quotes_and_source_override() -> Result<()> {
	let slot = title_slot(&"漢字\"\\🙂".repeat(300), None, "2026-10-11T00:00:00Z")?;
	assert_eq!(slot.len(), TITLE_SLOT_BYTES);
	let parsed: serde_json::Value = serde_json::from_slice(&slot)?;
	let title = parsed["title"].as_str().unwrap();
	assert!("漢字\"\\🙂".repeat(300).starts_with(title));
	assert!(!parsed.as_object().unwrap().contains_key("source"));
	let mut bytes = slot;
	bytes.extend_from_slice(
		b"{\"type\":\"session\",\"id\":\"s\",\"version\":3,\"titleSource\":\"user\"}\n",
	);
	let document = SessionDocument::decode(&bytes)?;
	assert!(!document.header().contains_key("titleSource"));
	assert!(title_slot("title", None, &"x".repeat(300)).is_err());
	Ok(())
}

#[test]
fn inspection_cli_reports_branch_and_damage_without_rewriting_or_rounding_data() -> Result<()> {
	let root = tempfile::tempdir()?;
	let bytes = b"{\"type\":\"session\",\"id\":\"s\",\"version\":3,\"unknown\":1844674407370955161999}\n{\"type\":\"message\",\"id\":\"a\",\"parentId\":null}\n{\"type\":\"message\",\"id\":\"b\",\"parentId\":null}\ntruncated\n";
	std::fs::write(root.path().join("session.jsonl"), bytes)?;
	let output = std::process::Command::new(env!("CARGO_BIN_EXE_harvest"))
		.args(["session", "--workspace"])
		.arg(root.path())
		.args(["--leaf", "a", "session.jsonl"])
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let report: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
		serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["branchEntries"].get(), "1");
	assert_eq!(report["entries"].get(), "2");
	assert_eq!(report["malformedRecords"].get(), "1");
	assert_eq!(report["migrationRequired"].get(), "false");
	let header: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
		serde_json::from_str(report["header"].get())?;
	assert_eq!(header["unknown"].get(), "1844674407370955161999");
	assert_eq!(std::fs::read(root.path().join("session.jsonl"))?, bytes);
	assert!(!root.path().join(".harvest").exists());
	assert!(!root.path().join(".omp").exists());
	Ok(())
}
