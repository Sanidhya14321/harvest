//! Thin N-API adapters for the shared terminal input protocol library.
use harvest_tui::keys::{self, KeyEventType as NativeEventType};
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

/// Match Kitty input against a codepoint and modifier mask.
#[napi]
pub fn matches_kitty_sequence(
	data: JsString,
	expected_codepoint: i32,
	expected_modifier: u32,
) -> Result<bool> {
	let data = js::utf8(data)?;
	Ok(keys::matches_kitty_sequence(data.as_bytes(), expected_codepoint, expected_modifier))
}

/// Parse terminal input into a normalized key identifier.
#[napi]
pub fn parse_key(data: JsString, kitty_protocol_active: bool) -> Result<Option<String>> {
	let data = js::utf8(data)?;
	Ok(keys::parse_key(data.as_bytes(), kitty_protocol_active).map(|key| key.into_owned()))
}

/// Match an exact legacy escape sequence against a key name.
#[napi]
pub fn matches_legacy_sequence(data: JsString, key_name: JsString) -> Result<bool> {
	let data = js::utf8(data)?;
	let key_name = js::utf8(key_name)?;
	Ok(keys::matches_legacy_sequence(data.as_bytes(), &key_name))
}

/// Match terminal input against a normalized key identifier.
#[napi]
pub fn matches_key(data: JsString, key_id: JsString, kitty_protocol_active: bool) -> Result<bool> {
	let data = js::utf8(data)?;
	let key_id = js::utf8(key_id)?;
	Ok(keys::matches_key(data.as_bytes(), &key_id, kitty_protocol_active))
}

/// Decode a complete Kitty keyboard protocol sequence.
#[napi]
pub fn parse_kitty_sequence(data: JsString) -> Result<Option<ParsedKittyResult>> {
	let data = js::utf8(data)?;
	Ok(keys::parse_kitty_sequence(data.as_bytes()).map(|parsed| ParsedKittyResult {
		codepoint:       parsed.codepoint,
		shifted_key:     parsed.shifted_key,
		base_layout_key: parsed.base_layout_key,
		modifier:        parsed.modifier,
		event_type:      parsed.event_type.map(|event| match event {
			NativeEventType::Press => KeyEventType::Press,
			NativeEventType::Repeat => KeyEventType::Repeat,
			NativeEventType::Release => KeyEventType::Release,
		}),
	}))
}

/// Decode text-producing Kitty CSI-u or xterm modifyOtherKeys input.
#[napi]
pub fn decode_printable_key(data: JsString) -> Result<Option<String>> {
	let data = js::utf8(data)?;
	Ok(keys::decode_printable_key(data.as_bytes()))
}

/// Match editor input in one bridge call, including keypad and Backspace
/// policy.
#[napi]
pub fn matches_input_key(
	data: JsString,
	key_id: JsString,
	kitty_protocol_active: bool,
	raw_backspace_is_ctrl: bool,
) -> Result<bool> {
	let data = js::utf8(data)?;
	let key_id = js::utf8(key_id)?;
	Ok(keys::matches_input_key(data.as_bytes(), &key_id, keys::InputMode {
		kitty_protocol_active,
		raw_backspace_is_ctrl,
	}))
}

/// Decode the keypad text fast path before matching navigation bindings.
#[napi]
pub fn decode_kitty_keypad_text(data: JsString) -> Result<Option<String>> {
	let data = js::utf8(data)?;
	Ok(keys::decode_kitty_keypad_text(data.as_bytes()))
}

/// Normalize editor input in one bridge call with explicit terminal facts.
#[napi]
pub fn parse_input_key(
	data: JsString,
	kitty_protocol_active: bool,
	raw_backspace_is_ctrl: bool,
) -> Result<Option<String>> {
	let data = js::utf8(data)?;
	Ok(keys::parse_input_key(data.as_bytes(), keys::InputMode {
		kitty_protocol_active,
		raw_backspace_is_ctrl,
	})
	.map(|key| key.into_owned()))
}

/// Extract text for insertion without replaying terminal control sequences.
#[napi]
pub fn extract_printable_text(data: JsString) -> Result<Option<String>> {
	let data = js::utf8(data)?;
	Ok(keys::extract_printable_text(&data).map(|text| text.into_owned()))
}

/// Decode tmux-reencoded paste control bytes before editor sanitization.
#[napi]
pub fn decode_reencoded_paste_controls(data: JsString) -> Result<String> {
	let data = js::utf8(data)?;
	Ok(harvest_tui::paste::decode_reencoded_controls(&data).into_owned())
}
