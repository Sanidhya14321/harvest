//! Versioned syntax-aware benchmark mutations. Uses the shared parse cache;
//! byte edits preserve formatting and source outside the selected ranges.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::{SupportLang, parse_cache::parse_cached};

mod bindings;
mod extended;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MutationKind {
	SwapComparison,
	SwapEquality,
	SwapLogical,
	RemoveNegation,
	SwapIncrementDecrement,
	SwapArithmetic,
	FlipBoolean,
	RemoveOptionalChain,
	SwapCallArgs,
	SwapNullish,
	UnicodeHyphen,
	OffByOne,
	SwapAdjacentLines,
	SwapIfElse,
	WrapRedundantIf,
	SwapSiblingBlocks,
	DuplicateBlock,
	RemoveCaseLabel,
	DuplicateLineFlip,
	MoveDistantBlock,
	SwapRegexQuantifier,
	CompositeMultiEdit,
	IdentifierMultiEdit,
}
impl MutationKind {
	pub const ALL: [Self; 23] = [
		Self::SwapComparison,
		Self::SwapEquality,
		Self::SwapLogical,
		Self::RemoveNegation,
		Self::SwapIncrementDecrement,
		Self::SwapArithmetic,
		Self::FlipBoolean,
		Self::RemoveOptionalChain,
		Self::SwapCallArgs,
		Self::SwapNullish,
		Self::UnicodeHyphen,
		Self::OffByOne,
		Self::SwapAdjacentLines,
		Self::SwapIfElse,
		Self::WrapRedundantIf,
		Self::SwapSiblingBlocks,
		Self::DuplicateBlock,
		Self::RemoveCaseLabel,
		Self::DuplicateLineFlip,
		Self::MoveDistantBlock,
		Self::SwapRegexQuantifier,
		Self::CompositeMultiEdit,
		Self::IdentifierMultiEdit,
	];

	pub fn name(self) -> &'static str {
		match self {
			Self::SwapComparison => "swap-comparison",
			Self::SwapEquality => "swap-equality",
			Self::SwapLogical => "swap-logical",
			Self::RemoveNegation => "remove-negation",
			Self::SwapIncrementDecrement => "swap-increment-decrement",
			Self::SwapArithmetic => "swap-arithmetic",
			Self::FlipBoolean => "flip-boolean",
			Self::RemoveOptionalChain => "remove-optional-chain",
			Self::SwapCallArgs => "swap-call-args",
			Self::SwapNullish => "swap-nullish",
			Self::UnicodeHyphen => "unicode-hyphen",
			Self::OffByOne => "off-by-one",
			Self::SwapAdjacentLines => "swap-adjacent-lines",
			Self::SwapIfElse => "swap-if-else",
			Self::WrapRedundantIf => "wrap-redundant-if",
			Self::SwapSiblingBlocks => "swap-sibling-blocks",
			Self::DuplicateBlock => "duplicate-block",
			Self::RemoveCaseLabel => "remove-case-label",
			Self::DuplicateLineFlip => "duplicate-line-flip",
			Self::MoveDistantBlock => "move-distant-block",
			Self::SwapRegexQuantifier => "swap-regex-quantifier",
			Self::CompositeMultiEdit => "composite-multi-edit",
			Self::IdentifierMultiEdit => "identifier-multi-edit",
		}
	}

	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|k| k.name() == name)
	}
}
#[derive(Debug, Clone, Serialize)]
pub struct SourceEdit {
	pub start: usize,
	pub end:   usize,
	pub old:   String,
	pub new:   String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
	pub kind:        MutationKind,
	pub line_number: usize,
	pub edits:       Vec<SourceEdit>,
}
fn edit(code: &str, node: Node<'_>, new: String) -> SourceEdit {
	SourceEdit {
		start: node.start_byte(),
		end: node.end_byte(),
		old: code[node.byte_range()].into(),
		new,
	}
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
	let mut cursor = node.walk();
	node
		.named_children(&mut cursor)
		.filter(|n| n.kind() != "comment")
		.collect()
}
fn is_statement(node: Node<'_>) -> bool {
	node.kind().ends_with("_statement")
		|| matches!(node.kind(), "lexical_declaration" | "variable_declaration")
}
fn swapped(code: &str, left: Node<'_>, right: Node<'_>) -> Vec<SourceEdit> {
	vec![
		edit(code, left, code[right.byte_range()].into()),
		edit(code, right, code[left.byte_range()].into()),
	]
}
fn span(node: Node<'_>) -> usize {
	node.end_position().row - node.start_position().row + 1
}
fn boundary(mut node: Node<'_>, code: &str) -> bool {
	for _ in 0..64 {
		let Some(parent) = node.parent() else {
			return false;
		};
		node = parent;
		if matches!(
			node.kind(),
			"for_statement" | "while_statement" | "do_statement" | "if_statement"
		) {
			return true;
		}
		if node.kind() == "binary_expression"
			&& node
				.child_by_field_name("operator")
				.is_some_and(|op| matches!(&code[op.byte_range()], "<" | "<=" | ">" | ">="))
		{
			return true;
		}
	}
	false
}
/// Sorted bounded candidates, never regex substitutions over arbitrary source.
/// The callback is checked before/after parsing and throughout the tree walk.
pub fn candidates(
	code: &str,
	lang: SupportLang,
	kind: MutationKind,
	heartbeat: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<Candidate>> {
	ensure!(
		matches!(lang, SupportLang::JavaScript | SupportLang::TypeScript | SupportLang::Tsx),
		"mutation language is unsupported"
	);
	ensure!(code.len() <= 256 * 1024, "mutation source exceeds 256 KiB");
	heartbeat()?;
	let tree =
		parse_cached(code, lang)?.ok_or_else(|| anyhow::anyhow!("mutation parse interrupted"))?;
	heartbeat()?;
	ensure!(!tree.root_node().has_error(), "mutation source contains syntax errors");
	if kind == MutationKind::IdentifierMultiEdit {
		return bindings::identifier_candidates(code, lang, tree.root_node(), heartbeat);
	}
	if kind == MutationKind::SwapRegexQuantifier {
		return extended::regex_candidates(code, tree.root_node(), heartbeat);
	}
	if kind == MutationKind::CompositeMultiEdit {
		return extended::composite_candidates(code, lang, heartbeat);
	}
	let mut duplicate_counts = std::collections::BTreeMap::new();
	if kind == MutationKind::DuplicateLineFlip {
		let mut pending = vec![tree.root_node()];
		let mut visited = 0;
		while let Some(node) = pending.pop() {
			heartbeat()?;
			visited += 1;
			ensure!(visited <= 100_000, "duplicate statement work budget exceeded");
			if is_statement(node) && node.kind() != "statement_block" {
				*duplicate_counts
					.entry(&code[node.byte_range()])
					.or_insert(0_usize) += 1;
			}
			pending.extend(children(node));
		}
	}
	let mut pending = vec![(tree.root_node(), 0)];
	let mut visited = 0;
	let mut result = Vec::new();
	let mut bytes = 0;
	while let Some((node, depth)) = pending.pop() {
		heartbeat()?;
		visited += 1;
		ensure!(visited <= 100_000 && depth <= 256, "mutation AST work budget exceeded");
		let mut edits = Vec::new();
		if let Some(operator) = node.child_by_field_name("operator") {
			let raw = &code[operator.byte_range()];
			let new = match (kind, raw, node.kind()) {
				(
					MutationKind::SwapComparison | MutationKind::DuplicateLineFlip,
					"<",
					"binary_expression",
				) => Some("<="),
				(
					MutationKind::SwapComparison | MutationKind::DuplicateLineFlip,
					"<=",
					"binary_expression",
				) => Some("<"),
				(
					MutationKind::SwapComparison | MutationKind::DuplicateLineFlip,
					">",
					"binary_expression",
				) => Some(">="),
				(
					MutationKind::SwapComparison | MutationKind::DuplicateLineFlip,
					">=",
					"binary_expression",
				) => Some(">"),
				(
					MutationKind::SwapEquality | MutationKind::DuplicateLineFlip,
					"===",
					"binary_expression",
				) => Some("!=="),
				(
					MutationKind::SwapEquality | MutationKind::DuplicateLineFlip,
					"!==",
					"binary_expression",
				) => Some("==="),
				(
					MutationKind::SwapEquality | MutationKind::DuplicateLineFlip,
					"==",
					"binary_expression",
				) => Some("!="),
				(
					MutationKind::SwapEquality | MutationKind::DuplicateLineFlip,
					"!=",
					"binary_expression",
				) => Some("=="),
				(MutationKind::SwapLogical, "&&", "binary_expression") => Some("||"),
				(MutationKind::SwapLogical, "||", "binary_expression") => Some("&&"),
				(MutationKind::SwapNullish, "??", "binary_expression") => Some("||"),
				(MutationKind::SwapArithmetic, "+", "binary_expression") => Some("-"),
				(MutationKind::SwapArithmetic, "-", "binary_expression") => Some("+"),
				(MutationKind::SwapArithmetic, "*", "binary_expression") => Some("/"),
				(MutationKind::SwapArithmetic, "/", "binary_expression") => Some("*"),
				(MutationKind::SwapIncrementDecrement, "++", "update_expression") => Some("--"),
				(MutationKind::SwapIncrementDecrement, "--", "update_expression") => Some("++"),
				(MutationKind::RemoveNegation, "!", "unary_expression") => Some(""),
				_ => None,
			};
			if let Some(new) = new {
				edits.push(edit(code, operator, new.into()));
			}
		}
		match kind {
			MutationKind::FlipBoolean | MutationKind::DuplicateLineFlip
				if matches!(node.kind(), "true" | "false") =>
			{
				edits.push(edit(
					code,
					node,
					if node.kind() == "true" {
						"false"
					} else {
						"true"
					}
					.into(),
				))
			},
			MutationKind::OffByOne if node.kind() == "number" && boundary(node, code) => {
				let new = match &code[node.byte_range()] {
					"0" => Some("1"),
					"1" => Some("0"),
					_ => None,
				};
				if let Some(new) = new {
					edits.push(edit(code, node, new.into()));
				}
			},
			MutationKind::OffByOne if node.kind() == "binary_expression" => {
				if let (Some(left), Some(right), Some(op)) = (
					node.child_by_field_name("left"),
					node.child_by_field_name("right"),
					node.child_by_field_name("operator"),
				) {
					let length = |n: Node<'_>| {
						n.kind() == "member_expression"
							&& n
								.child_by_field_name("property")
								.is_some_and(|p| &code[p.byte_range()] == "length")
					};
					match &code[op.byte_range()] {
						"<" | "<=" if length(right) => edits.push(edit(
							code,
							op,
							if &code[op.byte_range()] == "<" {
								"<="
							} else {
								"<"
							}
							.into(),
						)),
						"-" if length(left)
							&& right.kind() == "number"
							&& matches!(&code[right.byte_range()], "1" | "2") =>
						{
							edits.push(edit(
								code,
								right,
								if &code[right.byte_range()] == "1" {
									"2"
								} else {
									"1"
								}
								.into(),
							))
						},
						_ => {},
					}
				}
			},
			MutationKind::RemoveOptionalChain if node.kind() == "optional_chain" => {
				let member = node
					.parent()
					.is_some_and(|p| p.kind() == "member_expression");
				edits.push(edit(code, node, if member { "." } else { "" }.into()));
			},
			MutationKind::SwapCallArgs if node.kind() == "call_expression" => {
				if let Some(args) = node.child_by_field_name("arguments") {
					let args = children(args);
					if args.len() >= 2 && args[..2].iter().all(|n| n.kind() != "spread_element") {
						edits = swapped(code, args[0], args[1]);
					}
				}
			},
			MutationKind::UnicodeHyphen if node.kind() == "string_fragment" => {
				if let Some(offset) = code[node.byte_range()].find('-') {
					let start = node.start_byte() + offset;
					edits.push(SourceEdit { start, end: start + 1, old: "-".into(), new: "–".into() });
				}
			},
			MutationKind::SwapIfElse if node.kind() == "if_statement" => {
				if let (Some(left), Some(alternative)) =
					(node.child_by_field_name("consequence"), node.child_by_field_name("alternative"))
				{
					let right = if alternative.kind() == "else_clause" {
						children(alternative)
							.into_iter()
							.find(|n| n.kind() == "statement_block")
					} else {
						Some(alternative)
					};
					if let Some(right) = right
						.filter(|n| n.kind() == "statement_block" && left.kind() == "statement_block")
					{
						edits = swapped(code, left, right);
					}
				}
			},
			MutationKind::WrapRedundantIf
				if node.kind() == "statement_block" && !children(node).is_empty() =>
			{
				let text = &code[node.byte_range()];
				edits.push(edit(
					code,
					node,
					format!("{{\nif (true) {{ {} }}\n}}", &text[1..text.len() - 1]),
				));
			},
			MutationKind::DuplicateBlock
				if is_statement(node)
					&& (3..=120).contains(&span(node))
					&& matches!(
						node.kind(),
						"if_statement"
							| "expression_statement"
							| "for_statement"
							| "for_in_statement"
							| "while_statement"
							| "try_statement"
							| "switch_statement"
					) =>
			{
				edits.push(edit(
					code,
					node,
					format!("{}\n{}", &code[node.byte_range()], &code[node.byte_range()]),
				));
			},
			MutationKind::RemoveCaseLabel if node.kind() == "switch_case" => {
				let has_body = node.child_by_field_name("body").is_some();
				if !has_body && node.next_named_sibling().is_some() {
					edits.push(edit(code, node, String::new()));
				}
			},
			_ => {},
		}
		if kind == MutationKind::DuplicateLineFlip && !edits.is_empty() {
			let mut parent = node.parent();
			let mut repeated = false;
			while let Some(statement) = parent {
				if is_statement(statement) && statement.kind() != "statement_block" {
					repeated = duplicate_counts
						.get(&code[statement.byte_range()])
						.is_some_and(|&count| count >= 2);
					if repeated {
						break;
					}
				}
				parent = statement.parent();
			}
			if !repeated {
				edits.clear();
			}
		}
		if !edits.is_empty() && edits.iter().any(|e| e.old != e.new) {
			bytes += edits
				.iter()
				.map(|e| e.old.len() + e.new.len())
				.sum::<usize>();
			ensure!(
				result.len() < 4096 && bytes <= 8 * 1024 * 1024,
				"mutation candidate budget exceeded"
			);
			result.push(Candidate { kind, line_number: node.start_position().row + 1, edits });
		}
		let named = children(node);
		if matches!(kind, MutationKind::SwapAdjacentLines | MutationKind::SwapSiblingBlocks)
			&& matches!(node.kind(), "program" | "statement_block")
		{
			for pair in named.windows(2) {
				let (left, right) = (pair[0], pair[1]);
				let sizes = (span(left), span(right));
				let size_match = if kind == MutationKind::SwapAdjacentLines {
					sizes == (1, 1)
				} else {
					sizes.0.max(sizes.1) >= 3
						&& sizes.0 <= 80
						&& sizes.1 <= 80
						&& sizes.0 + sizes.1 <= 120
				};
				if is_statement(left)
					&& is_statement(right)
					&& size_match
					&& right
						.start_position()
						.row
						.saturating_sub(left.end_position().row)
						<= 2 && code[left.byte_range()] != code[right.byte_range()]
				{
					let edits = swapped(code, left, right);
					bytes += edits
						.iter()
						.map(|e| e.old.len() + e.new.len())
						.sum::<usize>();
					ensure!(
						result.len() < 4096 && bytes <= 8 * 1024 * 1024,
						"mutation candidate budget exceeded"
					);
					result.push(Candidate { kind, line_number: left.start_position().row + 1, edits });
				}
			}
		}
		if kind == MutationKind::MoveDistantBlock
			&& matches!(node.kind(), "program" | "statement_block")
			&& named.len() >= 5
		{
			for from in 0..named.len() - 1 {
				let moved = named[from];
				let next = named[from + 1];
				if !is_statement(moved) || !is_statement(next) || !(3..=60).contains(&span(moved)) {
					continue;
				}
				for target in named
					.iter()
					.skip(from + 3)
					.copied()
					.filter(|n| is_statement(*n))
				{
					heartbeat()?;
					let removed = &code[moved.start_byte()..next.start_byte()];
					let text = &code[moved.byte_range()];
					let edits = vec![
						SourceEdit {
							start: moved.start_byte(),
							end:   next.start_byte(),
							old:   removed.into(),
							new:   String::new(),
						},
						SourceEdit {
							start: target.end_byte(),
							end:   target.end_byte(),
							old:   String::new(),
							new:   format!("\n\n{text}"),
						},
					];
					bytes += edits
						.iter()
						.map(|e| e.old.len() + e.new.len())
						.sum::<usize>();
					ensure!(
						result.len() < 4096 && bytes <= 8 * 1024 * 1024,
						"mutation candidate budget exceeded"
					);
					result.push(Candidate { kind, line_number: moved.start_position().row + 1, edits });
				}
			}
		}
		for child in named.into_iter().rev() {
			pending.push((child, depth + 1));
		}
	}
	result.sort_by(|a, b| {
		a.edits[0]
			.start
			.cmp(&b.edits[0].start)
			.then(a.edits[0].end.cmp(&b.edits[0].end))
	});
	Ok(result)
}
/// Reject stale ranges, overlapping edits, unchanged output and invalid syntax.
pub fn apply(
	code: &str,
	lang: SupportLang,
	candidate: &Candidate,
	heartbeat: &mut dyn FnMut() -> Result<()>,
) -> Result<String> {
	ensure!(
		matches!(lang, SupportLang::JavaScript | SupportLang::TypeScript | SupportLang::Tsx),
		"mutation language is unsupported"
	);
	ensure!(
		code.len() <= 256 * 1024 && candidate.edits.len() <= 64,
		"mutation edit budget exceeded"
	);
	let mut edits: Vec<_> = candidate.edits.iter().collect();
	edits.sort_by_key(|e| e.start);
	let mut output = String::new();
	let mut cursor = 0;
	for edit in edits {
		heartbeat()?;
		ensure!(
			edit.start >= cursor
				&& edit.start <= edit.end
				&& code.get(edit.start..edit.end) == Some(edit.old.as_str()),
			"mutation ranges are stale or overlapping"
		);
		ensure!(
			edit.new.len() <= 256 * 1024
				&& output.len() + edit.start - cursor + edit.new.len() <= 256 * 1024,
			"mutated source exceeds limit"
		);
		output.push_str(&code[cursor..edit.start]);
		output.push_str(&edit.new);
		cursor = edit.end;
	}
	output.push_str(&code[cursor..]);
	ensure!(
		output.len() <= 256 * 1024 && output != code,
		"mutation output is oversized or unchanged"
	);
	heartbeat()?;
	let tree =
		parse_cached(&output, lang)?.ok_or_else(|| anyhow::anyhow!("mutated parse interrupted"))?;
	heartbeat()?;
	ensure!(!tree.root_node().has_error(), "mutation introduces invalid syntax");
	Ok(output)
}
