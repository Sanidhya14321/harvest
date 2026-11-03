//! Exact hunk preparation shared by native fixture tooling. Reuses pi-diff's
//! jsdiff-compatible line engine; every prepared prompt is re-solved exactly.
use anyhow::{Result, ensure};
use harvest_runtime::cancel::CancelToken;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placement {
	pub start:     usize,
	pub old_len:   usize,
	pub new_lines: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedHunk {
	pub old_block:  Vec<String>,
	pub new_block:  Vec<String>,
	pub start_line: usize,
	pub unique:     bool,
}
fn occurrences(lines: &[String], block: &[String]) -> Vec<usize> {
	if block.is_empty() {
		return Vec::new();
	}
	lines
		.windows(block.len())
		.enumerate()
		.filter_map(|(i, v)| (v == block).then_some(i))
		.collect()
}
fn apply(lines: &[String], placements: &[Placement]) -> Option<Vec<String>> {
	let mut cursor = 0;
	let mut output = Vec::new();
	for p in placements {
		let end = p.start.checked_add(p.old_len)?;
		if p.start < cursor || end > lines.len() {
			return None;
		}
		output.extend_from_slice(&lines[cursor..p.start]);
		output.extend_from_slice(&p.new_lines);
		cursor = end;
	}
	output.extend_from_slice(&lines[cursor..]);
	Some(output)
}
pub fn placements_from_diff(
	input: &str,
	expected: &str,
	cancellation: &CancelToken,
) -> Result<Vec<Placement>> {
	ensure!(
		input.len() <= 256 * 1024 && expected.len() <= 256 * 1024,
		"hunk source exceeds 256 KiB limit"
	);
	let old = input.split('\n').count();
	let new = expected.split('\n').count();
	ensure!(
		old + new <= 4000 && old.saturating_mul(new) <= 1_000_000,
		"hunk diff exceeds line work budget"
	);
	cancellation.heartbeat()?;
	let mut placements = Vec::new();
	let mut pending: Option<Placement> = None;
	let mut position = 0;
	for change in pi_diff::line_changes_str(input, expected) {
		cancellation.heartbeat()?;
		let mut lines: Vec<String> = change.value.split('\n').map(str::to_owned).collect();
		if lines.last().is_some_and(String::is_empty) {
			lines.pop();
		}
		if !change.added && !change.removed {
			if let Some(p) = pending.take() {
				placements.push(p);
			}
			position += lines.len();
		} else {
			let p = pending.get_or_insert_with(|| Placement {
				start:     position,
				old_len:   0,
				new_lines: Vec::new(),
			});
			if change.removed {
				p.old_len += lines.len();
				position += lines.len();
			} else {
				p.new_lines.extend(lines);
			}
		}
	}
	if let Some(p) = pending {
		placements.push(p);
	}
	Ok(placements)
}
fn merge_nearby(input: &[String], placements: &[Placement]) -> Option<Vec<Placement>> {
	let mut merged: Vec<Placement> = Vec::new();
	for p in placements {
		let end = p.start.checked_add(p.old_len)?;
		if end > input.len() {
			return None;
		}
		if let Some(previous) = merged.last_mut() {
			let previous_end = previous.start.checked_add(previous.old_len)?;
			let gap = p.start.checked_sub(previous_end)?;
			if gap <= 2 {
				previous.old_len += gap + p.old_len;
				previous
					.new_lines
					.extend_from_slice(&input[previous_end..p.start]);
				previous.new_lines.extend_from_slice(&p.new_lines);
				continue;
			}
		}
		merged.push(p.clone());
	}
	Some(merged)
}
fn take_above(h: &mut RenderedHunk, input: &[String], floor: usize) -> bool {
	if h.start_line <= 1 || h.start_line - 2 < floor {
		return false;
	}
	h.start_line -= 1;
	let line = &input[h.start_line - 1];
	h.old_block.insert(0, line.clone());
	h.new_block.insert(0, line.clone());
	true
}
fn take_below(h: &mut RenderedHunk, input: &[String], ceiling: usize) -> bool {
	let at = h.start_line - 1 + h.old_block.len();
	if at >= input.len() || at >= ceiling {
		return false;
	}
	h.old_block.push(input[at].clone());
	h.new_block.push(input[at].clone());
	true
}
pub fn render_hunks(input: &[String], placements: &[Placement]) -> Option<Vec<RenderedHunk>> {
	let merged = merge_nearby(input, placements)?;
	let mut hunks = Vec::new();
	let mut floor = 0;
	for (i, p) in merged.iter().enumerate() {
		let ceiling = merged.get(i + 1).map_or(input.len(), |p| p.start);
		let old = &input[p.start..p.start + p.old_len];
		let prefix = old
			.iter()
			.zip(&p.new_lines)
			.take_while(|(a, b)| a == b)
			.count();
		let suffix = old[prefix..]
			.iter()
			.rev()
			.zip(p.new_lines[prefix..].iter().rev())
			.take_while(|(a, b)| a == b)
			.count();
		let mut h = RenderedHunk {
			old_block:  old[prefix..old.len() - suffix].to_vec(),
			new_block:  p.new_lines[prefix..p.new_lines.len() - suffix].to_vec(),
			start_line: p.start + prefix + 1,
			unique:     true,
		};
		if h.old_block.is_empty() && h.new_block.is_empty() {
			continue;
		}
		if h.old_block.is_empty() || h.new_block.is_empty() {
			let visible_above = h.start_line >= 2
				&& h.start_line - 2 >= floor
				&& !input[h.start_line - 2].trim().is_empty();
			if visible_above {
				take_above(&mut h, input, floor);
			} else if !take_below(&mut h, input, ceiling) && !take_above(&mut h, input, floor) {
				return None;
			}
		}
		let mut extensions = 0;
		while occurrences(input, &h.old_block).len() > 1 {
			if extensions >= 10 || !take_above(&mut h, input, floor) {
				h.unique = false;
				break;
			}
			extensions += 1;
		}
		for _ in 0..2 {
			if !(h.old_block.first()?.trim().is_empty() || h.new_block.first()?.trim().is_empty())
				|| !take_above(&mut h, input, floor)
			{
				break;
			}
		}
		for _ in 0..2 {
			if !(h.old_block.last()?.trim().is_empty() || h.new_block.last()?.trim().is_empty())
				|| !take_below(&mut h, input, ceiling)
			{
				break;
			}
		}
		floor = p.start + p.old_len;
		hunks.push(h);
	}
	(!hunks.is_empty()).then_some(hunks)
}
pub fn solve_hunks(input: &[String], hunks: &[RenderedHunk]) -> Option<Vec<String>> {
	let mut placements = Vec::new();
	for h in hunks {
		let matches = occurrences(input, &h.old_block);
		let start = if h.unique {
			if matches.len() != 1 {
				return None;
			}
			matches[0]
		} else {
			let start = h.start_line.checked_sub(1)?;
			if !matches.contains(&start) {
				return None;
			}
			start
		};
		placements.push(Placement {
			start,
			old_len: h.old_block.len(),
			new_lines: h.new_block.clone(),
		});
	}
	placements.sort_by_key(|p| p.start);
	apply(input, &placements)
}
/// Exact reconstruction is the acceptance condition. Bounds/final-newline-only
/// changes that cannot be represented fail explicitly rather than weaken it.
pub fn prepare_hunks(
	input: &str,
	expected: &str,
	cancellation: &CancelToken,
) -> Result<Vec<RenderedHunk>> {
	let placements = placements_from_diff(input, expected, cancellation)?;
	let lines: Vec<_> = input.split('\n').map(str::to_owned).collect();
	let hunks = render_hunks(&lines, &placements)
		.ok_or_else(|| anyhow::anyhow!("change cannot be rendered as anchored hunks"))?;
	let solved = solve_hunks(&lines, &hunks)
		.ok_or_else(|| anyhow::anyhow!("hunk prompt is ambiguous or overlapping"))?;
	ensure!(solved.join("\n") == expected, "hunks do not reconstruct expected bytes");
	Ok(hunks)
}
/// Choose a fence longer than every content backtick run.
pub fn fence(hunks: &[RenderedHunk]) -> String {
	let longest = hunks
		.iter()
		.flat_map(|h| h.old_block.iter().chain(&h.new_block))
		.flat_map(|line| line.split(|c| c != '`'))
		.map(str::len)
		.max()
		.unwrap_or(0);
	"`".repeat(3.max(longest + 1))
}
