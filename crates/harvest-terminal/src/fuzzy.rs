//! Word-local selector ranking. Offsets and spans use UTF-16 units to retain
//! the scoring contract exposed by the JavaScript SDK.
#![allow(
	clippy::suboptimal_flops,
	reason = "preserve JavaScript scoring and stable tie rounding; fused operations change rankings"
)]
use std::{collections::HashMap, sync::LazyLock};

use regex::Regex;

const CACHE_ENTRIES: usize = 4096;
const CACHE_TEXT_UNITS: usize = 4096;
const SHADOW_RESCAN_UNITS: usize = 3;

static ACRONYM_BOUNDARY: LazyLock<Regex> =
	LazyLock::new(|| Regex::new("([A-Z]+)([A-Z][a-z])").expect("valid acronym regex"));
static CAMEL_BOUNDARY: LazyLock<Regex> =
	LazyLock::new(|| Regex::new("([a-z0-9])([A-Z])").expect("valid camel-case regex"));
static SEPARATOR: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"[^\p{Letter}\p{Mark}\p{Number}]+").expect("valid search separator regex")
});

fn normalize(text: &str) -> Vec<u16> {
	let acronym = ACRONYM_BOUNDARY.replace_all(text, "$1 $2");
	let camel = CAMEL_BOUNDARY.replace_all(&acronym, "$1 $2");
	let lower = camel.to_lowercase();
	SEPARATOR
		.replace_all(&lower, " ")
		.trim()
		.encode_utf16()
		.collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
	pub matches: bool,
	pub score:   f64,
}

impl Match {
	const EMPTY: Self = Self { matches: true, score: 0.0 };
	const NONE: Self = Self { matches: false, score: 0.0 };

	const fn scored(score: f64) -> Self {
		Self { matches: true, score }
	}
}

struct Word {
	text:    Vec<u16>,
	index:   usize,
	ordinal: usize,
}

struct SearchIndex {
	normalized: Vec<u16>,
	compact:    Vec<u16>,
	starts:     Vec<usize>,
	words:      Vec<Word>,
}

impl SearchIndex {
	fn new(text: &str) -> Self {
		let normalized = normalize(text);
		let mut compact = Vec::with_capacity(normalized.len());
		let mut starts = Vec::new();
		let mut words = Vec::new();
		let mut index = 0;
		if !normalized.is_empty() {
			for (ordinal, text) in normalized.split(|unit| *unit == 32).enumerate() {
				starts.push(compact.len());
				compact.extend_from_slice(text);
				words.push(Word { text: text.to_vec(), index, ordinal });
				index += text.len() + 1;
			}
		}
		Self { normalized, compact, starts, words }
	}
}

struct Query {
	normalized: Vec<u16>,
	compact:    Vec<u16>,
	tokens:     Vec<Vec<u16>>,
}

impl Query {
	fn new(query: &str) -> Option<Self> {
		let normalized = normalize(query);
		if normalized.is_empty() {
			return None;
		}
		let tokens = normalized
			.split(|unit| *unit == 32)
			.map(<[u16]>::to_vec)
			.collect();
		let compact = normalized
			.iter()
			.copied()
			.filter(|unit| *unit != 32)
			.collect();
		Some(Self { normalized, compact, tokens })
	}
}

fn occurrence(text: &[u16], needle: &[u16], from: usize) -> Option<usize> {
	if needle.is_empty() || from > text.len() {
		return None;
	}
	text[from..]
		.windows(needle.len())
		.position(|window| window == needle)
		.map(|at| at + from)
}

const fn boundary(text: &[u16], at: usize, len: usize) -> bool {
	(at == 0 || text[at - 1] == 32) && (at + len == text.len() || text[at + len] == 32)
}

fn phrase_at(text: &[u16], needle: &[u16]) -> Option<usize> {
	let first = occurrence(text, needle, 0)?;
	if boundary(text, first, needle.len()) {
		return Some(first);
	}
	if first == 0 || text[first - 1] == 32 || needle.len() < SHADOW_RESCAN_UNITS {
		return None;
	}
	let mut from = first + 1;
	while let Some(at) = occurrence(text, needle, from) {
		if boundary(text, at, needle.len()) {
			return Some(at);
		}
		from = at + 1;
	}
	None
}

fn compact_at(index: &SearchIndex, needle: &[u16]) -> Option<usize> {
	let first = occurrence(&index.compact, needle, 0)?;
	if index.starts.binary_search(&first).is_ok() {
		return Some(first);
	}
	if needle.len() < SHADOW_RESCAN_UNITS {
		return None;
	}
	let mut from = first + 1;
	while let Some(at) = occurrence(&index.compact, needle, from) {
		if index.starts.binary_search(&at).is_ok() {
			return Some(at);
		}
		from = at + 1;
	}
	None
}

#[allow(clippy::cast_precision_loss, reason = "search positions are bounded by input allocation")]
fn position(score: f64, at: usize) -> f64 {
	score + at as f64 * 0.01
}

#[allow(clippy::cast_precision_loss, reason = "search lengths are bounded by input allocation")]
fn character_score(query: &[u16], text: &[u16]) -> Option<(f64, usize)> {
	if query.len() > text.len() {
		return None;
	}
	let mut query_at = 0;
	let mut first = None;
	let mut last = None;
	let mut consecutive: usize = 0;
	let mut score = 0.0;
	for (at, unit) in text.iter().enumerate() {
		if query_at == query.len() {
			break;
		}
		if *unit != query[query_at] {
			continue;
		}
		first.get_or_insert(at);
		if last.is_some_and(|previous| previous + 1 == at) {
			consecutive += 1;
			score -= consecutive as f64 * 5.0;
		} else {
			consecutive = 0;
			if let Some(previous) = last {
				score += (at - previous - 1) as f64 * 2.0;
			}
		}
		score += at as f64 * 0.1;
		last = Some(at);
		query_at += 1;
	}
	if query_at < query.len() {
		return None;
	}
	Some((score, last? - first? + 1))
}

#[allow(clippy::cast_precision_loss, reason = "search lengths are bounded by input allocation")]
fn word_score(token: &[u16], word: &Word) -> Option<f64> {
	let text = &word.text;
	let score = if text == token {
		-200.0
	} else if text.starts_with(token) {
		-170.0 + (text.len() - token.len()) as f64 * 0.5
	} else if token.starts_with(text) && token.len() - text.len() <= 2 {
		-150.0 + (token.len() - text.len()) as f64
	} else if let Some(at) = occurrence(text, token, 0) {
		-20.0 + at as f64
	} else {
		let (score, span) = character_score(token, text)?;
		// ceil(length * 1.8), with no lossy float-to-index conversion.
		let max_span = (token.len() + 2).max((token.len() * 9).div_ceil(5));
		if span > max_span {
			return None;
		}
		-40.0 + score
	};
	Some(position(score, word.index))
}

#[allow(clippy::cast_precision_loss, reason = "acronym spans are at most six words")]
fn acronym_score(token: &[u16], index: &SearchIndex) -> Option<f64> {
	if !(2..=4).contains(&token.len()) || index.words.is_empty() {
		return None;
	}
	let mut query_at = 0;
	let mut first = None;
	let mut last = 0;
	let mut text_at = 0;
	for word in &index.words {
		if word.text.first() != Some(&token[query_at]) {
			continue;
		}
		if first.is_none() {
			first = Some(word.ordinal);
			text_at = word.index;
		}
		last = word.ordinal;
		query_at += 1;
		if query_at == token.len() {
			break;
		}
	}
	if query_at < token.len() {
		return None;
	}
	let span = last - first? + 1;
	if span > token.len() + 2 {
		return None;
	}
	Some(position(-30.0 + span as f64 * 4.0 - token.len() as f64 * 2.0, text_at))
}

fn direct_score(token: &[u16], index: &SearchIndex) -> Option<f64> {
	let mut best = compact_at(index, token).map(|at| position(-140.0, at));
	for score in index
		.words
		.iter()
		.filter_map(|word| word_score(token, word))
		.chain(acronym_score(token, index))
	{
		if best.is_none_or(|previous| score < previous) {
			best = Some(score);
		}
	}
	best
}

fn token_score(token: &[u16], index: &SearchIndex) -> Option<f64> {
	if let Some(score) = direct_score(token, index) {
		return Some(score);
	}
	let mut best = None;
	for at in 0..token.len().saturating_sub(1) {
		let alpha = |unit| (97..=122).contains(&unit);
		let digit = |unit| (48..=57).contains(&unit);
		if !((alpha(token[at]) && digit(token[at + 1])) || (digit(token[at]) && alpha(token[at + 1])))
		{
			continue;
		}
		let mut variant = token.to_vec();
		variant.swap(at, at + 1);
		if let Some(score) = direct_score(&variant, index) {
			let score = score + 5.0;
			if best.is_none_or(|previous| score < previous) {
				best = Some(score);
			}
		}
	}
	best
}

fn evaluate(query: &Query, index: &SearchIndex) -> Match {
	if index.words.is_empty() {
		return Match::NONE;
	}
	let mut score = 0.0;
	if let Some(at) = phrase_at(&index.normalized, &query.normalized) {
		score -= 1000.0;
		score += position(0.0, at);
	}
	if let Some(at) = compact_at(index, &query.compact) {
		score -= 1200.0;
		score += position(0.0, at);
	}
	for token in &query.tokens {
		let Some(value) = token_score(token, index) else {
			return Match::NONE;
		};
		score += value;
	}
	Match::scored(score)
}

/// Prepared text for long corpora; its index lives exactly as long as the
/// owner.
pub struct PreparedText(SearchIndex);

impl PreparedText {
	#[must_use]
	pub fn new(text: &str) -> Self {
		Self(SearchIndex::new(text))
	}

	#[must_use]
	pub fn matches(&self, query: &str) -> Match {
		Query::new(query).map_or(Match::EMPTY, |query| evaluate(&query, &self.0))
	}
}

/// A bounded index cache owned by a selector/binding environment, not a process
/// singleton. Ranking normalizes a query once for the complete candidate batch.
#[derive(Default)]
pub struct Matcher {
	cache: HashMap<String, SearchIndex>,
}

impl Matcher {
	pub fn clear(&mut self) {
		self.cache.clear();
	}

	fn evaluate_text(&mut self, query: &Query, text: &str) -> Match {
		if let Some(index) = self.cache.get(text) {
			return evaluate(query, index);
		}
		let index = SearchIndex::new(text);
		let result = evaluate(query, &index);
		if self.cache.len() < CACHE_ENTRIES && text.encode_utf16().count() <= CACHE_TEXT_UNITS {
			self.cache.insert(text.to_owned(), index);
		}
		result
	}

	#[must_use]
	pub fn matches(&mut self, query: &str, text: &str) -> Match {
		Query::new(query).map_or(Match::EMPTY, |query| self.evaluate_text(&query, text))
	}

	pub fn rank(&mut self, query: &str, texts: &[String]) -> Vec<(usize, f64)> {
		let query = Query::new(query);
		let mut ranked = Vec::new();
		for (at, text) in texts.iter().enumerate() {
			let result = query
				.as_ref()
				.map_or(Match::EMPTY, |query| self.evaluate_text(query, text));
			if result.matches {
				ranked.push((at, result.score));
			}
		}
		// Stable ties retain the original candidate order, as in Array.sort.
		ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
		ranked
	}
}
