use harvest_terminal::{input, printable};

fn units(text: &str) -> Vec<u16> {
	text.encode_utf16().collect()
}
fn events(input: &[u16], scan: &input::Scan) -> Vec<Vec<u16>> {
	scan
		.sequences
		.iter()
		.map(|range| input[range.clone()].to_vec())
		.collect()
}
fn scanned(text: &str) -> Vec<Vec<u16>> {
	let input = units(text);
	events(&input, &input::scan(&input, 0))
}
fn strings(texts: &[&str]) -> Vec<Vec<u16>> {
	texts.iter().map(|text| units(text)).collect()
}

#[test]
fn fragmented_protocol_replies_never_leak_payload_as_typed_keys() {
	let wire =
		"x\x1b[<35;20;5m\x1b]52;c;payload\x1b\\\x1bPq\x1b\\\x1b_Gr=1\x1b\\\x1bOA\x1b[97;1:3uy";
	let expected = strings(&[
		"x",
		"\x1b[<35;20;5m",
		"\x1b]52;c;payload\x1b\\",
		"\x1bPq\x1b\\",
		"\x1b_Gr=1\x1b\\",
		"\x1bOA",
		"\x1b[97;1:3u",
		"y",
	]);
	let wire = units(wire);
	for split in 1..wire.len() {
		let head = input::scan(&wire[..split], 0);
		let mut actual = events(&wire[..split], &head);
		let mut remainder = wire[head.remainder_start..split].to_vec();
		remainder.extend_from_slice(&wire[split..]);
		let tail = input::scan(&remainder, head.resume_search_from);
		actual.extend(events(&remainder, &tail));
		assert_eq!(actual, expected, "split {split}");
		assert_eq!(tail.remainder_start, remainder.len(), "split {split}");
	}
}

#[test]
fn meta_keys_mouse_prefixes_and_bare_double_escape_remain_distinct() {
	assert_eq!(scanned("\x1b\x1b[A"), strings(&["\x1b\x1b[A"]));
	assert_eq!(scanned("\x1b\x1b[<0;1;2M"), strings(&["\x1b", "\x1b[<0;1;2M"]));
	assert_eq!(scanned("\x1b\x1bd"), strings(&["\x1b", "\x1bd"]));
	assert_eq!(input::scan(&units("\x1b\x1b"), 0).remainder_start, 0);
	assert_eq!(input::scan(&units("\x1b[M a"), 0).remainder_start, 0);
	assert_eq!(scanned("\x1b[M ab"), strings(&["\x1b[M ab"]));
}

#[test]
fn scanner_preserves_surrogate_pairs_and_malformed_units_without_replacement() {
	assert_eq!(
		events(&[0xd83d, 0xde00, 0xdc00, 0x61], &input::scan(&[0xd83d, 0xde00, 0xdc00, 0x61], 0)),
		vec![vec![0xd83d, 0xde00], vec![0xdc00], vec![0x61]]
	);
}

#[test]
fn csi_caps_preserve_raw_bytes_but_torn_string_caps_request_discard() {
	let mut csi = units("\x1b[");
	csi.resize(input::MAX_CSI_UNITS + 4, 0x31);
	let scanned = input::scan(&csi, 0);
	assert_eq!(scanned.remainder_start, csi.len());
	assert_eq!(events(&csi, &scanned).concat(), csi);
	assert_eq!(scanned.sequences[0].len(), input::MAX_CSI_UNITS);
	let mut string = units("x\x1b]");
	string.resize(input::MAX_STRING_UNITS + 10, 0x31);
	let scanned = input::scan(&string, 0);
	assert_eq!(events(&string, &scanned), strings(&["x"]));
	assert_eq!(scanned.discard_from, Some(1));
	assert_eq!(scanned.remainder_start, string.len());
}

#[test]
fn resumed_string_scanning_finds_split_terminators_and_preserves_bel_rules() {
	for prefix in ["\x1b]52;c;", "\x1bPq", "\x1b_Gr=1;"] {
		let mut head = units(prefix);
		head.extend(std::iter::repeat_n(0x31, 16_384));
		head.push(0x1b);
		let partial = input::scan(&head, 0);
		assert_eq!(partial.resume_search_from, head.len());
		head.extend_from_slice(&[0x5c, 0x61]);
		let result = input::scan(&head, partial.resume_search_from);
		assert_eq!(events(&head, &result), vec![head[..head.len() - 1].to_vec(), units("a")]);
	}
	assert_eq!(scanned("\x1b]x\x07z"), strings(&["\x1b]x\x07", "z"]));
	assert_eq!(input::scan(&units("\x1bPx\x07"), 0).remainder_start, 0);
}

#[test]
fn text_fields_shifted_keys_and_keypad_inputs_insert_their_visible_text() {
	assert_eq!(printable::decode_key("\x1b[97:65;2u"), Some(units("A")));
	assert_eq!(printable::decode_key("\x1b[0;1;128512:769u"), Some(units("😀\u{301}")));
	assert_eq!(printable::decode_key("\x1b[0;1;0:127:97u"), Some(units("a")));
	assert_eq!(printable::decode_key("\x1b[57400;129u"), Some(units("1")));
	assert_eq!(printable::decode_key("\x1b[57413;2u"), Some(units("+")));
	assert_eq!(printable::decode_key("\x1b[27;2;128512~"), Some(units("😀")));
	assert_eq!(printable::keypad_text("\x1b[57404u"), Some(units("5")));
	assert_eq!(printable::keypad_text("\x1b[32u"), None);
}

#[test]
fn shortcuts_releases_and_invalid_codepoints_do_not_insert_text() {
	for sequence in [
		"\x1b[97;5u",
		"\x1b[97;1:3u",
		"\x1b[99;17u",
		"\x1b[27;3;97~",
		"\x1b[57344u",
		"\x1b[1114112u",
		"\x1b[0;1;1114112u",
		"\x1b[١u",
	] {
		assert_eq!(printable::decode_key(sequence), None, "{sequence:?}");
	}
	assert_eq!(printable::extract(&units("a\x08")), None);
	assert_eq!(printable::extract(&[0x61, 0x85]), None);
	assert_eq!(printable::extract(&[0xd800]), Some(vec![0xd800]));
	assert_eq!(printable::decode_key("\x1b[55296u"), Some(vec![0xd800]));
}
