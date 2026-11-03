//! Native selector grammar. Inclusive ranges share pi-ast's line range type;
//! source loading, decoding and displayed-line authority remain in the read
//! owner.
use std::path::Path;

use anyhow::{Result, bail, ensure};
use harvest_runtime::JailedFs;
use pi_ast::block::LineRange;

#[derive(Default)]
pub(super) struct Selection {
	pub raw: bool,
	tail:    Option<u32>,
	ranges:  Vec<LineRange>,
}
impl Selection {
	pub fn parse(input: &str) -> Result<Self> {
		ensure!(!input.is_empty() && input.len() <= 4096, "read selector exceeds limit");
		let mut selection = Self::default();
		let chunks: Vec<_> = input.split(':').collect();
		ensure!(chunks.len() <= 2, "invalid compound read selector");
		let mut range = None;
		for chunk in chunks {
			if chunk.eq_ignore_ascii_case("raw") {
				ensure!(!selection.raw, "duplicate raw selector");
				selection.raw = true;
			} else {
				ensure!(range.is_none(), "only one range-list or tail selector is allowed");
				range = Some(chunk);
			}
		}
		let Some(range) = range else {
			return Ok(selection);
		};
		if range.eq_ignore_ascii_case("conflicts") || range.eq_ignore_ascii_case("img") {
			bail!(
				"this read selector requires a retained conflict/image reader not connected to the \
				 native tool yet"
			);
		}
		if let Some(count) = range.strip_prefix('-') {
			selection.tail = Some(number(count)?);
			return Ok(selection);
		}
		ensure!(range.split(',').count() <= 32, "read selector exceeds 32 ranges");
		for chunk in range.split(',') {
			let chunk = chunk.strip_prefix(['L', 'l']).unwrap_or(chunk);
			let split = chunk
				.char_indices()
				.find(|(_, c)| !c.is_ascii_digit())
				.map(|(i, _)| i);
			let (start, end) = if let Some(split) = split {
				let start = number(&chunk[..split])?;
				let rest = &chunk[split..];
				let (count, rest) = if let Some(rest) = rest.strip_prefix('+') {
					(true, rest)
				} else if let Some(rest) = rest.strip_prefix("..") {
					(false, rest)
				} else if let Some(rest) = rest.strip_prefix('-') {
					(false, rest)
				} else {
					bail!("invalid read range separator");
				};
				let rest = rest.strip_prefix(['L', 'l']).unwrap_or(rest);
				let end = if count {
					start
						.checked_add(number(rest)? - 1)
						.ok_or_else(|| anyhow::anyhow!("read range overflow"))?
				} else if rest.is_empty() {
					u32::MAX
				} else {
					number(rest)?
				};
				(start, end)
			} else {
				(number(chunk)?, u32::MAX)
			};
			ensure!(end >= start, "read range ends before its start");
			selection
				.ranges
				.push(LineRange { start_line: start, end_line: end });
		}
		selection.ranges.sort_by_key(|r| r.start_line);
		let mut merged: Vec<LineRange> = Vec::new();
		for range in std::mem::take(&mut selection.ranges) {
			if let Some(last) = merged.last_mut() {
				if range.start_line <= last.end_line.saturating_add(1) {
					last.end_line = last.end_line.max(range.end_line);
					continue;
				}
			}
			merged.push(range);
		}
		selection.ranges = merged;
		Ok(selection)
	}

	pub fn ranges(&self, total: usize, offset: u32, limit: u32) -> Result<Vec<LineRange>> {
		let total = u32::try_from(total)?;
		let source = if let Some(tail) = self.tail {
			vec![LineRange {
				start_line: total.saturating_sub(tail).saturating_add(1).max(1),
				end_line:   total,
			}]
		} else if self.ranges.is_empty() {
			vec![LineRange {
				start_line: offset,
				end_line:   offset.saturating_add(limit - 1).min(total),
			}]
		} else {
			self.ranges.clone()
		};
		let mut result = Vec::new();
		for range in source {
			ensure!(
				range.start_line <= total && range.start_line > 0,
				"read range is past end of file"
			);
			result.push(LineRange {
				start_line: range.start_line,
				end_line:   range.end_line.min(total),
			});
		}
		Ok(result)
	}
}
fn number(value: &str) -> Result<u32> {
	ensure!(
		!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
		"read selector requires a positive line number/count"
	);
	let number: u32 = value.parse()?;
	ensure!(number > 0, "read selector requires a positive line number/count");
	Ok(number)
}
fn looks_like_selector(value: &str) -> bool {
	matches!(value.to_ascii_lowercase().as_str(), "raw" | "conflicts" | "img")
		|| (!value.is_empty()
			&& value
				.bytes()
				.all(|b| b.is_ascii_digit() || matches!(b, b'L' | b'l' | b',' | b'-' | b'+' | b'.')))
}

/// Literal entries (including symlinks) and ambiguous lookup errors take
/// precedence. Only a confirmed absent entry can be interpreted as
/// path:selector.
pub(super) fn split(fs: &JailedFs, input: &str) -> Result<(String, Option<String>)> {
	let Some((base, suffix)) = input.rsplit_once(':') else {
		return Ok((input.into(), None));
	};
	if base.is_empty() || !looks_like_selector(suffix) {
		return Ok((input.into(), None));
	}
	match fs.entry_exists(Path::new(input)) {
		Ok(false) => {},
		_ => return Ok((input.into(), None)),
	}
	if let Some((path, inner)) = base.rsplit_once(':') {
		if !path.is_empty()
			&& ((inner.eq_ignore_ascii_case("raw") && looks_like_selector(suffix))
				|| (suffix.eq_ignore_ascii_case("raw") && looks_like_selector(inner)))
		{
			return Ok((path.into(), Some(format!("{inner}:{suffix}"))));
		}
	}
	Ok((base.into(), Some(suffix.into())))
}
