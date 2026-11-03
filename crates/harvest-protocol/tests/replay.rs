use anyhow::Result;
use harvest_protocol::{
	display::{DisplayBlock, DisplayContent, ExecutionLanguage, ReplayProjection, project_branch},
	session::SessionDocument,
};

const REPLAY: &[u8] = include_bytes!("fixtures/replay.jsonl");

#[test]
fn portable_import_keeps_existing_sha256_ids_and_signed_payload_bytes() -> Result<()> {
	let source = include_bytes!("fixtures/legacy.jsonl");
	let document = SessionDocument::decode(source)?;
	assert_eq!(document.entries()[0]["id"].get(), "\"0b70e513\"");
	assert_eq!(document.entries()[1]["id"].get(), "\"ce30069f\"");
	let imported = SessionDocument::decode(&document.export_v3()?)?;
	let branch = imported.branch("ce30069f")?;
	assert_eq!(branch[1]["firstKeptEntryId"].get(), "\"0b70e513\"");
	let message: harvest_protocol::session::SessionObject =
		serde_json::from_str(branch[0]["message"].get())?;
	assert_eq!(message["role"].get(), "\"custom\"");
	assert_eq!(
		message["providerPayload"].get(),
		r#"{"signed":"opaque\u0061","n":1844674407370955161999}"#
	);
	assert_eq!(document.original_bytes(), source);
	Ok(())
}

#[test]
fn selected_branch_projects_tool_pairing_images_and_summaries_without_rewriting_source()
-> Result<()> {
	let document = SessionDocument::decode(REPLAY)?;
	let projection = project_branch(&document, "visible")?;
	assert_eq!(
		projection
			.entries
			.iter()
			.map(|entry| entry.id.as_str())
			.collect::<Vec<_>>(),
		["user", "assistant", "result", "compact", "summary", "hidden", "visible"]
	);
	let DisplayContent::Message { message } = &projection.entries[0].content else {
		panic!("user message was not projected");
	};
	assert!(
		matches!(&message.blocks[1], DisplayBlock::Image { data, mime_type } if data == "YWJj" && mime_type == "image/png")
	);
	let DisplayContent::Message { message } = &projection.entries[1].content else {
		panic!("assistant call was not projected");
	};
	assert!(
		matches!(&message.blocks[2], DisplayBlock::ToolCall { id, name, arguments: Some(arguments), .. }
		if id == "call-js" && name == "eval" && arguments.get() == r#"{"language":"js","code":"40 + 2","large":1844674407370955161999}"#)
	);
	let DisplayContent::Message { message } = &projection.entries[2].content else {
		panic!("tool result was not projected");
	};
	assert_eq!(message.tool_call_id.as_deref(), Some("call-js"));
	assert_eq!(message.tool_name.as_deref(), Some("eval"));
	assert!(matches!(&message.blocks[0], DisplayBlock::Text { text } if text == "42"));
	assert!(matches!(&projection.entries[3].content, DisplayContent::Compaction {
		first_kept_entry_id: Some(id), tokens_before: Some(1200), tokens_after: Some(120), ..
	} if id == "assistant"));
	assert!(
		matches!(&projection.entries[4].content, DisplayContent::BranchSummary { from_id: Some(id), .. } if id == "assistant")
	);
	assert!(!projection.entries[5].is_visible());
	assert!(projection.entries[6].is_visible());
	assert_eq!(document.export_v3()?, REPLAY);
	Ok(())
}

#[test]
fn display_serialization_excludes_hidden_and_opaque_data_but_source_keeps_exact_payloads()
-> Result<()> {
	let document = SessionDocument::decode(REPLAY)?;
	let projection = project_branch(&document, "visible")?;
	let rendered_payload = serde_json::to_string(&projection)?;
	for secret in [
		"provider-secret",
		"thinking-secret",
		"redacted-secret",
		"unknown-block-secret",
		"extension-secret",
		"preserved-secret",
		"hidden-body-secret",
		"hidden-details-secret",
	] {
		assert!(!rendered_payload.contains(secret), "display forwarded {secret}");
		assert!(std::str::from_utf8(document.original_bytes())?.contains(secret));
	}
	let decoded: ReplayProjection = serde_json::from_str(&rendered_payload)?;
	let DisplayContent::Message { message } = &decoded.entries[1].content else {
		panic!("serialized assistant missing");
	};
	assert!(matches!(message.blocks[1], DisplayBlock::RedactedThinking));
	assert!(matches!(&message.blocks[2], DisplayBlock::ToolCall { arguments: Some(arguments), .. }
		if arguments.get().contains("1844674407370955161999")));
	assert_eq!(document.export_v3()?, REPLAY);
	Ok(())
}

#[test]
fn future_records_and_malformed_content_do_not_block_known_rows_or_expose_payloads() -> Result<()> {
	let document = SessionDocument::decode(REPLAY)?;
	let projection = project_branch(&document, "other")?;
	assert_eq!(projection.entries.len(), 2);
	assert!(
		matches!(&projection.entries[1].content, DisplayContent::Unknown { entry_type } if entry_type == "future-entry")
	);
	assert!(!projection.entries[1].is_visible());
	assert!(!serde_json::to_string(&projection)?.contains("unknown-entry-secret"));
	let malformed = br#"{"type":"session","version":3,"id":"s"}
{"type":"message","id":"bad","parentId":null,"message":{"role":"assistant","content":[null,3,{"type":"text","text":false},{"type":"text","text":"known body"},{"type":"thinking","thinking":"known reasoning"},{"type":"image","data":false}]}}
"#;
	let document = SessionDocument::decode(malformed)?;
	let projection = project_branch(&document, "bad")?;
	let DisplayContent::Message { message } = &projection.entries[0].content else {
		panic!("malformed optional content hid a valid message");
	};
	assert!(matches!(&message.blocks[1], DisplayBlock::Text { text } if text == "known body"));
	assert!(
		matches!(&message.blocks[2], DisplayBlock::Thinking { thinking } if thinking == "known reasoning")
	);
	assert!(
		matches!(&message.blocks[3], DisplayBlock::Unknown { block_type } if block_type == "image")
	);
	assert_eq!(document.export_v3()?, malformed);
	Ok(())
}

#[test]
fn historical_executions_and_hidden_custom_messages_have_explicit_display_behavior() -> Result<()> {
	let body = br#"{"type":"session","version":3,"id":"s"}
{"type":"message","id":"sh","parentId":null,"message":{"role":"bashExecution","command":"exit 7","output":"stopped","exitCode":7,"cancelled":false}}
{"type":"message","id":"py","parentId":"sh","message":{"role":"pythonExecution","code":"print(42)","output":"42","cancelled":true,"truncated":true}}
{"type":"message","id":"js","parentId":"py","message":{"role":"jsExecution","code":"40+2","output":"42","exitCode":0}}
{"type":"message","id":"hidden","parentId":"js","message":{"role":"custom","display":false,"content":"private-hidden-context"}}
{"type":"model_change","id":"model","parentId":"hidden","model":"provider/model","role":"main"}
{"type":"thinking_level_change","id":"level","parentId":"model","thinkingLevel":null}
"#;
	let document = SessionDocument::decode(body)?;
	let projection = project_branch(&document, "level")?;
	for (entry, language, code, exit) in [
		(&projection.entries[0], ExecutionLanguage::Shell, "exit 7", Some(7)),
		(&projection.entries[1], ExecutionLanguage::Python, "print(42)", None),
		(&projection.entries[2], ExecutionLanguage::Javascript, "40+2", Some(0)),
	] {
		let DisplayContent::Message { message } = &entry.content else {
			panic!("historical execution missing");
		};
		let execution = message.execution.as_ref().expect("execution view");
		assert_eq!(execution.language, language);
		assert_eq!(execution.code, code);
		assert_eq!(execution.exit_code, exit);
	}
	assert!(!serde_json::to_string(&projection)?.contains("private-hidden-context"));
	assert!(
		matches!(&projection.entries[4].content, DisplayContent::ModelChange { model, role: Some(role) }
		if model == "provider/model" && role == "main")
	);
	assert!(matches!(&projection.entries[5].content, DisplayContent::ThinkingLevelChange {
		thinking_level: None,
	}));
	Ok(())
}

#[test]
fn selecting_missing_or_cyclic_branches_returns_errors_instead_of_partial_views() -> Result<()> {
	let document = SessionDocument::decode(REPLAY)?;
	assert!(project_branch(&document, "missing").is_err());
	let cycle = br#"{"type":"session","id":"s","version":3}
{"type":"message","id":"a","parentId":"b"}
{"type":"message","id":"b","parentId":"a"}
"#;
	assert!(project_branch(&SessionDocument::decode(cycle)?, "a").is_err());
	Ok(())
}
