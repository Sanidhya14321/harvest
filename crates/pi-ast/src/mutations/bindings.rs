//! Conservative lexical binding analysis for identifier mutation inputs.
//! Unsupported binding forms fail closed instead of guessing reference
//! identity.
use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Default)]
struct Scope<'a> {
	parent:   Option<usize>,
	bindings: BTreeMap<&'a str, usize>,
}
struct Binding<'a> {
	name:        &'a str,
	declaration: Node<'a>,
	references:  Vec<Node<'a>>,
}
struct Resolver<'a> {
	code:       &'a str,
	scopes:     Vec<Scope<'a>>,
	bindings:   Vec<Binding<'a>>,
	references: Vec<(usize, Node<'a>)>,
}
impl<'a> Resolver<'a> {
	fn scope(&mut self, parent: usize) -> usize {
		let id = self.scopes.len();
		self
			.scopes
			.push(Scope { parent: Some(parent), bindings: BTreeMap::new() });
		id
	}

	fn declare(&mut self, scope: usize, node: Node<'a>) -> Result<()> {
		ensure!(node.kind() == "identifier", "identifier mutation requires simple binding names");
		let name = &self.code[node.byte_range()];
		ensure!(
			!self.scopes[scope].bindings.contains_key(name),
			"identifier mutation rejects duplicate bindings"
		);
		let id = self.bindings.len();
		self.scopes[scope].bindings.insert(name, id);
		self
			.bindings
			.push(Binding { name, declaration: node, references: Vec::new() });
		Ok(())
	}

	fn parameters(&mut self, scope: usize, parameters: Node<'a>) -> Result<()> {
		if parameters.kind() == "identifier" {
			return self.declare(scope, parameters);
		}
		ensure!(parameters.kind() == "formal_parameters", "unsupported function parameters");
		for parameter in children(parameters) {
			self.declare(scope, parameter)?;
		}
		Ok(())
	}
}
const KEYWORDS: &str = "break case catch class const continue debugger default delete do else \
                        enum export extends false finally for function if implements import in \
                        instanceof interface let new null package private protected public return \
                        static super switch this throw true try typeof var void while with yield \
                        await arguments eval";
fn misspelled(name: &str) -> Option<String> {
	// ASCII-only identifiers keep byte positions and the legacy permutation
	// unambiguous. Unicode identifiers remain ordinary unchanged source data.
	if !name.is_ascii()
		|| name.len() < 2
		|| name.starts_with('_')
		|| KEYWORDS.split_whitespace().any(|k| k == name)
	{
		return None;
	}
	let mut bytes = name.as_bytes().to_vec();
	let last = bytes.len() - 1;
	if bytes.len() >= 4 {
		bytes.swap(0, last);
	} else {
		bytes.swap(0, 1);
	}
	let renamed = String::from_utf8(bytes).ok()?;
	if renamed == name
		|| renamed.as_bytes()[0].is_ascii_digit()
		|| KEYWORDS.split_whitespace().any(|k| k == renamed)
	{
		None
	} else {
		Some(renamed)
	}
}

pub(super) fn identifier_candidates(
	code: &str,
	lang: SupportLang,
	root: Node<'_>,
	heartbeat: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<Candidate>> {
	ensure!(
		lang == SupportLang::JavaScript,
		"identifier binding mutations currently support JavaScript only"
	);
	let mut resolver = Resolver {
		code,
		scopes: vec![Scope::default()],
		bindings: Vec::new(),
		references: Vec::new(),
	};
	// node, lexical scope, function/var scope, depth. All declarations are
	// collected before reference resolution so hoisting and shadowing use the
	// binding's identity rather than textual occurrence order.
	let mut pending = vec![(root, 0, 0, 0)];
	let mut visited = 0;
	while let Some((node, mut scope, function, depth)) = pending.pop() {
		heartbeat()?;
		visited += 1;
		ensure!(visited <= 100_000 && depth <= 256, "identifier binding work budget exceeded");
		ensure!(
			!node.kind().starts_with("jsx_"),
			"identifier mutation does not yet resolve JSX names"
		);
		match node.kind() {
			"import_statement"
			| "export_statement"
			| "class"
			| "class_declaration"
			| "method_definition"
			| "catch_clause"
			| "with_statement"
			| "object_pattern"
			| "array_pattern"
			| "assignment_pattern"
			| "rest_pattern"
			| "shorthand_property_identifier"
			| "shorthand_property_identifier_pattern" => {
				anyhow::bail!("unsupported identifier binding construct: {}", node.kind())
			},
			"call_expression"
				if node
					.child_by_field_name("function")
					.is_some_and(|n| n.kind() == "identifier" && &code[n.byte_range()] == "eval") =>
			{
				anyhow::bail!("identifier mutation rejects dynamic eval scope")
			},
			"function_declaration"
			| "generator_function_declaration"
			| "function_expression"
			| "generator_function"
			| "arrow_function" => {
				let inner = resolver.scope(scope);
				if let Some(name) = node.child_by_field_name("name") {
					let declaration =
						matches!(node.kind(), "function_declaration" | "generator_function_declaration");
					// Sloppy block-level function declarations have Annex B
					// bindings. Keep those inputs out of this conservative subset.
					ensure!(
						!declaration || scope == function,
						"block function bindings require Annex B analysis"
					);
					resolver.declare(if declaration { scope } else { inner }, name)?;
				}
				if let Some(parameters) = node
					.child_by_field_name("parameters")
					.or_else(|| node.child_by_field_name("parameter"))
				{
					resolver.parameters(inner, parameters)?;
				}
				let body = node
					.child_by_field_name("body")
					.ok_or_else(|| anyhow::anyhow!("function has no body"))?;
				// A function body shares the parameter/function scope. Nested
				// statement blocks below it create distinct lexical scopes.
				if body.kind() == "statement_block" {
					pending.extend(
						children(body)
							.into_iter()
							.rev()
							.map(|n| (n, inner, inner, depth + 1)),
					);
				} else {
					pending.push((body, inner, inner, depth + 1));
				}
				continue;
			},
			"statement_block" | "for_statement" | "for_in_statement" | "switch_body" => {
				scope = resolver.scope(scope);
			},
			"variable_declarator" => {
				let name = node
					.child_by_field_name("name")
					.ok_or_else(|| anyhow::anyhow!("variable has no binding"))?;
				let var = node
					.parent()
					.is_some_and(|p| p.kind() == "variable_declaration");
				resolver.declare(if var { function } else { scope }, name)?;
				if let Some(value) = node.child_by_field_name("value") {
					pending.push((value, scope, function, depth + 1));
				}
				continue;
			},
			"identifier" => {
				// Plain assignment targets are writes, not Babel-style read
				// references. Read-modify-write expressions remain reads.
				let assignment_target = node.parent().is_some_and(|p| {
					p.kind() == "assignment_expression" && p.child_by_field_name("left") == Some(node)
				});
				if !assignment_target {
					resolver.references.push((scope, node));
				}
			},
			_ => {},
		}
		// for-in/of declarations use dedicated fields rather than declarators;
		// reject them until that binding form has a reviewed implementation.
		ensure!(node.kind() != "for_in_statement", "for-in/of bindings are unsupported");
		pending.extend(
			children(node)
				.into_iter()
				.rev()
				.map(|n| (n, scope, function, depth + 1)),
		);
	}
	for (mut scope, reference) in resolver.references {
		heartbeat()?;
		let name = &code[reference.byte_range()];
		loop {
			if let Some(&id) = resolver.scopes[scope].bindings.get(name) {
				resolver.bindings[id].references.push(reference);
				break;
			}
			match resolver.scopes[scope].parent {
				Some(parent) => scope = parent,
				None => break,
			}
		}
	}
	let all_names: BTreeSet<_> = resolver.bindings.iter().map(|b| b.name).collect();
	let mut eligible = Vec::new();
	for binding in &resolver.bindings {
		let Some(renamed) = misspelled(binding.name).filter(|s| !all_names.contains(s.as_str()))
		else {
			continue;
		};
		let mut lines = BTreeMap::new();
		for &reference in &binding.references {
			lines
				.entry(reference.start_position().row)
				.and_modify(|old: &mut Node<'_>| {
					if reference.start_byte() < old.start_byte() {
						*old = reference;
					}
				})
				.or_insert(reference);
		}
		if lines.len() >= 2 {
			eligible.push((binding, renamed, lines.into_values().collect::<Vec<_>>()));
		}
	}
	let prefer_three = eligible.iter().any(|(_, _, refs)| refs.len() >= 3);
	let mut result = Vec::new();
	let mut bytes = 0;
	for (binding, renamed, references) in eligible {
		if prefer_three && references.len() < 3 {
			continue;
		}
		for start in 0..references.len() - 1 {
			for count in 2..=4.min(references.len() - start) {
				heartbeat()?;
				let selected = &references[start..start + count];
				let mut edits: Vec<_> = selected
					.iter()
					.map(|&n| edit(code, n, renamed.clone()))
					.collect();
				if selected
					.iter()
					.any(|n| n.start_position().row == binding.declaration.start_position().row)
				{
					edits.push(edit(code, binding.declaration, renamed.clone()));
				}
				edits.sort_by_key(|e| e.start);
				bytes += edits
					.iter()
					.map(|e| e.old.len() + e.new.len())
					.sum::<usize>();
				ensure!(
					result.len() < 4096 && bytes <= 8 * 1024 * 1024,
					"identifier mutation candidate budget exceeded"
				);
				result.push(Candidate {
					kind: MutationKind::IdentifierMultiEdit,
					line_number: selected[0].start_position().row + 1,
					edits,
				});
			}
		}
	}
	result.sort_by_key(|c| c.edits[0].start);
	Ok(result)
}
