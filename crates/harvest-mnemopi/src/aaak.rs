//! AAAK shorthand encoder (`core/aaak.ts`).

use std::sync::OnceLock;

use regex::Regex;

const CATEGORY_MAP: &[(&str, &str)] = &[
	("PREFERENCE", "PREF"),
	("TRAIT", "TRAIT"),
	("STATUS", "STAT"),
	("INSTRUCTION", "INST"),
	("PROJECT", "PROJ"),
	("LOCATION", "LOC"),
	("FAMILY", "FAM"),
	("OCCUPATION", "OCC"),
	("DECISION", "DEC"),
	("EVENT", "EVT"),
	("TOOL", "TOOL"),
	("FACT", "FACT"),
	("OPINION", "OPN"),
];

const PHRASE_MAP: &[(&str, &str)] = &[
	("User asked ", "ASK "),
	("User wants ", "WANT "),
	("User prefers ", "PREF "),
	("User likes ", "LIKE "),
	("User dislikes ", "DISLIKE "),
	("User is ", "IS "),
	("User has ", "HAS "),
	("User built ", "BUILT "),
	("User asked for ", "ASK "),
	("User requested ", "REQ "),
	("Married to ", "MARRIED→"),
	("Email: ", "@"),
	("GitHub: ", "GH:"),
	("Location: ", "LOC:"),
	("Phone: ", "PH:"),
	("User email is ", "@"),
	("User voice message ", "VM "),
	("User stack: ", "STACK|"),
	("Full-stack developer", "FSDEV"),
	("Software Developer", "SDEV"),
	("AI Systems Engineer", "AIENG"),
	("real-time", "RT"),
	("Real-time", "RT"),
	("bilingual", "bi"),
	("Bilingual", "bi"),
	("self-hosted", "selfhost"),
	("automation", "auto"),
	("transcription", "transc"),
	("translation", "transl"),
];

const STRUCTURAL: &[(&str, &str)] = &[
	(" - ", " | "),
	(" -- ", " | "),
	(" | ", " | "),
	(", ", " | "),
	(" and ", "+"),
	(" or ", "/"),
	(" for ", "→"),
	(" to ", "→"),
	(" with ", " w/ "),
	(" over ", ">"),
	(" instead of ", "!>"),
	(" because of ", "∵"),
	(" due to ", "∵"),
	(" using ", "→"),
	(" built ", "→"),
	(" in ", ":"),
	(" at ", "@"),
	(" on ", "@"),
	(" from ", "<-"),
];

fn sorted_phrases() -> &'static Vec<(&'static str, &'static str)> {
	static S: OnceLock<Vec<(&str, &str)>> = OnceLock::new();
	S.get_or_init(|| {
		let mut v = PHRASE_MAP.to_vec();
		// Stable sort by descending key length, matching Array#sort semantics.
		v.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
		v
	})
}

pub fn encode(text: &str) -> String {
	if text.is_empty() {
		return String::new();
	}
	if text.contains('|') && text.split_whitespace().count() <= 3 {
		return text.to_string();
	}
	let mut result = text.trim().to_string();
	for (full, short) in CATEGORY_MAP {
		let prefix = format!("{full}: ");
		if result.starts_with(&prefix) {
			result = result.replacen(&prefix, &format!("{short}|"), 1);
			break;
		}
	}
	for (phrase, short) in sorted_phrases() {
		result = result.replace(phrase, short);
	}
	for (pattern, rep) in STRUCTURAL {
		result = result.replace(pattern, rep);
	}
	static PAREN: OnceLock<Regex> = OnceLock::new();
	let paren = PAREN.get_or_init(|| Regex::new(r"\(\s*").expect("regex"));
	result = paren.replace_all(&result, "(").replace(" )", ")");
	result = result.replace("working correctly", "OK");
	result = result.replace("working", "OK");
	result = result.replace("complete", "DONE");
	result = result.replace("completed", "DONE");
	result.trim().to_string()
}
