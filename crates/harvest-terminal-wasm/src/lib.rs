//! Synchronous, bounded ABI for the portable terminal payload. All application
//! algorithms live in `harvest-terminal`; generated JavaScript only marshals
//! requests and owns the host-side callbacks.
use std::{cell::RefCell, collections::HashMap};

use harvest_terminal::{fuzzy, output, text};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_PREPARED_TEXTS: usize = 65_536;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
	Matches { query: String, text: String },
	Rank { query: String, texts: Vec<String> },
	Clear,
	Prepare { text: String },
	PreparedMatch { handle: u32, query: String },
	Release { handle: u32 },
	Chunks { text: Vec<u16>, max_bytes: Value },
	Coalesce { text: Vec<u16> },
	CanPrefix { text: Vec<u16> },
}

#[derive(Default)]
struct Engine {
	matcher:     fuzzy::Matcher,
	prepared:    HashMap<u32, fuzzy::PreparedText>,
	next_handle: u32,
}

fn match_value(result: fuzzy::Match) -> Value {
	json!({ "matches": result.matches, "score": result.score })
}

impl Engine {
	fn execute(&mut self, request: Request) -> Result<Value, &'static str> {
		Ok(match request {
			Request::Matches { query, text } => match_value(self.matcher.matches(&query, &text)),
			Request::Rank { query, texts } => {
				let ranks: Vec<Value> = self
					.matcher
					.rank(&query, &texts)
					.into_iter()
					.map(|(index, score)| json!({ "index": index, "score": score }))
					.collect();
				json!(ranks)
			},
			Request::Clear => {
				self.matcher.clear();
				Value::Null
			},
			Request::Prepare { text } => {
				if self.prepared.len() >= MAX_PREPARED_TEXTS {
					return Err("Too many live prepared fuzzy texts");
				}
				let handle = self
					.next_handle
					.checked_add(1)
					.ok_or("Prepared fuzzy text handles exhausted")?;
				self.next_handle = handle;
				self
					.prepared
					.insert(handle, fuzzy::PreparedText::new(&text));
				json!(handle)
			},
			Request::PreparedMatch { handle, query } => match_value(
				self
					.prepared
					.get(&handle)
					.ok_or("Prepared fuzzy text handle has been released")?
					.matches(&query),
			),
			Request::Release { handle } => {
				self.prepared.remove(&handle);
				Value::Null
			},
			Request::Chunks { text, max_bytes } => {
				let ranges: Vec<_> = output::conpty_chunks(
					&text,
					max_bytes
						.as_f64()
						.unwrap_or_else(|| match max_bytes.as_str() {
							Some("Infinity") => f64::INFINITY,
							Some("-Infinity") => f64::NEG_INFINITY,
							_ => f64::NAN,
						}),
				)
				.into_iter()
				.map(|range| [range.start, range.end])
				.collect();
				json!(ranges)
			},
			Request::Coalesce { text } => json!(output::coalesce_sgr(&text)),
			Request::CanPrefix { text } => json!(output::can_prefix_width(&text)),
		})
	}
}

fn layout_bounds(text: &[u16], dimensions: &[u32]) -> Result<(), &'static str> {
	if text.len() > 4 * 1024 * 1024 || dimensions.iter().any(|dimension| *dimension > 1024 * 1024) {
		return Err("Terminal text layout exceeds its input/dimension bound");
	}
	Ok(())
}

#[derive(Default)]
struct Bridge {
	input:    Vec<u8>,
	units:    Vec<u16>,
	response: Vec<u8>,
	engine:   Engine,
}

impl Bridge {
	fn ranges_response(&mut self, ranges: Vec<std::ops::Range<usize>>) -> *const u8 {
		if ranges.len() > MAX_REQUEST_BYTES / 8 {
			return std::ptr::null();
		}
		self.response.clear();
		for range in ranges {
			let (Ok(start), Ok(end)) = (u32::try_from(range.start), u32::try_from(range.end)) else {
				return std::ptr::null();
			};
			self.response.extend_from_slice(&start.to_le_bytes());
			self.response.extend_from_slice(&end.to_le_bytes());
		}
		self.response.as_ptr()
	}
}

thread_local! { static BRIDGE: RefCell<Bridge> = RefCell::new(Bridge::default()); }

/// Reserve host-writable linear memory. Zero indicates a rejected oversized
/// request. The pointer remains valid until `harvest_execute` or the next
/// reserve.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_reserve(length: usize) -> *mut u8 {
	if length > MAX_REQUEST_BYTES {
		return std::ptr::null_mut();
	}
	BRIDGE.with_borrow_mut(|bridge| {
		bridge.input.resize(length, 0);
		bridge.input.as_mut_ptr()
	})
}

/// Reserve aligned host-writable UTF-16 memory, bounded by the same byte cap.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_reserve_utf16(length: usize) -> *mut u16 {
	if length > MAX_REQUEST_BYTES / 2 {
		return std::ptr::null_mut();
	}
	BRIDGE.with_borrow_mut(|bridge| {
		bridge.units.resize(length, 0);
		bridge.units.as_mut_ptr()
	})
}

/// Execute the reserved request and return a host-readable response buffer.
///
/// Parsing errors are values, never traps; an invalid request cannot mutate the
/// engine. The response remains valid until the next execution.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_execute() -> *const u8 {
	BRIDGE.with_borrow_mut(|bridge| {
		let result = serde_json::from_slice::<Request>(&bridge.input)
			.map_err(|error| error.to_string())
			.and_then(|request| bridge.engine.execute(request).map_err(str::to_owned));
		let response = match result {
			Ok(value) => json!({ "value": value }),
			Err(error) => json!({ "error": error }),
		};
		bridge.response = serde_json::to_vec(&response).expect("JSON values serialize");
		bridge.input.clear();
		bridge.response.as_ptr()
	})
}

/// Byte length of the most recent response.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_response_len() -> usize {
	BRIDGE.with_borrow(|bridge| bridge.response.len())
}

/// Rank NUL-delimited UTF-8 candidates, returning packed index/f64 pairs.
///
/// The generated host uses the JSON ABI for inputs containing embedded NUL.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_rank() -> *const u8 {
	BRIDGE.with_borrow_mut(|bridge| {
		let Ok(input) = std::str::from_utf8(&bridge.input) else {
			return std::ptr::null();
		};
		let mut parts = input.split('\0');
		let query = parts.next().unwrap_or_default();
		let texts: Vec<String> = parts.map(str::to_owned).collect();
		let ranks = bridge.engine.matcher.rank(query, &texts);
		if ranks.len() > MAX_REQUEST_BYTES / 12 {
			return std::ptr::null();
		}
		bridge.response.clear();
		for (index, score) in ranks {
			let Ok(index) = u32::try_from(index) else {
				return std::ptr::null();
			};
			bridge.response.extend_from_slice(&index.to_le_bytes());
			bridge.response.extend_from_slice(&score.to_le_bytes());
		}
		bridge.response.as_ptr()
	})
}

/// Split a UTF-8 memory packet into packed UTF-16 start/end offsets.
///
/// Host UTF-8 encoding replaces unmatched surrogates with one three-byte
/// scalar, preserving their UTF-16 length and encoded width for slicing the
/// original.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_chunks(max_bytes: f64) -> *const u8 {
	BRIDGE.with_borrow_mut(|bridge| {
		let Ok(text) = std::str::from_utf8(&bridge.input) else {
			return std::ptr::null();
		};
		let chunks = output::conpty_utf8_chunks(text, max_bytes);
		bridge.ranges_response(chunks)
	})
}

/// Split a host-encoded UTF-16 packet into packed start/end offsets.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_chunks_utf16(max_bytes: f64) -> *const u8 {
	BRIDGE.with_borrow_mut(|bridge| {
		let chunks = output::conpty_chunks(&bridge.units, max_bytes);
		bridge.ranges_response(chunks)
	})
}

fn text_record(response: &mut Vec<u8>, units: &[u16], width: usize) -> Option<()> {
	let length = u32::try_from(units.len()).ok()?;
	let width = u32::try_from(width).ok()?;
	if response
		.len()
		.saturating_add(8)
		.saturating_add(units.len() * 2)
		> MAX_REQUEST_BYTES
	{
		return None;
	}
	response.extend_from_slice(&width.to_le_bytes());
	response.extend_from_slice(&length.to_le_bytes());
	for unit in units {
		response.extend_from_slice(&unit.to_le_bytes());
	}
	Some(())
}

/// Measure reserved UTF-16 directly, without JSON or Unicode replacement.
/// `u32::MAX` indicates an input limit violation.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_text_width(tab_width: u32) -> u32 {
	BRIDGE.with_borrow(|bridge| {
		if layout_bounds(&bridge.units, &[]).is_err() {
			return u32::MAX;
		}
		u32::try_from(text::visible_width(&bridge.units, tab_width)).unwrap_or(u32::MAX)
	})
}

/// Execute a bounded UTF-16 layout operation.
///
/// The output begins with a u32
/// unchanged-input flag, then records (u32 width, u32 unit count, UTF-16 LE).
/// Zero indicates invalid bounds or operation; no partial result is exposed.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_text(
	operation: u32,
	a: u32,
	b: u32,
	c: u32,
	flags: u32,
	tab_width: u32,
) -> *const u8 {
	BRIDGE.with_borrow_mut(|bridge| {
		bridge.response.clear();
		if layout_bounds(&bridge.units, &[a, b, c]).is_err() {
			return std::ptr::null();
		}
		bridge.response.extend_from_slice(&0u32.to_le_bytes());
		let result = match operation {
			0 => {
				let rows = text::wrap_text_with_ansi(&bridge.units, a, tab_width);
				rows
					.iter()
					.try_for_each(|row| text_record(&mut bridge.response, row, 0))
			},
			1 => {
				let ellipsis = match b {
					0 => text::Ellipsis::Unicode,
					1 => text::Ellipsis::Ascii,
					2 => text::Ellipsis::Omit,
					_ => return std::ptr::null(),
				};
				match text::truncate_to_width(
					&bridge.units,
					a,
					Some(ellipsis),
					Some(flags != 0),
					tab_width,
				) {
					std::borrow::Cow::Borrowed(_) => {
						bridge.response[0] = 1;
						Some(())
					},
					std::borrow::Cow::Owned(units) => text_record(&mut bridge.response, &units, 0),
				}
			},
			2 => {
				let (units, width) =
					text::slice_with_width(&bridge.units, a, b, Some(flags != 0), tab_width);
				text_record(&mut bridge.response, &units, width)
			},
			3 => {
				let (before, before_width, after, after_width) =
					text::extract_segments(&bridge.units, a, b, c, flags != 0, tab_width);
				text_record(&mut bridge.response, &before, before_width)
					.and_then(|()| text_record(&mut bridge.response, &after, after_width))
			},
			_ => None,
		};
		if result.is_none() {
			bridge.response.clear();
			return std::ptr::null();
		}
		bridge.response.as_ptr()
	})
}

/// Apply a terminal-probed Jamo policy to this engine instance.
#[unsafe(no_mangle)]
pub extern "C" fn harvest_text_jamo(value: u32) {
	text::set_hangul_compat_jamo_width_override(value as u8);
}

#[cfg(test)]
mod tests {
	use super::*;

	fn units(text: &str) -> String {
		text.to_owned()
	}

	#[test]
	fn released_prepared_handles_cannot_be_reused() {
		let mut engine = Engine::default();
		let handle = engine
			.execute(Request::Prepare { text: units("OpenCode Go") })
			.unwrap();
		let handle = u32::try_from(handle.as_u64().unwrap()).unwrap();
		assert_eq!(
			engine
				.execute(Request::PreparedMatch { handle, query: units("og") })
				.unwrap()["matches"],
			true
		);
		engine.execute(Request::Release { handle }).unwrap();
		assert!(
			engine
				.execute(Request::PreparedMatch { handle, query: units("og") })
				.is_err()
		);
		let next = engine
			.execute(Request::Prepare { text: units("Other") })
			.unwrap();
		assert_ne!(next, json!(handle));
	}

	#[test]
	fn oversized_request_is_rejected_before_allocation() {
		assert!(harvest_reserve(MAX_REQUEST_BYTES + 1).is_null());
	}
}
