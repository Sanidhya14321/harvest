//! Structural facts for cleanup discovery. Uses the shared TypeScript tree;
//! heuristics and reference indexing belong to the host, never deletion policy.
use anyhow::{Result, ensure};
use serde::Serialize;
use tree_sitter::Node;

use crate::{SupportLang, parse_cache::parse_cached};

#[derive(Debug, Serialize)]
pub struct Class {
	pub name:    String,
	pub methods: usize,
}
#[derive(Debug)]
pub struct Reexport {
	pub name: String,
	pub from: String,
}
#[derive(Debug, Default)]
pub struct Structure {
	pub exported_names:       Vec<String>,
	pub export_star:          Vec<String>,
	pub named_reexports:      Vec<Reexport>,
	pub classes:              Vec<Class>,
	pub top_level_statements: usize,
	pub deep_imports:         usize,
	pub recovered:            bool,
}
fn text<'a>(node: Node<'_>, code: &'a str) -> &'a str {
	&code[node.byte_range()]
}
fn hex(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, count: usize) -> Result<u32> {
	let mut value = 0;
	for _ in 0..count {
		value = value * 16
			+ chars
				.next()
				.and_then(|c| c.to_digit(16))
				.ok_or_else(|| anyhow::anyhow!("invalid source escape"))?;
	}
	Ok(value)
}
fn decoded(raw: &str) -> Result<String> {
	ensure!(raw.len() <= 16 * 1024, "cleanup declaration/specifier exceeds 16 KiB");
	let mut output = Vec::new();
	let mut chars = raw.chars().peekable();
	while let Some(character) = chars.next() {
		if character != '\\' {
			output.extend(character.encode_utf16(&mut [0; 2]).iter().copied());
			continue;
		}
		let escaped = chars
			.next()
			.ok_or_else(|| anyhow::anyhow!("incomplete source escape"))?;
		let scalar = match escaped {
			'\n' => continue,
			'\r' => {
				if chars.peek() == Some(&'\n') {
					chars.next();
				}
				continue;
			},
			'u' if chars.peek() == Some(&'{') => {
				chars.next();
				let mut value = 0_u32;
				let mut count = 0;
				loop {
					let c = chars
						.next()
						.ok_or_else(|| anyhow::anyhow!("incomplete Unicode escape"))?;
					if c == '}' {
						break;
					}
					count += 1;
					ensure!(count <= 6, "Unicode escape exceeds six digits");
					value = value * 16
						+ c.to_digit(16)
							.ok_or_else(|| anyhow::anyhow!("invalid Unicode escape"))?;
				}
				ensure!(count > 0, "empty Unicode escape");
				let c =
					char::from_u32(value).ok_or_else(|| anyhow::anyhow!("invalid Unicode scalar"))?;
				output.extend(c.encode_utf16(&mut [0; 2]).iter().copied());
				continue;
			},
			'u' => hex(&mut chars, 4)?,
			'x' => hex(&mut chars, 2)?,
			'n' => 10,
			'r' => 13,
			't' => 9,
			'v' => 11,
			'b' => 8,
			'f' => 12,
			'0'..='7' => {
				let mut number = escaped.to_digit(8).expect("octal digit");
				for _ in 0..if escaped <= '3' { 2 } else { 1 } {
					let Some(next) = chars.peek().and_then(|c| c.to_digit(8)) else {
						break;
					};
					chars.next();
					number = number * 8 + next;
				}
				number
			},
			other => {
				output.extend(other.encode_utf16(&mut [0; 2]).iter().copied());
				continue;
			},
		};
		output.push(u16::try_from(scalar)?);
	}
	Ok(String::from_utf16(&output)?)
}
fn name(node: Node<'_>, code: &str) -> Result<String> {
	let raw = text(node, code);
	if node.kind() == "string" {
		ensure!(raw.len() >= 2, "invalid string node");
		decoded(&raw[1..raw.len() - 1])
	} else {
		decoded(raw)
	}
}
fn declaration(node: Node<'_>) -> Node<'_> {
	if node.kind() == "ambient_declaration" {
		let mut cursor = node.walk();
		if let Some(inner) = node
			.named_children(&mut cursor)
			.find(|child| child.kind() != "comment")
		{
			return inner;
		}
	}
	node
}
fn declared(node: Node<'_>, exported: bool, code: &str, result: &mut Structure) -> Result<()> {
	let node = declaration(node);
	match node.kind() {
		"class_declaration" | "abstract_class_declaration" | "class" => {
			let class_name = node
				.child_by_field_name("name")
				.map(|node| name(node, code))
				.transpose()?;
			let mut methods = 0;
			if let Some(body) = node.child_by_field_name("body") {
				let mut cursor = body.walk();
				for member in body.named_children(&mut cursor) {
					if matches!(
						member.kind(),
						"method_definition" | "method_signature" | "abstract_method_signature"
					) {
						let constructor = member.child_by_field_name("name").is_some_and(|n| {
							n.kind() == "property_identifier" && text(n, code) == "constructor"
						});
						let mut member_cursor = member.walk();
						let accessor = member
							.children(&mut member_cursor)
							.any(|child| matches!(child.kind(), "get" | "set"));
						if !constructor || accessor {
							methods += 1;
						}
					}
				}
			}
			result
				.classes
				.push(Class { name: class_name.clone().unwrap_or_else(|| "<anon>".into()), methods });
			if exported && let Some(name) = class_name {
				result.exported_names.push(name);
			}
		},
		"function_expression"
		| "function_declaration"
		| "generator_function_declaration"
		| "function_signature"
		| "interface_declaration"
		| "type_alias_declaration"
		| "enum_declaration"
		| "internal_module" => {
			if exported
				&& let Some(identifier) = node.child_by_field_name("name")
				&& identifier.kind() != "string"
			{
				let value = name(identifier, code)?;
				result
					.exported_names
					.push(if identifier.kind() == "nested_identifier" {
						value.split('.').next().unwrap_or(&value).into()
					} else {
						value
					});
			}
		},
		"lexical_declaration" | "variable_declaration" if exported => {
			let mut cursor = node.walk();
			for child in node.named_children(&mut cursor) {
				if child.kind() == "variable_declarator"
					&& let Some(identifier) = child.child_by_field_name("name")
					&& identifier.kind() == "identifier"
				{
					result.exported_names.push(name(identifier, code)?);
				}
			}
		},
		_ => {},
	}
	Ok(())
}
pub fn inspect(code: &str, cancellation: &mut dyn FnMut() -> Result<()>) -> Result<Structure> {
	ensure!(code.len() <= 4 * 1024 * 1024, "cleanup source exceeds 4 MiB");
	cancellation()?;
	let tree = parse_cached(code, SupportLang::TypeScript)?
		.ok_or_else(|| anyhow::anyhow!("cleanup source could not be parsed"))?;
	let root = tree.root_node();
	let mut result = Structure { recovered: root.has_error(), ..Default::default() };
	let mut cursor = root.walk();
	for (index, statement) in root.named_children(&mut cursor).enumerate() {
		ensure!(index <= 100_000, "cleanup source exceeds statement budget");
		cancellation()?;
		if matches!(statement.kind(), "comment" | "hash_bang_line") {
			continue;
		}
		result.top_level_statements += 1;
		if statement.kind() == "import_statement" {
			if let Some(source) = statement.child_by_field_name("source")
				&& name(source, code)?.starts_with("../../..")
			{
				result.deep_imports += 1;
			}
		} else if statement.kind() == "export_statement" {
			if let Some(inner) = statement
				.child_by_field_name("declaration")
				.or_else(|| statement.child_by_field_name("value"))
			{
				declared(inner, true, code, &mut result)?;
			}
			let source = statement
				.child_by_field_name("source")
				.map(|source| name(source, code))
				.transpose()?;
			let mut children_cursor = statement.walk();
			let children: Vec<_> = statement.named_children(&mut children_cursor).collect();
			let clause = children
				.iter()
				.find(|child| child.kind() == "export_clause");
			if let Some(clause) = clause {
				let mut specifier_cursor = clause.walk();
				for specifier in clause
					.named_children(&mut specifier_cursor)
					.filter(|child| child.kind() == "export_specifier")
				{
					let Some(identifier) = specifier
						.child_by_field_name("alias")
						.or_else(|| specifier.child_by_field_name("name"))
					else {
						continue;
					};
					let exported = name(identifier, code)?;
					match &source {
						Some(from) if from.starts_with('.') => result
							.named_reexports
							.push(Reexport { name: exported, from: from.clone() }),
						None => result.exported_names.push(exported),
						_ => {},
					}
				}
			} else if let Some(source) = source
				&& source.starts_with('.')
				&& !children
					.iter()
					.any(|child| child.kind() == "namespace_export")
			{
				result.export_star.push(source);
			}
		} else {
			declared(statement, false, code, &mut result)?;
		}
	}
	Ok(result)
}
