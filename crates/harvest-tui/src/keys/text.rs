//! Text-producing keyboard encodings share the key parser's numeric boundaries.
use std::borrow::Cow;

use super::{
	CP_KP_0, CP_KP_DECIMAL, KeyEventType, LOCK_MASK, MOD_SHIFT, keypad_num_lock_text_codepoint,
	keypad_operator_text_codepoint, parse_csi_u, parse_modify_other_keys,
};

/// Decode Kitty CSI-u or xterm modifyOtherKeys text. Bindings and release
/// events must never insert characters into the focused editor.
pub fn decode_printable_key(data: &[u8]) -> Option<String> {
	decode_kitty_printable(data).or_else(|| {
		if !data.ends_with(b"~") {
			return None;
		}
		let (modifier, codepoint) = parse_modify_other_keys(data)?;
		if (modifier & !LOCK_MASK) & !MOD_SHIFT != 0 {
			return None;
		}
		Some(printable_scalar(u32::try_from(codepoint).ok()?)?.to_string())
	})
}

/// Decode only the Kitty keypad text fast path used before binding matching.
pub fn decode_kitty_keypad_text(data: &[u8]) -> Option<String> {
	let parsed = csi_u(data)?;
	if !(CP_KP_0..=CP_KP_DECIMAL).contains(&parsed.codepoint)
		&& keypad_operator_text_codepoint(parsed.codepoint).is_none()
	{
		return None;
	}
	decode_kitty_printable(data)
}

/// Accept ordinary UTF-8 text, but keep raw controls and protocol replies out
/// of the text insertion path. Encoded printable input is normalized first.
pub fn extract_printable_text(data: &str) -> Option<Cow<'_, str>> {
	if let Some(decoded) = decode_printable_key(data.as_bytes()) {
		return Some(Cow::Owned(decoded));
	}
	if data.is_empty() || data.chars().any(char::is_control) {
		return None;
	}
	Some(Cow::Borrowed(data))
}

fn csi_u(data: &[u8]) -> Option<super::ParsedKittySequence> {
	if !data.starts_with(b"\x1b[") || !data.ends_with(b"u") || data.len() < 4 {
		return None;
	}
	// The text insertion API requires explicit modifier digits when a field is
	// present, matching the existing editor's CSI-u grammar. The lower-level
	// key parser separately accepts Kitty's empty-modifier form.
	if let Some(separator) = data.iter().position(|byte| *byte == b';')
		&& !data.get(separator + 1)?.is_ascii_digit()
	{
		return None;
	}
	parse_csi_u(data)
}

fn decode_kitty_printable(data: &[u8]) -> Option<String> {
	let parsed = csi_u(data)?;
	let modifier = parsed.modifier & !LOCK_MASK;
	if modifier & !MOD_SHIFT != 0
		|| super::optional_kitty_event_type(parsed.event_type) == Some(KeyEventType::Release)
	{
		return None;
	}
	if let Some(text) = data[2..data.len() - 1].split(|byte| *byte == b';').nth(2) {
		let mut decoded = String::new();
		for part in text
			.split(|byte| *byte == b':')
			.filter(|part| !part.is_empty())
		{
			let value = std::str::from_utf8(part).ok()?.parse::<u32>().ok()?;
			if value >= 32 && value != 127 {
				decoded.push(char::from_u32(value)?);
			}
		}
		if !decoded.is_empty() {
			return Some(decoded);
		}
	}
	let codepoint = keypad_operator_text_codepoint(parsed.codepoint)
		.or_else(|| {
			(modifier == 0)
				.then(|| keypad_num_lock_text_codepoint(parsed.codepoint))
				.flatten()
		})
		.unwrap_or_else(|| {
			if modifier & MOD_SHIFT != 0 {
				parsed.shifted_key.unwrap_or(parsed.codepoint)
			} else {
				parsed.codepoint
			}
		});
	if (0xe000..=0xf8ff).contains(&codepoint) {
		return None;
	}
	Some(printable_scalar(u32::try_from(codepoint).ok()?)?.to_string())
}

const fn printable_scalar(codepoint: u32) -> Option<char> {
	if codepoint < 32 || codepoint == 127 {
		return None;
	}
	char::from_u32(codepoint)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn text_fields_preserve_multiple_scalars_and_shifted_symbols() {
		assert_eq!(decode_printable_key(b"\x1b[0;1;945:128512u").as_deref(), Some("α😀"));
		assert_eq!(decode_printable_key(b"\x1b[49:33;2u").as_deref(), Some("!"));
		assert_eq!(decode_printable_key(b"\x1b[27;2;65~").as_deref(), Some("A"));
	}

	#[test]
	fn bindings_releases_and_malformed_scalars_never_insert_text() {
		for input in [
			b"\x1b[97;5u".as_slice(),
			b"\x1b[97;1:3u",
			b"\x1b[97;17u",
			b"\x1b[0;1;1114112u",
			b"\x1b[0;1;55296u",
			b"\x1b[0;;65u",
			b"\x1b[27;5;97~",
			b"\x1b[27;1;127~",
			b"\x1b[27;1;65",
		] {
			assert_eq!(decode_printable_key(input), None, "accepted {input:?}");
		}
	}

	#[test]
	fn keypad_text_overrides_navigation_only_when_it_produces_text() {
		assert_eq!(decode_kitty_keypad_text(b"\x1b[57400u").as_deref(), Some("1"));
		assert_eq!(decode_kitty_keypad_text(b"\x1b[57400;129u").as_deref(), Some("1"));
		assert_eq!(decode_kitty_keypad_text(b"\x1b[57400;5u"), None);
		assert_eq!(decode_kitty_keypad_text(b"\x1b[57413;2u").as_deref(), Some("+"));
		assert_eq!(decode_kitty_keypad_text(b"\x1b[97u"), None);
	}

	#[test]
	fn ordinary_unicode_is_insertable_but_protocol_bytes_and_c1_controls_are_not() {
		assert_eq!(extract_printable_text("你好😀").as_deref(), Some("你好😀"));
		assert_eq!(extract_printable_text("\x1b[31mred"), None);
		assert_eq!(extract_printable_text("a\u{009b}b"), None);
	}
}
