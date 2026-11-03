//! Fact extraction (`core/extraction.ts`): LLM prompt + parser + heuristics.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::providers::{Llm, MAX_LLM_RESPONSE_CHARS};

const EXTRACTION_PROMPT: &str = include_str!("prompts/extraction.md");
const FLAT_FACT_LIMIT: usize = 5;
const CATEGORY_LIMIT: usize = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KgTriple {
	pub subject:   String,
	pub predicate: String,
	pub object:    String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedFactCategories {
	pub facts:        Vec<String>,
	pub instructions: Vec<String>,
	pub preferences:  Vec<String>,
	pub timelines:    Vec<String>,
	pub kg:           Vec<KgTriple>,
}

impl ExtractedFactCategories {
	pub fn count(&self) -> usize {
		self.facts.len()
			+ self.instructions.len()
			+ self.preferences.len()
			+ self.timelines.len()
			+ self.kg.len()
	}
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
	cell.get_or_init(|| Regex::new(pat).expect("valid regex"))
}

/// Build the extraction prompt. `MNEMOPI_EXTRACTION_PROMPT` overrides the
/// bundled template.
pub fn build_extraction_prompt(text: &str, lang: &str) -> String {
	let template = std::env::var("MNEMOPI_EXTRACTION_PROMPT")
		.ok()
		.filter(|v| !v.is_empty())
		.unwrap_or_else(|| EXTRACTION_PROMPT.trim_end().to_string());
	template.replace("{text}", text).replace("{lang}", lang)
}

fn normalize_fact(fact: &str) -> String {
	fact.trim().trim_end_matches(['.', '!', '?']).to_string()
}

fn strip_fence(raw: &str) -> String {
	let s = raw.trim();
	if !s.starts_with("```") {
		return s.to_string();
	}
	static OPEN: OnceLock<Regex> = OnceLock::new();
	static CLOSE: OnceLock<Regex> = OnceLock::new();
	let s = re(&OPEN, r"(?i)^```(?:json)?\s*").replace(s, "");
	re(&CLOSE, r"\s*```$").replace(&s, "").trim().to_string()
}

fn normalize_array(items: Option<&Value>, fields: &[&str], join: bool) -> Vec<String> {
	let Some(Value::Array(items)) = items else {
		return Vec::new();
	};
	let mut out = Vec::new();
	for item in items {
		let text = match item {
			Value::String(s) => Some(s.trim().to_string()),
			Value::Object(map) => {
				let mut parts = Vec::new();
				for key in fields {
					if let Some(Value::String(c)) = map.get(*key) {
						let t = c.trim();
						if !t.is_empty() {
							parts.push(t.to_string());
							if !join {
								break;
							}
						}
					}
				}
				(!parts.is_empty()).then(|| parts.join(" "))
			},
			_ => None,
		};
		if let Some(t) = text.filter(|t| !t.is_empty()) {
			let n = normalize_fact(&t);
			if !n.is_empty() {
				out.push(n);
				if out.len() >= CATEGORY_LIMIT {
					break;
				}
			}
		}
	}
	out
}

fn normalize_kg(items: Option<&Value>) -> Vec<KgTriple> {
	let Some(Value::Array(items)) = items else {
		return Vec::new();
	};
	let part = |v: Option<&Value>| match v {
		Some(Value::String(s)) => normalize_fact(s),
		_ => String::new(),
	};
	let mut out = Vec::new();
	for item in items {
		let (s, p, o) = match item {
			Value::Object(m) => {
				(part(m.get("subject")), part(m.get("predicate")), part(m.get("object")))
			},
			Value::Array(a) => (part(a.first()), part(a.get(1)), part(a.get(2))),
			_ => continue,
		};
		if !s.is_empty() && !p.is_empty() && !o.is_empty() {
			out.push(KgTriple { subject: s, predicate: p, object: o });
			if out.len() >= CATEGORY_LIMIT {
				break;
			}
		}
	}
	out
}

const FACT_FIELDS: &[&str] = &["fact", "text", "content", "value", "statement"];
const INSTR_FIELDS: &[&str] =
	&["instruction", "rule", "fact", "text", "content", "value", "statement"];
const PREF_FIELDS: &[&str] = &["preference", "fact", "text", "content", "value", "statement"];
const TIMELINE_FIELDS: &[&str] =
	&["description", "event", "timeline", "date", "fact", "text", "content", "value", "statement"];

/// Port of `parseExtractedFactCategories`.
pub fn parse_extracted(raw_output: &str) -> ExtractedFactCategories {
	let raw: String = raw_output.chars().take(MAX_LLM_RESPONSE_CHARS).collect();
	let raw = raw.trim();
	if raw.is_empty() || raw.eq_ignore_ascii_case("NO_FACTS") {
		return ExtractedFactCategories::default();
	}
	let clean = strip_fence(raw);
	if clean.starts_with('{') {
		match serde_json::from_str::<Value>(&clean) {
			Ok(Value::Object(p)) => {
				return ExtractedFactCategories {
					facts:        normalize_array(p.get("facts"), FACT_FIELDS, false),
					instructions: normalize_array(p.get("instructions"), INSTR_FIELDS, false),
					preferences:  normalize_array(p.get("preferences"), PREF_FIELDS, false),
					timelines:    normalize_array(p.get("timelines"), TIMELINE_FIELDS, true),
					kg:           normalize_kg(p.get("kg")),
				};
			},
			Ok(_) => {},
			Err(_) => {
				static QUOTED: OnceLock<Regex> = OnceLock::new();
				let facts: Vec<String> = re(&QUOTED, r#""([^"]{10,})""#)
					.captures_iter(raw)
					.filter_map(|c| c.get(1).map(|m| normalize_fact(m.as_str())))
					.filter(|f| !f.is_empty())
					.take(FLAT_FACT_LIMIT)
					.collect();
				if !facts.is_empty() {
					return ExtractedFactCategories { facts, ..Default::default() };
				}
			},
		}
	}
	static BULLET: OnceLock<Regex> = OnceLock::new();
	let bullet = re(&BULLET, r"^[\s\d.\-*]+");
	let mut cleaned = Vec::new();
	for line in raw.lines() {
		let fact = bullet.replace(line, "").trim().to_string();
		if fact.chars().count() > 10 {
			let n = normalize_fact(&fact);
			if !n.is_empty() {
				cleaned.push(n);
			}
		}
		if cleaned.len() >= FLAT_FACT_LIMIT {
			break;
		}
	}
	ExtractedFactCategories { facts: cleaned, ..Default::default() }
}

fn sentence_case(v: &str) -> String {
	let t = v.trim().trim_end_matches(['.', '!', '?']);
	let mut c = t.chars();
	match c.next() {
		Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
		None => String::new(),
	}
}

fn add_unique(out: &mut Vec<String>, value: &str) {
	let fact = sentence_case(value);
	if fact.chars().count() > 10 && !out.contains(&fact) {
		out.push(fact);
	}
}

/// Port of `heuristicExtractFacts`.
pub fn heuristic_extract_facts(text: &str) -> Vec<String> {
	static WS: OnceLock<Regex> = OnceLock::new();
	static SPLIT: OnceLock<Regex> = OnceLock::new();
	static PATS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
	static INSTR: OnceLock<Regex> = OnceLock::new();
	let normalized = re(&WS, r"\s+").replace_all(text, " ").trim().to_string();
	if normalized.is_empty() {
		return Vec::new();
	}
	let pats = PATS.get_or_init(|| {
		[
			(r"(?i)\bmy name is\s+([^,.!?;]+)", "The user's name is "),
			(r"(?i)\bi (?:am|work as)\s+(?:an?\s+)?([^,.!?;]+)", "The user is "),
			(r"(?i)\bi work (?:at|for)\s+([^,.!?;]+)", "The user works at "),
			(r"(?i)\bi (?:live in|am based in)\s+([^,.!?;]+)", "The user lives in "),
			(r"(?i)\bi (?:use|uses|am using)\s+([^,.!?;]+)", "The user uses "),
			(r"(?i)\bi (?:like|love|prefer|enjoy)\s+([^,.!?;]+)", "The user prefers "),
			(r"(?i)\bi (?:hate|dislike|do not like|don't like)\s+([^,.!?;]+)", "The user dislikes "),
		]
		.into_iter()
		.map(|(p, l)| (Regex::new(p).expect("regex"), l))
		.collect()
	});
	let instr = re(&INSTR, r"(?i)\b(?:i|you)\s+(always|never)\s+([^,.!?;]+)");
	let mut facts = Vec::new();
	for clause in re(&SPLIT, r"(?i)(?:[.!?;]+|\s+and\s+|\s+but\s+)").split(&normalized) {
		let c = clause.trim();
		for (pat, label) in pats {
			if let Some(m) = pat.captures(c).and_then(|cap| cap.get(1)) {
				add_unique(&mut facts, &format!("{label}{}", m.as_str()));
			}
		}
		if let Some(cap) = instr.captures(c) {
			if let (Some(a), Some(b)) = (cap.get(1), cap.get(2)) {
				add_unique(
					&mut facts,
					&format!("Instruction: {} {}", a.as_str().to_lowercase(), b.as_str()),
				);
			}
		}
	}
	facts.truncate(5);
	facts
}

/// LLM extraction with heuristic fallback (`extractFactCategoriesSafe`).
pub fn extract_fact_categories(
	llm: Option<&dyn Llm>,
	text: &str,
	max_tokens: u32,
) -> ExtractedFactCategories {
	if text.trim().is_empty() {
		return ExtractedFactCategories::default();
	}
	if let Some(llm) = llm {
		if let Ok(raw) = llm.complete(&build_extraction_prompt(text, "en"), max_tokens) {
			let parsed = parse_extracted(&raw);
			if parsed.count() > 0 {
				return parsed;
			}
		}
	}
	ExtractedFactCategories { facts: heuristic_extract_facts(text), ..Default::default() }
}
