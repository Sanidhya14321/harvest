//! Stable transcript queries shared by native protocol consumers.
//! Pages retain original message payloads; they are never display projections.
use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};

pub const PAGE_BYTES: usize = 768 * 1024;
const SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
	pub session_id:    String,
	pub leaf_id:       Option<String>,
	pub message_count: usize,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
	version:  u8,
	#[serde(flatten)]
	snapshot: Snapshot,
	offset:   usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<'a> {
	pub messages:       &'a [Box<RawValue>],
	#[serde(skip_serializing_if = "Option::is_none")]
	pub next_cursor:    Option<String>,
	pub total_messages: usize,
}

/// Explicit stale-cursor errors allow clients to restart pagination.
#[derive(Debug)]
pub struct StaleCursor;
impl std::fmt::Display for StaleCursor {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("RPC message cursor is stale")
	}
}
impl std::error::Error for StaleCursor {}

fn decode(cursor: &str) -> Result<Cursor> {
	let invalid = || anyhow::anyhow!("Invalid RPC message cursor");
	ensure!(!cursor.is_empty() && cursor.len() <= 2048, "Invalid RPC message cursor");
	let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
	ensure!(URL_SAFE_NO_PAD.encode(&bytes) == cursor, "Invalid RPC message cursor");
	let mut json: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
	ensure!(json.get("leafId").is_some(), "Invalid RPC message cursor");
	for key in ["version", "messageCount", "offset"] {
		let integer = safe_integer(&json[key]).ok_or_else(invalid)?;
		json[key] = Value::from(integer);
	}
	let value: Cursor = serde_json::from_value(json).map_err(|_| invalid())?;
	let valid_id = |id: &str| !id.is_empty() && id.encode_utf16().count() <= 256;
	ensure!(
		value.version == 1
			&& valid_id(&value.snapshot.session_id)
			&& value.snapshot.leaf_id.as_deref().is_none_or(valid_id)
			&& u64::try_from(value.snapshot.message_count)? <= SAFE_INTEGER
			&& value.offset <= value.snapshot.message_count,
		"Invalid RPC message cursor"
	);
	Ok(value)
}

/// JSON numbers such as `1.0` and `1e0` are whole numbers in the JS protocol.
pub fn safe_integer(value: &Value) -> Option<u64> {
	if let Some(integer) = value.as_u64() {
		return (integer <= SAFE_INTEGER).then_some(integer);
	}
	let number = value.as_f64()?;
	if !number.is_finite() || number < 0.0 || number.fract().abs() > 0.0 {
		return None;
	}
	let integer: u64 = format!("{:.0}", number.abs()).parse().ok()?;
	(integer <= SAFE_INTEGER).then_some(integer)
}

/// Preserve legacy page sizes, cursor shape and oversized-single-message
/// policy. The transport must reject an oversized v1 frame explicitly, never
/// truncate it.
pub fn page<'a>(
	messages: &'a [Box<RawValue>],
	snapshot: &Snapshot,
	cursor: Option<&str>,
	limit: Option<usize>,
) -> Result<Page<'a>> {
	ensure!(
		snapshot.message_count == messages.len(),
		"RPC message snapshot does not match current messages"
	);
	let limit = limit.unwrap_or(100);
	ensure!((1..=256).contains(&limit), "RPC message page limit must be between 1 and 256");
	let offset = if let Some(cursor) = cursor {
		let cursor = decode(cursor)?;
		if cursor.snapshot != *snapshot {
			bail!(StaleCursor);
		}
		cursor.offset
	} else {
		0
	};
	let mut end = offset;
	let mut bytes = 2_usize;
	while end < messages.len() && end - offset < limit {
		// Compact JSON determines the legacy byte budget; return the lossless raw
		// record even when it contains formatting or opaque provider fields.
		let value: Value = serde_json::from_str(messages[end].get())?;
		let size = serde_json::to_vec(&value)?
			.len()
			.saturating_add(usize::from(end != offset));
		if end > offset && bytes.saturating_add(size) > PAGE_BYTES {
			break;
		}
		bytes = bytes.saturating_add(size);
		end += 1;
	}
	let next_cursor = if end < messages.len() {
		Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Cursor {
			version:  1,
			snapshot: snapshot.clone(),
			offset:   end,
		})?))
	} else {
		None
	};
	Ok(Page { messages: &messages[offset..end], next_cursor, total_messages: messages.len() })
}

/// Last copy candidate: skip only completely empty aborted assistants. A later
/// tool-only or blank assistant does not fall back to older visible text.
pub fn last_assistant_text(messages: &[Box<RawValue>]) -> Result<Option<String>> {
	for message in messages.iter().rev() {
		let message: Value = serde_json::from_str(message.get())?;
		if message["role"] != "assistant" {
			continue;
		}
		let Some(content) = message["content"].as_array() else {
			continue;
		};
		if message["stopReason"] == "aborted" && content.is_empty() {
			continue;
		}
		let text: String = content
			.iter()
			.filter(|item| item["type"] == "text")
			.filter_map(|item| item["text"].as_str())
			.collect();
		// ECMAScript trim includes the BOM and excludes NEL (U+0085).
		let text =
			text.trim_matches(|ch: char| ch == '\u{feff}' || (ch.is_whitespace() && ch != '\u{85}'));
		return Ok((!text.is_empty()).then(|| text.to_owned()));
	}
	Ok(None)
}
