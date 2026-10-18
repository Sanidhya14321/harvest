use harvest_protocol::{display::project_branch, session::SessionDocument};
use harvest_terminal::{
	layout::PlainTextLayout,
	replay::{ReplayError, ReplayLimits, render_projection, render_projection_bounded},
};

#[test]
fn selected_branch_renderer_pairs_tools_and_renders_history_without_provider_or_image_payloads()
-> Result<(), Box<dyn std::error::Error>> {
	let document = SessionDocument::decode(include_bytes!("fixtures/replay.jsonl"))?;
	let projection = project_branch(&document, "visible")?;
	let rows = render_projection(&projection, 200, &mut PlainTextLayout::default())?;
	let transcript = rows.join("\n");
	for visible in [
		"Tool call: eval [call-js]",
		"Tool result: eval [call-js]",
		"1844674407370955161999",
		"42",
		"[Image: image/png]",
		"Context compaction",
		"Branch summary",
	] {
		assert!(transcript.contains(visible), "missing projected display content {visible}");
	}
	for private in [
		"provider-secret",
		"thinking-secret",
		"redacted-secret",
		"unknown-block-secret",
		"extension-secret",
		"preserved-secret",
		"hidden-body-secret",
		"unknown-entry-secret",
		"YWJj",
	] {
		assert!(!transcript.contains(private), "terminal leaked {private}");
	}
	assert_eq!(document.export_v3()?, document.original_bytes());
	Ok(())
}

#[test]
fn terminal_replay_renders_control_characters_visibly_in_headings_bodies_and_tool_arguments()
-> Result<(), Box<dyn std::error::Error>> {
	let source = br#"{"type":"session","version":3,"id":"s"}
{"type":"message","id":"a","parentId":null,"message":{"role":"assistant\u001b","content":[{"type":"text","text":"body\u001b[3J\tline"},{"type":"toolCall","id":"call\u001b","name":"tool\u001b","arguments":{"value":"arg\u001b"}}]}}
"#;
	let document = SessionDocument::decode(source)?;
	let projection = project_branch(&document, "a")?;
	let rows = render_projection(&projection, 100, &mut PlainTextLayout::new(4))?;
	let transcript = rows.join("\n");
	assert!(transcript.contains("assistant␛"));
	assert!(transcript.contains("body␛[3J    line"));
	assert!(transcript.contains("Tool call: tool␛ [call␛]"));
	assert!(transcript.contains(r#"{"value":"arg\u001b"}"#));
	assert!(
		rows
			.iter()
			.all(|row| !row.contains('\x1b') && !row.contains('\t'))
	);
	assert_eq!(document.export_v3()?, source);
	Ok(())
}

fn text_projection(
	text: &str,
) -> Result<harvest_protocol::display::ReplayProjection, Box<dyn std::error::Error>> {
	let body = serde_json::json!({
		"type": "message", "id": "a", "parentId": null,
		"message": {"role": "assistant", "content": [{"type": "text", "text": text}]}
	});
	let source = format!("{{\"type\":\"session\",\"version\":3,\"id\":\"s\"}}\n{body}\n");
	Ok(project_branch(&SessionDocument::decode(source.as_bytes())?, "a")?)
}

#[test]
fn oversized_session_text_is_rejected_before_layout_and_no_partial_transcript_is_returned()
-> Result<(), Box<dyn std::error::Error>> {
	let projection = text_projection(&"x".repeat(64 * 1024 + 1))?;
	assert_eq!(
		render_projection(&projection, 1, &mut PlainTextLayout::default()),
		Err(ReplayError::LayoutChunkLimit)
	);
	let limits = ReplayLimits { text_bytes: 32, ..ReplayLimits::default() };
	assert_eq!(
		render_projection_bounded(
			&text_projection("small body over the total text budget")?,
			1,
			&mut PlainTextLayout::default(),
			limits
		),
		Err(ReplayError::TextLimit)
	);
	Ok(())
}

#[test]
fn tab_expansion_and_large_layout_output_hit_row_or_byte_caps_without_partial_success()
-> Result<(), Box<dyn std::error::Error>> {
	let projection = text_projection(&"\tx".repeat(256))?;
	let limits = ReplayLimits { rows: 16, ..ReplayLimits::default() };
	assert_eq!(
		render_projection_bounded(&projection, 1, &mut PlainTextLayout::new(16), limits),
		Err(ReplayError::RowLimit)
	);
	let limits = ReplayLimits { output_bytes: 20, ..ReplayLimits::default() };
	assert_eq!(
		render_projection_bounded(
			&text_projection("output exceeds the byte budget")?,
			100,
			&mut PlainTextLayout::default(),
			limits
		),
		Err(ReplayError::OutputLimit)
	);
	Ok(())
}
