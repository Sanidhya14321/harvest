//! Consumer contracts captured from the legacy renderer, plus bounded failures.
use std::{
	collections::BTreeMap,
	io::{Cursor, Write},
};

use harvest_media::docx::{self, Conversion, Options};
use harvest_runtime::cancel::{AbortReason, CancelToken};

fn archive(parts: &[(&str, &str)]) -> Vec<u8> {
	let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
	for (name, value) in parts {
		zip.start_file(*name, zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(value.as_bytes()).unwrap();
	}
	zip.finish().unwrap().into_inner()
}
fn document(xml: &str) -> Vec<u8> {
	archive(&[("word/document.xml", xml)])
}

#[test]
fn legacy_documents_preserve_html_diagnostics_lists_tables_and_inline_images() {
	let fixtures: &[(&[u8], &str)] = &[
		(
			include_bytes!("fixtures/docx/formatting.docx"),
			include_str!("fixtures/docx/formatting.json"),
		),
		(include_bytes!("fixtures/docx/lists.docx"), include_str!("fixtures/docx/lists.json")),
		(
			include_bytes!("fixtures/docx/tables-links.docx"),
			include_str!("fixtures/docx/tables-links.json"),
		),
		(include_bytes!("fixtures/docx/images.docx"), include_str!("fixtures/docx/images.json")),
	];
	for (input, expected) in fixtures {
		let actual = docx::convert(input, &Options::default(), &CancelToken::default()).unwrap();
		assert_eq!(actual, serde_json::from_str::<Conversion>(expected).unwrap());
	}
}
#[test]
fn image_callbacks_receive_payload_and_control_escaped_attributes() {
	let cancel = CancelToken::default();
	let prepared =
		docx::prepare(include_bytes!("fixtures/docx/images.docx"), &Options::default(), &cancel)
			.unwrap();
	assert_eq!(prepared.images[0].content_type, "image/png");
	assert_eq!(prepared.images[0].alt_text, "Pixel");
	assert!(prepared.images[0].base64.starts_with("iVBORw0KGgo"));
	let attrs = [BTreeMap::from([
		("alt".into(), "image_1".into()),
		("src".into(), "data:image/png;base64,".into()),
	])];
	assert_eq!(
		prepared.finish(Some(&attrs), &cancel).unwrap(),
		serde_json::from_str::<Conversion>(include_str!("fixtures/docx/image-callback.json"))
			.unwrap()
	);
	let attrs = [BTreeMap::from([("src".into(), "a\"<&".into())])];
	assert_eq!(
		prepared.finish(Some(&attrs), &cancel).unwrap().value,
		"<p><img src=\"a&quot;&lt;&amp;\" /></p>"
	);
	assert!(prepared.finish(Some(&[]), &cancel).is_err());
	let attrs = [BTreeMap::from([("src onerror".into(), "bad".into())])];
	assert!(prepared.finish(Some(&attrs), &cancel).is_err());
}
#[test]
fn malformed_archives_entities_and_deep_xml_are_rejected() {
	let cancel = CancelToken::default();
	assert!(docx::convert(b"not a zip", &Options::default(), &cancel).is_err());
	for xml in [
		"<document><body><p></body></document>".to_owned(),
		"<!DOCTYPE document [<!ENTITY x 'expanded'>]><document><body><p>&x;</p></body></document>"
			.to_owned(),
		format!("<document><body>{}x{}</body></document>", "<p>".repeat(70), "</p>".repeat(70)),
	] {
		assert!(docx::convert(&document(&xml), &Options::default(), &cancel).is_err());
	}
}
#[test]
fn cancellation_prevents_conversion_and_callback_completion() {
	let mut cancel = CancelToken::default();
	let abort = cancel.emplace_abort_token();
	let prepared =
		docx::prepare(include_bytes!("fixtures/docx/images.docx"), &Options::default(), &cancel)
			.unwrap();
	abort.abort(AbortReason::User);
	assert!(prepared.finish(None, &cancel).is_err());
	assert!(
		docx::convert(include_bytes!("fixtures/docx/formatting.docx"), &Options::default(), &cancel)
			.is_err()
	);
}
#[test]
fn markdown_reader_omits_embedded_binary_and_preserves_document_text() {
	let value =
		docx::markdown(include_bytes!("fixtures/docx/formatting.docx"), &CancelToken::default())
			.unwrap();
	assert!(value.contains("Title & More"));
	let images =
		docx::markdown(include_bytes!("fixtures/docx/images.docx"), &CancelToken::default()).unwrap();
	assert!(images.contains("Pixel"));
	assert!(!images.contains("base64"));
}

#[test]
fn custom_style_maps_override_headings_and_default_mapping_can_be_disabled() {
	let input = include_bytes!("fixtures/docx/formatting.docx");
	let options = Options {
		style_map: vec!["p[style-name='heading 1'] => h2:fresh".into()],
		..Options::default()
	};
	let converted = docx::convert(input, &options, &CancelToken::default()).unwrap();
	assert!(converted.value.starts_with("<h2>Title &amp; More</h2>"));
	let options = Options { include_default_style_map: false, ..Options::default() };
	let converted = docx::convert(input, &options, &CancelToken::default()).unwrap();
	assert!(converted.value.starts_with("<p>Title &amp; More</p>"));
}

#[test]
fn oversized_declared_zip_members_and_table_spans_fail_before_rendering() {
	let mut input = document("<document><body><p><r><t>small</t></r></p></body></document>");
	let central = input.windows(4).position(|m| m == b"PK\x01\x02").unwrap();
	let advertised = (64_u32 * 1024 * 1024 + 1).to_le_bytes();
	input[central + 24..central + 28].copy_from_slice(&advertised);
	input[22..26].copy_from_slice(&advertised);
	let error = docx::convert(&input, &Options::default(), &CancelToken::default()).unwrap_err();
	assert!(error.to_string().contains("64 MiB"), "{error:#}");
	let input = document(
		"<document><body><tbl><tr><tc><tcPr><gridSpan \
		 val='4097'/></tcPr><p><r><t>x</t></r></p></tc></tr></tbl></body></document>",
	);
	let error = docx::convert(&input, &Options::default(), &CancelToken::default()).unwrap_err();
	assert!(error.to_string().contains("4096 columns"));
}

#[test]
fn cyclic_style_inheritance_terminates_and_explicit_false_overrides_parent_formatting() {
	let input = archive(&[
		(
			"word/document.xml",
			"<document><body><p><r><rPr><rStyle \
			 val='Child'/></rPr><t>inherited</t></r></p></body></document>",
		),
		(
			"word/styles.xml",
			"<styles><style type='character' styleId='Base'><name val='Base'/><basedOn \
			 val='Child'/><rPr><b/></rPr></style><style type='character' styleId='Child'><name \
			 val='Child'/><basedOn val='Base'/><rPr><b val='false'/><i/></rPr></style></styles>",
		),
	]);
	let options =
		Options { style_map: vec!["r[style-name='Child'] => code".into()], ..Options::default() };
	let result = docx::convert(&input, &options, &CancelToken::default()).unwrap();
	assert_eq!(result.value, "<p><code><em>inherited</em></code></p>");
	assert!(result.messages.is_empty());
}
