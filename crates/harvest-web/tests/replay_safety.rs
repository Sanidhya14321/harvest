use harvest_protocol::{display::project_branch, session::SessionDocument};
use harvest_web::{image_source, searchable_text};

#[test]
fn replay_rejects_active_image_formats_urls_and_invalid_inline_data() {
	assert_eq!(image_source("image/png", "YWJj").as_deref(), Some("data:image/png;base64,YWJj"));
	for (mime, data) in [
		("image/svg+xml", "PHN2Zz4="),
		("text/html", "YWJj"),
		("image/png", "https://example.invalid/image.png"),
		("image/jpeg", "not base64"),
	] {
		assert!(image_source(mime, data).is_none());
	}
	assert!(image_source("image/png", &"A".repeat(8 * 1024 * 1024 + 4)).is_none());
}

#[test]
fn browser_search_only_includes_visible_branch_content_and_lossless_tool_arguments()
-> Result<(), Box<dyn std::error::Error>> {
	let document = SessionDocument::decode(include_bytes!("fixtures/replay.jsonl"))?;
	let projection = project_branch(&document, "visible")?;
	let searchable = projection
		.entries
		.iter()
		.map(searchable_text)
		.collect::<Vec<_>>()
		.join("\n");
	assert!(searchable.contains("1844674407370955161999"));
	assert!(searchable.contains("40 + 2"));
	assert!(searchable.contains("42"));
	for secret in [
		"provider-secret",
		"thinking-secret",
		"hidden-body-secret",
		"extension-secret",
		"unknown-entry-secret",
	] {
		assert!(!searchable.contains(secret));
	}
	Ok(())
}
