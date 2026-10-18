use anyhow::Result;
use harvest_protocol::{
	display::{DisplayBlock, DisplayContent, project_branch},
	session::SessionDocument,
};

#[test]
fn embedded_replay_cannot_close_its_data_script_or_treat_snapshot_markers_as_templates()
-> Result<()> {
	let document = SessionDocument::decode(include_bytes!(
		"../../crates/harvest-protocol/tests/fixtures/replay.jsonl"
	))?;
	let mut projection = project_branch(&document, "visible")?;
	let hostile = "</script><script>globalThis.injection=1</script><img src=x \
	               onerror=alert(1)><!--HARVEST_BOOTSTRAP-->\u{2028}";
	let message = projection
		.entries
		.iter_mut()
		.find_map(|entry| {
			if let DisplayContent::Message { message } = &mut entry.content {
				Some(message)
			} else {
				None
			}
		})
		.unwrap();
	message
		.blocks
		.push(DisplayBlock::Text { text: hostile.into() });
	let html = xtask::web_replay::render(
		&projection,
		"const wasm_bindgen=()=>Promise.resolve();",
		b"\0asm\x01\0\0\0",
	)?;
	let data = html
		.split_once("id=\"harvest-replay-data\">")
		.unwrap()
		.1
		.split_once("</script>")
		.unwrap()
		.0;
	assert!(!data.contains('<'));
	assert!(!data.contains('\u{2028}'));
	let decoded: serde_json::Value = serde_json::from_str(data)?;
	assert_eq!(decoded, serde_json::to_value(&projection)?);
	assert!(!html.contains("<script>globalThis.injection=1"));
	Ok(())
}

#[test]
fn invalid_assets_cannot_generate_a_seemingly_complete_export() -> Result<()> {
	let document = SessionDocument::decode(include_bytes!(
		"../../crates/harvest-protocol/tests/fixtures/replay.jsonl"
	))?;
	let projection = project_branch(&document, "visible")?;
	assert!(
		xtask::web_replay::render(&projection, "bindings", b"<html>missing asset</html>").is_err()
	);
	assert!(
		xtask::web_replay::render(&projection, "</ScRiPt><script>bad()", b"\0asm\x01\0\0\0").is_err()
	);
	Ok(())
}
