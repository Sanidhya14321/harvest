//! Thin bindings to the reusable terminal engines.
use harvest_terminal::{fuzzy, input, output, printable};
use napi::{
	JsString, Result,
	bindgen_prelude::{Either, Uint32Array, Utf16String},
};
use napi_derive::napi;

use crate::js;

#[napi(object)]
pub struct SelectorFuzzyMatch {
	pub matches: bool,
	pub score:   f64,
}

impl From<fuzzy::Match> for SelectorFuzzyMatch {
	fn from(value: fuzzy::Match) -> Self {
		Self { matches: value.matches, score: value.score }
	}
}

#[napi(object)]
pub struct SelectorFuzzyRank {
	pub index: u32,
	pub score: f64,
}

#[napi]
pub struct SelectorFuzzyMatcher {
	matcher: fuzzy::Matcher,
}

#[napi]
impl SelectorFuzzyMatcher {
	#[napi(constructor)]
	#[must_use]
	pub fn new() -> Self {
		Self { matcher: fuzzy::Matcher::default() }
	}

	#[napi]
	pub fn matches(&mut self, query: String, text: String) -> SelectorFuzzyMatch {
		self.matcher.matches(&query, &text).into()
	}

	#[napi]
	pub fn rank(&mut self, query: String, texts: Vec<String>) -> Result<Vec<SelectorFuzzyRank>> {
		self
			.matcher
			.rank(&query, &texts)
			.into_iter()
			.map(|(index, score)| {
				Ok(SelectorFuzzyRank {
					index: u32::try_from(index)
						.map_err(|_| napi::Error::from_reason("Too many fuzzy candidates"))?,
					score,
				})
			})
			.collect()
	}

	#[napi]
	pub fn clear(&mut self) {
		self.matcher.clear();
	}
}

impl Default for SelectorFuzzyMatcher {
	fn default() -> Self {
		Self::new()
	}
}

#[napi]
pub struct SelectorFuzzyText {
	text: fuzzy::PreparedText,
}

#[napi]
impl SelectorFuzzyText {
	#[napi(constructor)]
	#[must_use]
	pub fn new(text: String) -> Self {
		Self { text: fuzzy::PreparedText::new(&text) }
	}

	#[napi]
	#[must_use]
	pub fn matches(&self, query: String) -> SelectorFuzzyMatch {
		self.text.matches(&query).into()
	}
}

#[napi]
pub fn coalesce_terminal_sgr(line: JsString<'_>) -> Result<Either<JsString<'_>, Utf16String>> {
	let units = js::utf16(line)?;
	Ok(output::coalesce_sgr(&units)
		.map_or(Either::A(line), |units| Either::B(Utf16String::from(units))))
}

#[napi]
pub fn chunk_terminal_output(data: JsString<'_>, max_bytes: f64) -> Result<Vec<Utf16String>> {
	let units = js::utf16(data)?;
	Ok(output::conpty_chunks(&units, max_bytes)
		.into_iter()
		.map(|range| Utf16String::from(units[range].to_vec()))
		.collect())
}

#[napi]
pub fn can_prefix_terminal_width(line: JsString<'_>) -> Result<bool> {
	Ok(output::can_prefix_width(&js::utf16(line)?))
}

#[napi(object)]
pub struct TerminalInputScan {
	pub ranges:             Uint32Array,
	pub remainder_start:    u32,
	pub resume_search_from: u32,
	pub discard_from:       Option<u32>,
}

/// Transitional host adapter; the native terminal calls the same plain engines.
#[napi]
#[derive(Default)]
pub struct TerminalInput;

#[napi]
impl TerminalInput {
	#[napi(constructor)]
	#[must_use]
	pub const fn new() -> Self {
		Self
	}

	#[napi]
	pub fn scan(&self, data: JsString<'_>, resume_search_from: u32) -> Result<TerminalInputScan> {
		let result = input::scan(&js::utf16(data)?, resume_search_from as usize);
		let offset = |value| {
			u32::try_from(value)
				.map_err(|_| napi::Error::from_reason("Terminal input offset exceeds host range"))
		};
		let mut ranges = Vec::with_capacity(result.sequences.len() * 2);
		for range in result.sequences {
			ranges.push(offset(range.start)?);
			ranges.push(offset(range.end)?);
		}
		Ok(TerminalInputScan {
			ranges:             Uint32Array::new(ranges),
			remainder_start:    offset(result.remainder_start)?,
			resume_search_from: offset(result.resume_search_from)?,
			discard_from:       result.discard_from.map(offset).transpose()?,
		})
	}

	#[napi]
	pub fn decode_printable_key(&self, data: JsString<'_>) -> Result<Option<Utf16String>> {
		Ok(printable::decode_key(&js::utf8(data)?).map(Utf16String::from))
	}

	#[napi]
	pub fn extract_printable_text(&self, data: JsString<'_>) -> Result<Option<Utf16String>> {
		Ok(printable::extract(&js::utf16(data)?).map(Utf16String::from))
	}

	#[napi]
	pub fn decode_keypad_text(&self, data: JsString<'_>) -> Result<Option<Utf16String>> {
		Ok(printable::keypad_text(&js::utf8(data)?).map(Utf16String::from))
	}
}
