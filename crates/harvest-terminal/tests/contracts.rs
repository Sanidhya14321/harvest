use harvest_terminal::{
	fuzzy::{Matcher, PreparedText},
	output::{coalesce_sgr, conpty_chunks},
};
use serde::Deserialize;

#[derive(Deserialize)]
struct FuzzyCase {
	query:   String,
	text:    String,
	matches: bool,
	score:   f64,
}

#[test]
fn selector_scores_preserve_word_phrase_unicode_and_typo_contracts() {
	let fixtures: Vec<FuzzyCase> =
		serde_json::from_str(include_str!("fixtures/fuzzy.json")).expect("fuzzy fixture");
	let mut matcher = Matcher::default();
	for case in fixtures {
		let result = matcher.matches(&case.query, &case.text);
		assert_eq!(result.matches, case.matches, "query {:?}, text {:?}", case.query, case.text);
		assert!(
			(result.score - case.score).abs() < 1e-9,
			"query {:?}, text {:?}: {} != {}",
			case.query,
			case.text,
			result.score,
			case.score
		);
		assert_eq!(PreparedText::new(&case.text).matches(&case.query), result);
	}
}

#[test]
fn ranking_retains_ties_and_removes_nonmatches_after_cache_reset() {
	let texts = ["reimage image provider", "image provider", "not a match", "image provider"]
		.map(str::to_owned);
	let mut matcher = Matcher::default();
	let indices = |ranked: Vec<(usize, f64)>| {
		ranked
			.into_iter()
			.map(|(index, _)| index)
			.collect::<Vec<_>>()
	};
	assert_eq!(indices(matcher.rank("image provider", &texts)), [1, 3, 0]);
	matcher.clear();
	assert_eq!(indices(matcher.rank("image provider", &texts)), [1, 3, 0]);
	assert_eq!(indices(matcher.rank("!!!", &texts)), [0, 1, 2, 3]);
}

#[test]
fn sgr_merging_preserves_incomplete_color_boundaries_and_non_sgr_bytes() {
	let input: Vec<u16> = "\x1b[38;2;255;0m\x1b[31m\x1b[m\x1b[1mX\x1b[2K"
		.encode_utf16()
		.collect();
	let output = coalesce_sgr(&input).expect("adjacent run");
	assert_eq!(String::from_utf16(&output).expect("UTF-16"), "\x1b[38;2;255;0m\x1b[31;0;1mX\x1b[2K");
	let run = "\x1b[38;2;1;2;3m".repeat(10);
	let output = coalesce_sgr(&run.encode_utf16().collect::<Vec<_>>()).expect("color run");
	let output = String::from_utf16(&output).expect("UTF-16");
	for params in output.split("\x1b[").skip(1) {
		assert!(params.trim_end_matches('m').split([';', ':']).count() <= 16);
	}
}

#[test]
fn conpty_chunks_preserve_surrogates_and_prefer_newline_boundaries() {
	let input: Vec<u16> = "你好\n😀\nlast".encode_utf16().collect();
	let chunks = conpty_chunks(&input, 8.0);
	let strings: Vec<String> = chunks
		.iter()
		.map(|range| String::from_utf16(&input[range.clone()]).expect("whole scalars"))
		.collect();
	assert_eq!(strings, ["你好\n", "😀\n", "last"]);
	assert!(strings.iter().all(|chunk| chunk.len() <= 8));
	let torn = [0xd800, 10, 0xdc00, 0, 65];
	let chunks = conpty_chunks(&torn, 4.0);
	let rebuilt: Vec<u16> = chunks
		.into_iter()
		.flat_map(|range| torn[range].to_vec())
		.collect();
	assert_eq!(rebuilt, torn);
}

#[test]
fn utf8_chunk_offsets_match_utf16_writes_for_scalar_and_line_boundaries() {
	let text = "a\0你好\n😀\nlast";
	let units: Vec<u16> = text.encode_utf16().collect();
	for cap in [0.0, 1.0, 4.5, 8.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
		assert_eq!(
			harvest_terminal::output::conpty_utf8_chunks(text, cap),
			conpty_chunks(&units, cap)
		);
	}
}
