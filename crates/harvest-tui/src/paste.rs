//! Normalize tmux's encoded Ctrl+letter paste bytes before editor sanitization.
use std::borrow::Cow;

/// Preserve arbitrary UTF-8 and non-letter control encodings. Only the two
/// documented tmux Ctrl+letter forms are converted to ASCII control bytes.
pub fn decode_reencoded_controls(text: &str) -> Cow<'_, str> {
	let bytes = text.as_bytes();
	let mut output = None::<String>;
	let mut copied = 0;
	let mut position = 0;
	while position < bytes.len() {
		if bytes[position] == 0x1b
			&& let Some((length, control)) = decode_control(&bytes[position..])
		{
			let result = output.get_or_insert_with(|| String::with_capacity(text.len()));
			result.push_str(&text[copied..position]);
			result.push(char::from(control));
			position += length;
			copied = position;
		} else {
			position += 1;
		}
	}
	match output {
		Some(mut result) => {
			result.push_str(&text[copied..]);
			Cow::Owned(result)
		},
		None => Cow::Borrowed(text),
	}
}

fn decode_control(bytes: &[u8]) -> Option<(usize, u8)> {
	let tail = bytes.strip_prefix(b"\x1b[")?;
	let (digits, suffix, prefix) = match tail.strip_prefix(b"27;5;") {
		Some(digits) => (digits, b"~".as_slice(), 7),
		None => (tail, b";5u".as_slice(), 2),
	};
	let mut value = 0_u32;
	let mut length = 0;
	for &digit in digits {
		if !digit.is_ascii_digit() {
			break;
		}
		value = value
			.checked_mul(10)?
			.checked_add(u32::from(digit - b'0'))?;
		length += 1;
	}
	if length == 0 || !digits[length..].starts_with(suffix) {
		return None;
	}
	let letter = u8::try_from(value).ok()?;
	let control = match letter {
		b'a'..=b'z' => letter - b'a' + 1,
		b'A'..=b'Z' => letter - b'A' + 1,
		_ => return None,
	};
	Some((prefix + length + suffix.len(), control))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tmux_paste_newlines_tabs_and_carriage_returns_survive_control_stripping() {
		let decoded = decode_reencoded_controls("α\x1b[106;5uβ\x1b[27;5;73~γ\x1b[77;5uδ");
		assert_eq!(decoded, "α\nβ\tγ\rδ");
	}

	#[test]
	fn unknown_or_nonletter_encodings_cannot_synthesize_terminal_controls() {
		let text = "\x1b[0;5u\x1b[27;5;91~\x1b[106;3u\x1b[999999999999999999999;5u\x1b[106;5";
		assert_eq!(decode_reencoded_controls(text), text);
	}

	#[test]
	fn a_malformed_sequence_does_not_hide_a_later_encoded_line_break() {
		assert_eq!(decode_reencoded_controls("\x1b[12;\x1b[27;5;106~tail"), "\x1b[12;\ntail");
	}
}
