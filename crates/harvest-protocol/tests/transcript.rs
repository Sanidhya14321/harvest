use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use harvest_protocol::transcript::{self, Snapshot, StaleCursor};
use serde_json::{Value, json, value::RawValue};

fn raw(value: Value) -> Box<RawValue> {
	serde_json::value::to_raw_value(&value).unwrap()
}

#[test]
fn captured_legacy_pages_preserve_cursor_bytes_payloads_and_errors() {
	let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/rpc-pages.json")).unwrap();
	for case in cases {
		let messages: Vec<_> = case["messages"]
			.as_array()
			.unwrap()
			.iter()
			.cloned()
			.map(raw)
			.collect();
		let snapshot: Snapshot = serde_json::from_value(case["snapshot"].clone()).unwrap();
		let result = transcript::page(
			&messages,
			&snapshot,
			case["options"]["cursor"].as_str(),
			case["options"]["limit"]
				.as_u64()
				.map(|n| usize::try_from(n).unwrap()),
		);
		if let Some(error) = case["error"].as_str() {
			let failure = result.unwrap_err();
			assert_eq!(failure.to_string(), error, "{}", case["name"]);
			assert_eq!(failure.is::<StaleCursor>(), case["code"] == "stale_cursor");
		} else {
			assert_eq!(
				serde_json::to_value(result.unwrap()).unwrap(),
				case["expected"],
				"{}",
				case["name"]
			);
		}
	}
}

#[test]
fn large_history_pages_have_no_loss_overlap_or_rewritten_provider_fields() {
	let messages: Vec<_> = (0..40)
		.map(|i| {
			raw(
				json!({"role":"user","content":format!("{i}:{}", "🦀".repeat(8192)),"opaque":{"index":i}}),
			)
		})
		.collect();
	let snapshot = Snapshot {
		session_id:    "large".into(),
		leaf_id:       Some("leaf".into()),
		message_count: messages.len(),
	};
	let mut cursor = None;
	let mut collected = Vec::new();
	let mut pages = 0;
	loop {
		let page = transcript::page(&messages, &snapshot, cursor.as_deref(), Some(256)).unwrap();
		assert!(serde_json::to_vec(&page).unwrap().len() < 1024 * 1024);
		collected.extend(page.messages.iter().map(|message| message.get().to_owned()));
		pages += 1;
		cursor = page.next_cursor;
		if cursor.is_none() {
			break;
		}
	}
	assert!(pages > 1);
	assert_eq!(
		collected,
		messages
			.iter()
			.map(|message| message.get().to_owned())
			.collect::<Vec<_>>()
	);
}

#[test]
fn oversized_single_message_is_not_truncated_and_snapshot_count_is_checked() {
	let messages =
		vec![raw(json!({"content":"x".repeat(2*1024*1024)})), raw(json!({"content":"next"}))];
	let mut snapshot =
		Snapshot { session_id: "large".into(), leaf_id: None, message_count: 2 };
	let first = transcript::page(&messages, &snapshot, None, None).unwrap();
	assert_eq!(first.messages.len(), 1);
	assert_eq!(first.messages[0].get(), messages[0].get());
	let next = transcript::page(&messages, &snapshot, first.next_cursor.as_deref(), None).unwrap();
	assert_eq!(next.messages[0].get(), messages[1].get());
	assert!(next.next_cursor.is_none());
	snapshot.message_count = 1;
	assert!(transcript::page(&messages, &snapshot, None, None).is_err());
}

#[test]
fn cursor_validation_rejects_wrong_types_unsafe_counts_and_noncanonical_encoding() {
	assert_eq!(transcript::safe_integer(&json!(-0.0)), Some(0));
	assert_eq!(transcript::safe_integer(&json!(1.0)), Some(1));
	assert_eq!(transcript::safe_integer(&json!(1.5)), None);
	let snapshot = Snapshot { session_id: "s".into(), leaf_id: None, message_count: 0 };
	for value in [
		json!({"version":2,"sessionId":"s","leafId":null,"messageCount":0,"offset":0}),
		json!({"version":1,"sessionId":"","leafId":null,"messageCount":0,"offset":0}),
		json!({"version":1,"sessionId":"s","leafId":"","messageCount":0,"offset":0}),
		json!({"version":1,"sessionId":"s","leafId":null,"messageCount":9007199254740992_u64,"offset":0}),
		json!({"version":1,"sessionId":"s","leafId":null,"messageCount":0,"offset":1}),
		json!({"version":1,"sessionId":"s","messageCount":0,"offset":0}),
	] {
		let cursor = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap());
		assert_eq!(
			transcript::page(&[], &snapshot, Some(&cursor), None)
				.unwrap_err()
				.to_string(),
			"Invalid RPC message cursor"
		);
	}
	for cursor in ["", "AA=", "Zh", "_w", &"x".repeat(2049)] {
		assert!(transcript::page(&[], &snapshot, Some(cursor), None).is_err());
	}
}

#[test]
fn last_assistant_copy_rules_preserve_text_blocks_and_empty_abort_fallback() {
	let visible = raw(
		json!({"role":"assistant","content":[{"type":"text","text":"\u{feff} first"},{"type":"thinking","thinking":"secret"},{"type":"text","text":" second \n"}]}),
	);
	let empty_abort = raw(json!({"role":"assistant","stopReason":"aborted","content":[]}));
	assert_eq!(
		transcript::last_assistant_text(&[visible.clone(), empty_abort])
			.unwrap()
			.as_deref(),
		Some("first second")
	);
	for content in
		[json!([]), json!([{"type":"toolCall","name":"tool"}]), json!([{"type":"text","text":" \n"}])]
	{
		assert!(
			transcript::last_assistant_text(&[
				visible.clone(),
				raw(json!({"role":"assistant","content":content}))
			])
			.unwrap()
			.is_none()
		);
	}
	assert_eq!(
		transcript::last_assistant_text(&[raw(
			json!({"role":"assistant","content":[{"type":"text","text":"\u{85}"}]})
		)])
		.unwrap()
		.as_deref(),
		Some("\u{85}")
	);
	assert!(transcript::last_assistant_text(&[]).unwrap().is_none());
}
