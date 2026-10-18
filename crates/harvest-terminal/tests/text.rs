use std::borrow::Cow;

use harvest_terminal::{
	frame::TextLayout,
	layout::PlainTextLayout,
	text::{self, Ellipsis},
};
fn utf16(value: &str) -> Vec<u16> {
	value.encode_utf16().collect()
}

#[test]
fn truncation_preserves_styles_and_graphemes_while_borrowing_unchanged_inputs() {
	let text = utf16("\x1b[31m界🙂x\x1b[0m");
	let fitted = text::truncate_to_width(&text, 5, None, None, 3);
	assert!(
		matches!(fitted, Cow::Borrowed(_)),
		"the N-API adapter must preserve the original string handle when unchanged"
	);
	let cut = text::truncate_to_width(&text, 4, Some(Ellipsis::Unicode), None, 3);
	assert_eq!(String::from_utf16_lossy(&cut), "\x1b[31m界\x1b[0m…");
	assert_eq!(text::visible_width(&cut, 3), 3);
	let padded =
		text::truncate_to_width(&utf16("界"), 4, Some(Ellipsis::Omit), Some(true), 3).into_owned();
	assert_eq!(String::from_utf16_lossy(&padded), "界  ");
}

#[test]
fn isolated_utf16_and_zero_length_overlay_boundaries_remain_lossless() {
	let input = [0xd800, b'x' as u16];
	assert!(matches!(text::truncate_to_width(&input, 8, None, None, 3), Cow::Borrowed(_)));
	assert_eq!(text::slice_with_width(&input, 0, 0, None, 3), (Vec::new(), 0));
	let (left, left_width, right, right_width) =
		text::extract_segments(&utf16("\x1b[31ma界b\x1b[0m"), 1, 3, 1, true, 3);
	assert_eq!(String::from_utf16_lossy(&left), "\x1b[31ma");
	assert_eq!(left_width, 1);
	assert_eq!(String::from_utf16_lossy(&right), "\x1b[31mb");
	assert_eq!(right_width, 1);
}

#[test]
fn untrusted_terminal_text_cannot_move_the_cursor_or_split_a_frame_row() {
	let mut layout = PlainTextLayout::new(3);
	let rows = layout.wrap_text("界🙂\tline\n\x1b[3Jdanger\rnext\u{009b}2J", 8);
	let body = rows.join("\n");
	assert!(body.contains("␛[3J"));
	assert!(
		rows
			.iter()
			.all(|row| !row.chars().any(char::is_control) && text::visible_width(&utf16(row), 3) <= 8)
	);
	assert_eq!(layout.fit_row("a\r\nb\t\x1b", 20), "a  b   ␛");
}
