use std::path::Path;

use super::*;

const TWO_WAY: &str =
	"fn a() {}\n<<<<<<< HEAD\nlet x = 1;\n=======\nlet x = 2;\n>>>>>>> feature\nfn b() {}\n";
const DIFF3: &str = "<<<<<<< ours\none\n||||||| base\nzero\n=======\ntwo\n>>>>>>> theirs\n";

fn register(text: &str) -> (ConflictHistory, Vec<Entry>) {
	let history = ConflictHistory::new();
	let lines: Vec<&str> = text.split('\n').collect();
	let entries = scan_lines(&lines, 1)
		.into_iter()
		.map(|block| history.register(Path::new("/w/f.rs"), "f.rs", block))
		.collect();
	(history, entries)
}

#[test]
fn markers_are_strict_and_only_closed_blocks_count() {
	let lines =
		["<<<<<<<x", "<<<<<<< ok", "a", "=======", "b", ">>>>>>>", "<<<<<<< open", "c", "======="];
	let blocks = scan_lines(&lines, 10);
	assert_eq!(blocks.len(), 1);
	let block = &blocks[0];
	assert_eq!((block.start_line, block.separator_line, block.end_line), (11, 13, 15));
	assert_eq!((block.ours_label.as_deref(), block.theirs_label.as_deref()), (Some("ok"), None));
	assert_eq!(
		(block.ours.as_slice(), block.theirs.as_slice()),
		(&["a".to_owned()][..], &["b".to_owned()][..])
	);
	let diff3 = scan_lines(&DIFF3.split('\n').collect::<Vec<_>>(), 1);
	assert_eq!(diff3[0].base, Some(vec!["zero".to_owned()]));
	assert_eq!(diff3[0].base_line, Some(3));
}

#[test]
fn splice_replaces_only_the_marker_block_and_trims_echoed_context() {
	let (_, entries) = register(TWO_WAY);
	let (text, leading, trailing) = splice(TWO_WAY, &entries[0], "let x = 3;").unwrap();
	assert_eq!(text, "fn a() {}\nlet x = 3;\nfn b() {}\n");
	assert_eq!((leading, trailing), (0, 0));
	// A two-line trailing echo is always trimmed.
	let text = "<<<<<<<\na\n=======\nb\n>>>>>>>\nkeep1\nkeep2\n";
	let (_, entries) = register(text);
	let (spliced, _, trailing) = splice(text, &entries[0], "a\nkeep1\nkeep2").unwrap();
	assert_eq!((spliced.as_str(), trailing), ("a\nkeep1\nkeep2\n", 2));
}

#[test]
fn crlf_regions_keep_crlf_after_a_splice() {
	let text = "x\r\n<<<<<<< a\r\none\r\n=======\r\ntwo\r\n>>>>>>> b\r\ny\r\n";
	let (_, entries) = register(text);
	let (spliced, ..) =
		splice(text, &entries[0], &expand_tokens("@theirs", &entries[0]).unwrap()).unwrap();
	assert_eq!(spliced, "x\r\ntwo\r\ny\r\n");
}

#[test]
fn tokens_expand_whole_lines_and_base_needs_diff3() {
	let (_, entries) = register(TWO_WAY);
	assert_eq!(
		expand_tokens("// both\n@both", &entries[0]).unwrap(),
		"// both\nlet x = 1;\nlet x = 2;"
	);
	assert_eq!(expand_tokens("x @ours", &entries[0]).unwrap(), "x @ours");
	assert!(
		expand_tokens("@base", &entries[0])
			.unwrap_err()
			.to_string()
			.contains("no base section")
	);
}

#[test]
fn uris_parse_with_scopes_wildcards_and_recovered_prefixes() {
	assert_eq!(parse_uri("src/a.rs").unwrap(), None);
	assert_eq!(
		parse_uri("conflict://3/theirs").unwrap(),
		Some(Uri {
			id:               Some(3),
			scope:            Some(Scope::Theirs),
			recovered_prefix: None,
		})
	);
	assert_eq!(
		parse_uri("conflict://*").unwrap(),
		Some(Uri { id: None, scope: None, recovered_prefix: None })
	);
	assert_eq!(
		parse_uri("a.ts:conflict://2")
			.unwrap()
			.unwrap()
			.recovered_prefix
			.as_deref(),
		Some("a.ts")
	);
	assert!(
		parse_uri("conflict://*/ours")
			.unwrap_err()
			.to_string()
			.contains("does not accept a scope")
	);
	assert!(
		parse_uri("conflict://0")
			.unwrap_err()
			.to_string()
			.contains("id must be ≥ 1")
	);
	assert!(
		parse_uri("conflict://x")
			.unwrap_err()
			.to_string()
			.contains("positive integer")
	);
	assert!(
		parse_uri("conflict://1/mine")
			.unwrap_err()
			.to_string()
			.contains("scope must be one of")
	);
}

#[test]
fn reregistering_a_block_reuses_its_id() {
	let (history, first) = register(TWO_WAY);
	let lines: Vec<&str> = TWO_WAY.split('\n').collect();
	let again = history.register(Path::new("/w/f.rs"), "f.rs", scan_lines(&lines, 1).remove(0));
	assert_eq!(again.id, first[0].id);
	assert_eq!(history.entries().len(), 1);
}

#[test]
fn directives_parse_and_reject_partial_blocks() {
	let map = parse_directives("1: @ours\n#2 = @theirs\n")
		.unwrap()
		.unwrap();
	assert_eq!(map.get(&2).map(String::as_str), Some("@theirs"));
	assert_eq!(parse_directives("@ours").unwrap(), None);
	assert!(
		parse_directives("1: @ours\n1: @theirs")
			.unwrap_err()
			.to_string()
			.contains("twice")
	);
	let error = parse_directives("1: @ours\n2: literal")
		.unwrap_err()
		.to_string();
	assert!(error.contains("Per-id bulk only accepts the tokens"), "{error}");
}
