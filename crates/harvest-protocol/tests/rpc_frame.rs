use base64::{Engine, engine::general_purpose::STANDARD};
use harvest_protocol::{
	digest::content_hash,
	rpc_frame::{self, CHUNK_BYTES, Decoder, MAX_FRAME_BYTES, MAX_LOGICAL_BYTES, Version},
};
use serde_json::{Value, json};

fn lines(value: &Value) -> Vec<Vec<u8>> {
	rpc_frame::encode(value, Version::V2, "rpc-1".into())
		.unwrap()
		.collect::<anyhow::Result<_>>()
		.unwrap()
}
fn large() -> Value {
	json!({"data":"😀".repeat(300_000),"type":"response"})
}
fn chunk(index: usize, data: &[u8]) -> Value {
	json!({"type":"rpc_chunk","chunkId":"test","count":2,"index":index,"byteLength":MAX_FRAME_BYTES,"data":STANDARD.encode(data)})
}
fn push(decoder: &mut Decoder, value: &Value) -> anyhow::Result<Option<Value>> {
	decoder.push(&serde_json::to_vec(value).unwrap())
}

#[test]
fn physical_lines_match_captured_legacy_wire_bytes_at_boundaries_and_across_unicode() {
	let cases: Vec<Value> =
		serde_json::from_str(include_str!("fixtures/rpc-transport.json")).unwrap();
	for case in cases {
		let repetitions = usize::try_from(case["repetitions"].as_u64().unwrap()).unwrap();
		let frame = json!({"command":"get_messages","data":{"payload":case["glyph"].as_str().unwrap().repeat(repetitions)},"id":"oracle","success":true,"type":"response"});
		let logical = serde_json::to_vec(&frame).unwrap();
		assert_eq!(logical.len(), usize::try_from(case["logicalBytes"].as_u64().unwrap()).unwrap());
		assert_eq!(content_hash(&logical), case["logicalHash"]);
		let lines = lines(&frame);
		assert_eq!(lines.len(), case["lines"].as_array().unwrap().len());
		let mut decoder = Decoder::new(Version::V2);
		let mut result = None;
		for (line, expected) in lines.iter().zip(case["lines"].as_array().unwrap()) {
			assert!(line.len() <= MAX_FRAME_BYTES);
			assert_eq!(line.len(), usize::try_from(expected["bytes"].as_u64().unwrap()).unwrap());
			assert_eq!(content_hash(line), expected["hash"], "{}", case["name"]);
			result = decoder.push(line).unwrap();
		}
		assert_eq!(result, Some(frame));
		decoder.finish().unwrap();
	}
}

#[test]
fn v1_refuses_large_messages_v2_preserves_them_without_loss() {
	let value = large();
	assert!(rpc_frame::encode(&value, Version::V1, "ignored".into()).is_err());
	let mut decoder = Decoder::default();
	assert!(
		decoder
			.push(&lines(&value)[0])
			.unwrap_err()
			.to_string()
			.contains("before protocol negotiation")
	);
	let mut decoder = Decoder::default();
	decoder.set_version(Version::V2).unwrap();
	let mut result = None;
	for line in lines(&value) {
		result = decoder.push(&line).unwrap();
	}
	assert_eq!(result, Some(value));
}

#[test]
fn metadata_base64_and_physical_limits_are_checked_before_reassembly() {
	let original = chunk(0, b"{");
	for (field, bad) in [
		("chunkId", json!("")),
		("chunkId", json!("x".repeat(129))),
		("index", json!(-1)),
		("index", json!(0.5)),
		("index", json!(2)),
		("count", json!(1)),
		("count", json!(257)),
		("count", json!(9007199254740992_u64)),
		("byteLength", json!(MAX_FRAME_BYTES - 1)),
		("byteLength", json!(MAX_LOGICAL_BYTES + 1)),
		("data", json!("")),
		("data", json!("Zh==")),
		("data", json!("ew")),
		("data", json!("e w==")),
		("data", json!(STANDARD.encode(vec![0; CHUNK_BYTES + 1]))),
	] {
		let mut value = original.clone();
		value[field] = bad;
		assert!(push(&mut Decoder::new(Version::V2), &value).is_err(), "{field}");
	}
	assert!(
		Decoder::new(Version::V2)
			.push(&vec![b' '; MAX_FRAME_BYTES + 1])
			.is_err()
	);
	for value in [json!(null), json!([]), json!("frame")] {
		assert!(push(&mut Decoder::new(Version::V2), &value).is_err());
	}
}

#[test]
fn broken_sequences_poison_the_stream_and_release_partial_payloads() {
	for bad in [
		json!({"type":"response"}),
		chunk(0, b"{"),
		{
			let mut wrong = chunk(1, b"}");
			wrong["chunkId"] = json!("other");
			wrong
		},
		{
			let mut wrong = chunk(1, b"}");
			wrong["count"] = json!(3);
			wrong
		},
		chunk(1, b"}"),
	] {
		let mut decoder = Decoder::new(Version::V2);
		assert!(push(&mut decoder, &chunk(0, b"{")).unwrap().is_none());
		assert!(decoder.set_version(Version::V1).is_err());
		assert!(push(&mut decoder, &bad).is_err());
		assert!(push(&mut decoder, &json!({"type":"ready"})).is_err());
		assert!(decoder.finish().is_err());
	}
	let mut decoder = Decoder::new(Version::V2);
	assert!(push(&mut decoder, &chunk(1, b"}")).is_err());
	let mut decoder = Decoder::new(Version::V2);
	push(&mut decoder, &chunk(0, b"{")).unwrap();
	assert!(decoder.finish().is_err());
}

#[test]
fn invalid_logical_utf8_json_and_nonobjects_are_rejected() {
	for bytes in [
		vec![b' '; MAX_FRAME_BYTES],
		vec![0xff; MAX_FRAME_BYTES],
		format!("\"{}\"", "x".repeat(MAX_FRAME_BYTES - 2)).into_bytes(),
	] {
		let count = bytes.len().div_ceil(CHUNK_BYTES);
		let mut decoder = Decoder::new(Version::V2);
		for (index, part) in bytes.chunks(CHUNK_BYTES).enumerate() {
			let frame = json!({"type":"rpc_chunk","chunkId":"invalid","index":index,"count":count,"byteLength":bytes.len(),"data":STANDARD.encode(part)});
			let result = push(&mut decoder, &frame);
			if index + 1 == count {
				assert!(result.is_err());
			} else {
				assert!(result.unwrap().is_none());
			}
		}
	}
}

#[test]
fn logical_ceiling_is_enforced_during_serialization() {
	let payload = "x".repeat(MAX_LOGICAL_BYTES);
	assert!(rpc_frame::encode(&json!({"payload":payload}), Version::V2, "limit".into()).is_err());
}
