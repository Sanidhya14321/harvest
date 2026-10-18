//! Thin N-API bindings for the shared legacy and Kitty keyboard decoder.
use harvest_terminal::keys;
use napi::{JsString, Result};
use napi_derive::napi;

use crate::js;

/// Event types from Kitty keyboard protocol (flag 2).
#[napi]
pub enum KeyEventType {
	/// Key press event.
	Press   = 1,
	/// Key repeat event.
	Repeat  = 2,
	/// Key release event.
	Release = 3,
}

impl From<keys::KeyEventType> for KeyEventType {
	fn from(event: keys::KeyEventType) -> Self {
		match event {
			keys::KeyEventType::Press => Self::Press,
			keys::KeyEventType::Repeat => Self::Repeat,
			keys::KeyEventType::Release => Self::Release,
		}
	}
}

/// Parsed Kitty keyboard protocol sequence result for a Kitty input sequence.
#[napi(object)]
pub struct ParsedKittyResult {
	/// Primary codepoint associated with the key.
	pub codepoint:       i32,
	/// Optional shifted key codepoint from the sequence.
	pub shifted_key:     Option<i32>,
	/// Optional base layout key codepoint from the sequence.
	pub base_layout_key: Option<i32>,
	/// Modifier bitmask (shift/alt/ctrl), excluding lock bits.
	pub modifier:        u32,
	/// Optional event type (1 = press, 2 = repeat, 3 = release).
	pub event_type:      Option<KeyEventType>,
}

impl From<keys::ParsedKittyResult> for ParsedKittyResult {
	fn from(parsed: keys::ParsedKittyResult) -> Self {
		Self {
			codepoint:       parsed.codepoint,
			shifted_key:     parsed.shifted_key,
			base_layout_key: parsed.base_layout_key,
			modifier:        parsed.modifier,
			event_type:      parsed.event_type.map(KeyEventType::from),
		}
	}
}

/// Match Kitty protocol input against a codepoint and modifier mask.
#[napi]
pub fn matches_kitty_sequence(
	data: JsString,
	expected_codepoint: i32,
	expected_modifier: u32,
) -> Result<bool> {
	let data = js::utf8(data)?;
	Ok(keys::matches_kitty_sequence(data.as_bytes(), expected_codepoint, expected_modifier))
}

/// Parse terminal input and return a normalized key identifier.
#[napi]
pub fn parse_key(data: JsString, kitty_protocol_active: bool) -> Result<Option<String>> {
	let data = js::utf8(data)?;
	Ok(keys::parse_key(data.as_bytes(), kitty_protocol_active).map(std::borrow::Cow::into_owned))
}

/// Check if input matches a legacy escape sequence for the given key name.
#[napi]
pub fn matches_legacy_sequence(data: JsString, key_name: JsString) -> Result<bool> {
	let data = js::utf8(data)?;
	let key_name = js::utf8(key_name)?;
	Ok(keys::matches_legacy_sequence(data.as_bytes(), &key_name))
}

/// Match input data against a key identifier string.
#[napi]
pub fn matches_key(data: JsString, key_id: JsString, kitty_protocol_active: bool) -> Result<bool> {
	let data = js::utf8(data)?;
	let key_id = js::utf8(key_id)?;
	Ok(keys::matches_key(data.as_bytes(), &key_id, kitty_protocol_active))
}

/// Parse a Kitty keyboard protocol sequence.
#[napi]
pub fn parse_kitty_sequence(data: JsString) -> Result<Option<ParsedKittyResult>> {
	let data = js::utf8(data)?;
	Ok(keys::parse_kitty_sequence(data.as_bytes()).map(ParsedKittyResult::from))
}
