//! Bounded port of the legacy producer's lexical difficulty heuristics. These
//! metrics are selection hints, not AST semantics or benchmark result scores.
use std::{
	collections::{BTreeMap, BTreeSet},
	sync::LazyLock,
};

use anyhow::{Result, ensure};
use harvest_runtime::cancel::CancelToken;
use regex::Regex;
use serde::Serialize;

pub const DIFFICULTY_VERSION: &str = "rust-difficulty-v1";
#[derive(Debug, Clone, Copy, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
	Easy,
	Medium,
	Hard,
	Nightmare,
}
impl Difficulty {
	pub const fn minimum(self) -> u32 {
		match self {
			Self::Easy => 0,
			Self::Medium => 2,
			Self::Hard => 5,
			Self::Nightmare => 8,
		}
	}

	pub fn prefers(self, metrics: &DifficultyMetrics) -> bool {
		match self {
			Self::Easy => metrics.line_count < 150 && metrics.repeated_line_groups < 3,
			Self::Medium => (100..=300).contains(&metrics.line_count),
			Self::Hard => metrics.line_count > 200 && metrics.similar_blocks >= 3,
			Self::Nightmare => metrics.repeated_line_groups > 0 && metrics.line_count > 200,
		}
	}
}
#[derive(Debug, Clone, Serialize)]
pub struct FunctionRange {
	pub name:  String,
	pub start: usize,
	pub end:   usize,
}
#[derive(Debug, Clone, Serialize)]
pub struct DifficultyMetrics {
	pub version:              &'static str,
	pub line_count:           usize,
	pub repeated_line_groups: usize,
	pub similar_blocks:       usize,
	pub density:              f64,
	pub max_indent:           usize,
	pub function_ranges:      Vec<FunctionRange>,
}
// ECMAScript whitespace, rather than Rust's broader Unicode White_Space.
fn space(character: char) -> bool {
	matches!(character, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
		'\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn trim(line: &str) -> &str {
	line.trim_matches(space)
}
fn indent(line: &str) -> usize {
	line
		.chars()
		.take_while(|c| space(*c))
		.map(char::len_utf16)
		.sum()
}
fn trivial(line: &str) -> bool {
	matches!(line, "{" | "}" | "};" | ");" | "});" | "return;" | "break;" | "continue;" | "")
}
const WS: &str = r"[\x09-\x0D\x20\x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
static FUNCTIONS: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(&format!(
	r"(?:function{WS}+([A-Za-z0-9_]+)|(?:const|let|var){WS}+([A-Za-z0-9_]+){WS}*={WS}*(?:async{WS}*)?\([^)]*\){WS}*=>)"
)).expect("static function heuristic")
});
static BLOCKS: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(&format!(
	r"(?:function{WS}+([A-Za-z0-9_]+)|const{WS}+([A-Za-z0-9_]+){WS}*={WS}*(?:async{WS}*)?\([^)]*\){WS}*=>)"
)).expect("static block heuristic")
});
static IDENTIFIERS: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?-u:\b)[a-z][a-zA-Z0-9]*(?-u:\b)").expect("static identifier heuristic")
});

pub(super) struct FileAnalysis<'a> {
	pub metrics: DifficultyMetrics,
	lines:       Vec<&'a str>,
	repeated:    BTreeMap<&'a str, usize>,
	ambiguity:   BTreeMap<String, bool>,
	scores:      BTreeMap<usize, u32>,
	work:        usize,
}
impl<'a> FileAnalysis<'a> {
	pub fn new(code: &'a str, cancellation: &CancelToken) -> Result<Self> {
		ensure!(code.len() <= 256 * 1024, "difficulty source exceeds 256 KiB");
		let lines: Vec<_> = code.split('\n').collect();
		ensure!(lines.len() <= 4000, "difficulty source exceeds 4000 lines");
		let mut repeated = BTreeMap::new();
		let mut ranges = Vec::new();
		let mut current: Option<(String, usize)> = None;
		let mut depth = 0_i64;
		let mut max_indent = 0;
		for (index, line) in lines.iter().enumerate() {
			cancellation.heartbeat()?;
			let stripped = trim(line);
			if stripped.encode_utf16().count() >= 10 && !trivial(stripped) && stripped != "]," {
				*repeated.entry(stripped).or_insert(0) += 1;
			}
			if !stripped.is_empty() {
				max_indent = max_indent.max(indent(line));
			}
			if depth == 0
				&& let Some(found) = FUNCTIONS.captures(line)
			{
				if let Some((name, start)) = current.take() {
					ranges.push(FunctionRange { name, start, end: index });
				}
				current = Some((
					found
						.get(1)
						.or_else(|| found.get(2))
						.expect("static capture")
						.as_str()
						.into(),
					index + 1,
				));
			}
			depth += line.bytes().filter(|b| *b == b'{').count() as i64
				- line.bytes().filter(|b| *b == b'}').count() as i64;
			if depth == 0
				&& line.contains('}')
				&& let Some((name, start)) = current.take()
			{
				ranges.push(FunctionRange { name, start, end: index + 1 });
			}
		}
		if let Some((name, start)) = current {
			ranges.push(FunctionRange { name, start, end: lines.len() });
		}
		repeated.retain(|_, count| *count >= 2);
		let units = code.encode_utf16().count();
		let non_space: usize = code
			.chars()
			.filter(|c| !space(*c))
			.map(char::len_utf16)
			.sum();
		let metrics = DifficultyMetrics {
			version: DIFFICULTY_VERSION,
			line_count: lines.len(),
			repeated_line_groups: repeated.len(),
			similar_blocks: BLOCKS.find_iter(code).count(),
			density: if units == 0 {
				0.0
			} else {
				non_space as f64 / units as f64
			},
			max_indent,
			function_ranges: ranges,
		};
		Ok(Self {
			metrics,
			lines,
			repeated,
			ambiguity: BTreeMap::new(),
			scores: BTreeMap::new(),
			work: 0,
		})
	}

	fn ambiguous(&mut self, line: usize, cancellation: &CancelToken) -> Result<bool> {
		let Some(name) = self
			.metrics
			.function_ranges
			.iter()
			.find(|range| (range.start..=range.end).contains(&line))
			.map(|range| range.name.clone())
		else {
			return Ok(false);
		};
		if let Some(value) = self.ambiguity.get(&name) {
			return Ok(*value);
		}
		// Retain the legacy heuristic's first same-named range behavior. This is
		// deliberately not a claim that regex ranges resolve lexical bindings.
		let range = self
			.metrics
			.function_ranges
			.iter()
			.find(|range| range.name == name)
			.expect("known function name");
		let mut identifiers = BTreeSet::new();
		for text in &self.lines[range.start - 1..range.end] {
			cancellation.heartbeat()?;
			for found in IDENTIFIERS.find_iter(text) {
				identifiers.insert(found.as_str());
			}
		}
		ensure!(identifiers.len() <= 4096, "difficulty identifier set exceeds 4096");
		let mut pairs = 0;
		'compare: for a in &identifiers {
			cancellation.heartbeat()?;
			for b in &identifiers {
				self.work += 1;
				ensure!(self.work <= 1_000_000, "difficulty similarity work exceeds one million pairs");
				if a != b && (a.contains(*b) || b.contains(*a)) {
					pairs += 1;
					if pairs >= 3 {
						break 'compare;
					}
				}
			}
		}
		let ambiguous = pairs >= 3;
		self.ambiguity.insert(name, ambiguous);
		Ok(ambiguous)
	}

	pub fn score(&mut self, line: usize, cancellation: &CancelToken) -> Result<u32> {
		cancellation.heartbeat()?;
		ensure!((1..=self.lines.len()).contains(&line), "difficulty line is outside source");
		if let Some(score) = self.scores.get(&line) {
			return Ok(*score);
		}
		let metrics = &self.metrics;
		let mut score = if metrics.line_count > 300 {
			3
		} else if metrics.line_count > 150 {
			1
		} else {
			0
		};
		if line as f64 >= metrics.line_count as f64 * 0.33
			&& line as f64 <= metrics.line_count as f64 * 0.66
		{
			score += 2;
		}
		score += self
			.repeated
			.get(trim(self.lines[line - 1]))
			.copied()
			.unwrap_or(0)
			.min(5) as u32;
		score += if metrics.similar_blocks >= 5 {
			3
		} else if metrics.similar_blocks >= 3 {
			1
		} else {
			0
		};
		score += if metrics.density > 0.75 {
			2
		} else if metrics.density > 0.65 {
			1
		} else {
			0
		};
		let indentation = indent(self.lines[line - 1]);
		score += if indentation >= 16 {
			2
		} else if indentation >= 8 {
			1
		} else {
			0
		};
		if self.lines[line.saturating_sub(3)..self.lines.len().min(line + 2)]
			.iter()
			.filter(|text| trivial(trim(text)))
			.count()
			>= 3
		{
			score += 2;
		}
		if self.ambiguous(line, cancellation)? {
			score += 2;
		}
		self.scores.insert(line, score);
		Ok(score)
	}
}
/// Inspect bounded lexical metrics without executing or changing source data.
pub fn difficulty_metrics(code: &str, cancellation: &CancelToken) -> Result<DifficultyMetrics> {
	Ok(FileAnalysis::new(code, cancellation)?.metrics)
}
