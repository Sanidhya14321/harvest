//! Additional native producer policies. Composite offsets refer to one original
//! snapshot; they do not claim the historical sequential Babel selection
//! policy.
use super::*;

pub(super) fn regex_candidates(
	code: &str,
	root: Node<'_>,
	heartbeat: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<Candidate>> {
	let mut pending = vec![(root, 0)];
	let mut visited = 0;
	let mut regex_visited = 0;
	let mut result = Vec::new();
	while let Some((node, depth)) = pending.pop() {
		heartbeat()?;
		visited += 1;
		ensure!(visited <= 100_000 && depth <= 256, "mutation AST work budget exceeded");
		if node.kind() == "regex" {
			// Unicode-set notation has different character-class nesting rules.
			// Until the shared grammar supports it, never guess where +/* apply.
			if node
				.child_by_field_name("flags")
				.is_some_and(|n| code[n.byte_range()].contains('v'))
			{
				continue;
			}
			if let Some(pattern) = node.child_by_field_name("pattern") {
				let text = &code[pattern.byte_range()];
				let parsed = parse_cached(text, SupportLang::Regex)?
					.ok_or_else(|| anyhow::anyhow!("regex mutation parse interrupted"))?;
				heartbeat()?;
				if parsed.root_node().has_error() {
					continue;
				}
				let mut quantifiers = vec![(parsed.root_node(), 0)];
				while let Some((quantifier, depth)) = quantifiers.pop() {
					heartbeat()?;
					regex_visited += 1;
					ensure!(
						regex_visited <= 100_000 && depth <= 256,
						"regex mutation work budget exceeded"
					);
					let replacement = match quantifier.kind() {
						"one_or_more" => Some("*"),
						"zero_or_more" => Some("+"),
						_ => None,
					};
					if let Some(new) = replacement {
						let start = pattern.start_byte() + quantifier.start_byte();
						ensure!(result.len() < 4096, "regex mutation candidate budget exceeded");
						result.push(Candidate {
							kind:        MutationKind::SwapRegexQuantifier,
							line_number: pattern.start_position().row
								+ quantifier.start_position().row
								+ 1,
							edits:       vec![SourceEdit {
								start,
								end: start + 1,
								old: code[start..start + 1].into(),
								new: new.into(),
							}],
						});
					}
					quantifiers.extend(
						children(quantifier)
							.into_iter()
							.rev()
							.map(|n| (n, depth + 1)),
					);
				}
			}
		}
		pending.extend(children(node).into_iter().rev().map(|n| (n, depth + 1)));
	}
	result.sort_by_key(|c| c.edits[0].start);
	Ok(result)
}

const TOKEN_FAMILIES: [MutationKind; 13] = [
	MutationKind::SwapComparison,
	MutationKind::SwapEquality,
	MutationKind::SwapLogical,
	MutationKind::RemoveNegation,
	MutationKind::SwapIncrementDecrement,
	MutationKind::SwapArithmetic,
	MutationKind::FlipBoolean,
	MutationKind::RemoveOptionalChain,
	MutationKind::SwapCallArgs,
	MutationKind::SwapNullish,
	MutationKind::UnicodeHyphen,
	MutationKind::OffByOne,
	MutationKind::SwapRegexQuantifier,
];

pub(super) fn composite_candidates(
	code: &str,
	lang: SupportLang,
	heartbeat: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<Candidate>> {
	let mut pool = Vec::new();
	let mut pool_bytes = 0;
	for kind in TOKEN_FAMILIES {
		for candidate in candidates(code, lang, kind, heartbeat)? {
			pool_bytes += candidate
				.edits
				.iter()
				.map(|e| e.old.len() + e.new.len())
				.sum::<usize>();
			ensure!(
				pool.len() < 4096 && pool_bytes <= 8 * 1024 * 1024,
				"composite input budget exceeded"
			);
			pool.push(candidate);
		}
	}
	pool.sort_by_key(|c| c.edits.iter().map(|e| e.start).min().unwrap_or(0));
	let mut result = Vec::new();
	let mut bytes = 0;
	let mut work = 0;
	for (index, first) in pool.iter().enumerate() {
		let mut kinds = vec![first.kind];
		let mut edits = first.edits.clone();
		// Bounded neighborhood enumeration avoids a combinatorial powerset.
		for next in pool.iter().skip(index + 1).take(64) {
			heartbeat()?;
			work += 1;
			ensure!(work <= 262_144, "composite selection work budget exceeded");
			if kinds.contains(&next.kind)
				|| next
					.edits
					.iter()
					.any(|b| edits.iter().any(|a| a.start < b.end && b.start < a.end))
			{
				continue;
			}
			kinds.push(next.kind);
			edits.extend(next.edits.iter().cloned());
			let mut ordered = edits.clone();
			ordered.sort_by_key(|e| e.start);
			bytes += ordered
				.iter()
				.map(|e| e.old.len() + e.new.len())
				.sum::<usize>();
			ensure!(
				result.len() < 4096 && bytes <= 8 * 1024 * 1024,
				"composite candidate budget exceeded"
			);
			result.push(Candidate {
				kind:        MutationKind::CompositeMultiEdit,
				line_number: first.line_number,
				edits:       ordered,
			});
			if kinds.len() == 5 {
				break;
			}
		}
	}
	Ok(result)
}
