//! Text-producing Kitty and xterm keys, distinct from shortcut normalization.
use std::sync::LazyLock;

use regex::{Captures, Regex};

static KITTY: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
		r"^\x1b\[([0-9]+)(?::([0-9]*))?(?::([0-9]+))?(?:;([0-9]+))?(?::([0-9]+))?(?:;([0-9:]*))?u$",
	)
	.expect("valid Kitty text grammar")
});
static MODIFY_OTHER: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"^\x1b\[27;([0-9]+);([0-9]+)~$").expect("valid xterm text grammar")
});
const LOCK_MASK: u32 = 64 + 128;

fn number(captures: &Captures<'_>, index: usize) -> Option<f64> {
	captures
		.get(index)
		.and_then(|value| value.as_str().parse().ok())
}

// The legacy host applies ECMAScript's 32-bit conversion after subtracting one.
// This also preserves malformed oversized-modifier handling at the boundary.
fn modifiers(value: f64) -> u32 {
	if value.is_finite() {
		(value - 1.0).rem_euclid(4_294_967_296.0) as u32
	} else {
		0
	}
}

fn append_codepoint(text: &mut Vec<u16>, codepoint: f64) -> Option<()> {
	if !codepoint.is_finite() || !(32.0..=1_114_111.0).contains(&codepoint) || codepoint == 127.0 {
		return None;
	}
	let codepoint = codepoint as u32;
	if codepoint <= 0xffff {
		text.push(codepoint as u16);
	} else {
		let astral = codepoint - 0x1_0000;
		text.push(0xd800 + (astral >> 10) as u16);
		text.push(0xdc00 + (astral & 0x3ff) as u16);
	}
	Some(())
}

const fn keypad(codepoint: f64, digits: bool) -> Option<&'static str> {
	match codepoint as u32 {
		57410 => Some("/"),
		57411 => Some("*"),
		57412 => Some("-"),
		57413 => Some("+"),
		57415 => Some("="),
		57399 if digits => Some("0"),
		57400 if digits => Some("1"),
		57401 if digits => Some("2"),
		57402 if digits => Some("3"),
		57403 if digits => Some("4"),
		57404 if digits => Some("5"),
		57405 if digits => Some("6"),
		57406 if digits => Some("7"),
		57407 if digits => Some("8"),
		57408 if digits => Some("9"),
		57409 if digits => Some("."),
		_ => None,
	}
}

fn kitty_text(captures: &Captures<'_>) -> Option<Vec<u16>> {
	let codepoint = number(captures, 1)?;
	if !codepoint.is_finite() || captures.get(5).is_some_and(|event| event.as_str() == "3") {
		return None;
	}
	let modifier = modifiers(number(captures, 4).unwrap_or(1.0)) & !LOCK_MASK;
	if modifier & !1 != 0 {
		return None;
	}
	if let Some(field) = captures.get(6) {
		let mut text = Vec::new();
		for value in field.as_str().split(':').filter(|value| !value.is_empty()) {
			let value: f64 = value.parse().ok()?;
			if value.is_finite() && value >= 32.0 && value != 127.0 {
				append_codepoint(&mut text, value)?;
			}
		}
		if !text.is_empty() {
			return Some(text);
		}
	}
	if let Some(text) = keypad(codepoint, modifier == 0) {
		return Some(text.encode_utf16().collect());
	}
	let effective = if modifier & 1 == 1 {
		number(captures, 2).unwrap_or(codepoint)
	} else {
		codepoint
	};
	if (57_344.0..=63_743.0).contains(&effective) {
		return None;
	}
	let mut text = Vec::new();
	append_codepoint(&mut text, effective)?;
	Some(text)
}

/// Decode text-bearing escape input; shortcuts and releases never insert text.
#[must_use]
pub fn decode_key(data: &str) -> Option<Vec<u16>> {
	if let Some(captures) = KITTY.captures(data) {
		return kitty_text(&captures);
	}
	let captures = MODIFY_OTHER.captures(data)?;
	let modifier = number(&captures, 1)?;
	if !modifier.is_finite() || modifiers(modifier) & !LOCK_MASK & !1 != 0 {
		return None;
	}
	let mut text = Vec::new();
	append_codepoint(&mut text, number(&captures, 2)?)?;
	Some(text)
}

/// Keypad digits/operators override shortcut navigation only for text events.
#[must_use]
pub fn keypad_text(data: &str) -> Option<Vec<u16>> {
	let captures = KITTY.captures(data)?;
	keypad(number(&captures, 1)?, true)?;
	kitty_text(&captures)
}

/// Preserve raw text, including unmatched UTF-16 units, but reject control
/// input.
#[must_use]
pub fn extract(data: &[u16]) -> Option<Vec<u16>> {
	if data.first() == Some(&0x1b)
		&& let Ok(sequence) = String::from_utf16(data)
		&& let Some(text) = decode_key(&sequence)
	{
		return Some(text);
	}
	if data.is_empty()
		|| data
			.iter()
			.any(|unit| *unit < 32 || (0x7f..=0x9f).contains(unit))
	{
		return None;
	}
	Some(data.to_vec())
}
