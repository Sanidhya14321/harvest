//! Syntax highlighting using syntect.
//!
//! Provides ANSI-colored output for code blocks. Takes theme colors as input
//! and maps syntect scopes to 11 semantic categories:
//! - comment, keyword, function, variable, string, number, type, operator,
//!   punctuation, inserted, deleted
//!
//! Palettes are borrowed for each operation; the parser retains no host string
//! handles. Hosts own code/color bounds and provide validated display prefixes.
//! Streaming pushes must contain complete lines, except the final chunk.
//!
//! ```
//! use harvest_tui::highlight::{Colors, highlight_code};
//! let colors = Colors { keyword: "\x1b[35m", ..Colors::default() };
//! let rendered = highlight_code("let value = 1;\n", Some("rust"), &colors);
//! assert!(rendered.contains("\x1b[35mlet\x1b[39m"));
//! ```

use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use syntect::parsing::{
	ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxDefinition, SyntaxReference, SyntaxSet,
};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static SCOPE_MATCHERS: OnceLock<ScopeMatchers> = OnceLock::new();

// Thread-local cache for scope -> color index lookups
thread_local! {
	static SCOPE_COLOR_CACHE: RefCell<HashMap<Scope, usize>> = RefCell::new(HashMap::with_capacity(256));
}

/// Syntaxes bundled in addition to syntect's defaults: syntect ships none of
/// these, so we vendor their `.sublime-syntax` sources and fold them into the
/// set.
const EXTRA_SYNTAXES: &[&str] = &[
	include_str!("syntaxes/Julia.sublime-syntax"),
	include_str!("syntaxes/Nix.sublime-syntax"),
	include_str!("syntaxes/Mermaid.sublime-syntax"),
	include_str!("syntaxes/TypeScript.sublime-syntax"),
	include_str!("syntaxes/TypeScriptReact.sublime-syntax"),
];

fn get_syntax_set() -> &'static SyntaxSet {
	SYNTAX_SET.get_or_init(build_syntax_set)
}

/// Load syntect's newline-aware defaults and add the vendored extra syntaxes.
/// A vendored syntax that fails to parse is skipped rather than breaking all
/// highlighting; the bundled-language tests guard against silent absence.
fn build_syntax_set() -> SyntaxSet {
	let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
	for src in EXTRA_SYNTAXES {
		if let Ok(def) = SyntaxDefinition::load_from_str(src, true, None) {
			builder.add(def);
		}
	}
	builder.build()
}

/// Pre-compiled scope patterns for fast matching.
struct ScopeMatchers {
	// Comment (index 0)
	comment: Scope,

	// String (index 4)
	string:             Scope,
	constant_character: Scope,
	meta_string:        Scope,

	// Number (index 5)
	constant_numeric: Scope,
	constant_integer: Scope,
	constant:         Scope,

	// Keyword (index 1)
	keyword:          Scope,
	storage_type:     Scope,
	storage_modifier: Scope,

	// Function (index 2)
	entity_name_function: Scope,
	support_function:     Scope,
	meta_function_call:   Scope,
	variable_function:    Scope,

	// Type (index 6)
	entity_name_type:      Scope,
	support_type:          Scope,
	support_class:         Scope,
	entity_name_class:     Scope,
	entity_name_struct:    Scope,
	entity_name_enum:      Scope,
	entity_name_interface: Scope,
	entity_name_trait:     Scope,

	// Operator (index 7)
	keyword_operator:     Scope,
	punctuation_accessor: Scope,

	// Punctuation (index 8)
	punctuation: Scope,

	// Variable (index 3)
	variable:    Scope,
	entity_name: Scope,
	meta_path:   Scope,

	// Diff (indices 9, 10)
	markup_inserted:  Scope,
	markup_deleted:   Scope,
	meta_diff_header: Scope,
	meta_diff_range:  Scope,
}

impl ScopeMatchers {
	fn new() -> Self {
		Self {
			comment:               Scope::new("comment").unwrap(),
			string:                Scope::new("string").unwrap(),
			constant_character:    Scope::new("constant.character").unwrap(),
			meta_string:           Scope::new("meta.string").unwrap(),
			constant_numeric:      Scope::new("constant.numeric").unwrap(),
			constant_integer:      Scope::new("constant.integer").unwrap(),
			constant:              Scope::new("constant").unwrap(),
			keyword:               Scope::new("keyword").unwrap(),
			storage_type:          Scope::new("storage.type").unwrap(),
			storage_modifier:      Scope::new("storage.modifier").unwrap(),
			entity_name_function:  Scope::new("entity.name.function").unwrap(),
			support_function:      Scope::new("support.function").unwrap(),
			meta_function_call:    Scope::new("meta.function-call").unwrap(),
			variable_function:     Scope::new("variable.function").unwrap(),
			entity_name_type:      Scope::new("entity.name.type").unwrap(),
			support_type:          Scope::new("support.type").unwrap(),
			support_class:         Scope::new("support.class").unwrap(),
			entity_name_class:     Scope::new("entity.name.class").unwrap(),
			entity_name_struct:    Scope::new("entity.name.struct").unwrap(),
			entity_name_enum:      Scope::new("entity.name.enum").unwrap(),
			entity_name_interface: Scope::new("entity.name.interface").unwrap(),
			entity_name_trait:     Scope::new("entity.name.trait").unwrap(),
			keyword_operator:      Scope::new("keyword.operator").unwrap(),
			punctuation_accessor:  Scope::new("punctuation.accessor").unwrap(),
			punctuation:           Scope::new("punctuation").unwrap(),
			variable:              Scope::new("variable").unwrap(),
			entity_name:           Scope::new("entity.name").unwrap(),
			meta_path:             Scope::new("meta.path").unwrap(),
			markup_inserted:       Scope::new("markup.inserted").unwrap(),
			markup_deleted:        Scope::new("markup.deleted").unwrap(),
			meta_diff_header:      Scope::new("meta.diff.header").unwrap(),
			meta_diff_range:       Scope::new("meta.diff.range").unwrap(),
		}
	}
}

fn get_scope_matchers() -> &'static ScopeMatchers {
	SCOPE_MATCHERS.get_or_init(ScopeMatchers::new)
}

/// Theme colors for syntax highlighting.
/// Each color is an ANSI escape sequence (e.g., "\x1b[38;2;255;0;0m").
#[derive(Clone, Copy, Debug, Default)]
pub struct Colors<'a> {
	/// ANSI color for comments.
	pub comment:     &'a str,
	/// ANSI color for keywords.
	pub keyword:     &'a str,
	/// ANSI color for function names.
	pub function:    &'a str,
	/// ANSI color for variables and identifiers.
	pub variable:    &'a str,
	/// ANSI color for string literals.
	pub string:      &'a str,
	/// ANSI color for numeric literals.
	pub number:      &'a str,
	/// ANSI color for type identifiers.
	pub r#type:      &'a str,
	/// ANSI color for operators.
	pub operator:    &'a str,
	/// ANSI color for punctuation tokens.
	pub punctuation: &'a str,
	/// ANSI color for diff inserted lines.
	pub inserted:    Option<&'a str>,
	/// ANSI color for diff deleted lines.
	pub deleted:     Option<&'a str>,
}

/// Language alias mappings: (aliases, target syntax name).
/// Used for languages not in syntect's default set or with non-standard names.
const LANG_ALIASES: &[(&[&str], &str)] = &[
	(&["ts", "mts", "cts", "typescript"], "TypeScript"),
	(&["tsx"], "TypeScriptReact"),
	(&["js", "jsx", "javascript", "mjs", "cjs"], "JavaScript"),
	(&["py", "python"], "Python"),
	(&["rb", "ruby"], "Ruby"),
	(&["jl", "julia"], "Julia"),
	(&["nix"], "Nix"),
	(&["mermaid", "mmd"], "Mermaid"),
	(&["rs", "rust"], "Rust"),
	(&["go", "golang"], "Go"),
	(&["java"], "Java"),
	(&["kt", "kotlin"], "Java"),
	(&["swift"], "Objective-C"),
	(&["c", "h"], "C"),
	(&["cpp", "cc", "cxx", "c++", "hpp", "hxx", "hh"], "C++"),
	(&["cs", "csharp"], "C#"),
	(&["php"], "PHP"),
	(&["sh", "bash", "zsh", "shell"], "Bash"),
	(&["ps1", "powershell"], "PowerShell"),
	(&["html", "htm", "astro", "vue", "svelte"], "HTML"),
	(&["css"], "CSS"),
	(&["scss"], "SCSS"),
	(&["sass"], "Sass"),
	(&["less"], "LESS"),
	(&["json"], "JSON"),
	(&["yaml", "yml"], "YAML"),
	(&["toml"], "TOML"),
	(&["xml"], "XML"),
	(&["md", "markdown"], "Markdown"),
	(&["sql"], "SQL"),
	(&["lua"], "Lua"),
	(&["r"], "R"),
	(&["scala"], "Scala"),
	(&["clj", "clojure"], "Clojure"),
	(&["el", "elisp", "emacs-lisp", "emacslisp"], "Lisp"),
	(&["ex", "exs", "elixir"], "Ruby"),
	(&["erl", "erlang"], "Erlang"),
	(&["hs", "haskell"], "Haskell"),
	(&["ml", "ocaml"], "OCaml"),
	(&["vim"], "VimL"),
	(&["graphql", "gql"], "GraphQL"),
	(&["proto", "protobuf"], "Protocol Buffers"),
	(&["tf", "hcl", "terraform"], "Terraform"),
	(&["dockerfile", "docker", "containerfile"], "Dockerfile"),
	(&["makefile", "make", "just", "justfile"], "Makefile"),
	(&["cmake", "cmakelists"], "CMake"),
	(&["ini", "cfg", "conf", "config", "properties"], "INI"),
	(&["diff", "patch"], "Diff"),
	(&["gitignore", "gitattributes", "gitmodules"], "Git Ignore"),
];

/// Find syntax name from alias table using case-insensitive comparison.
#[inline]
fn find_alias(lang: &str) -> Option<&'static str> {
	LANG_ALIASES
		.iter()
		.find(|(aliases, _)| aliases.iter().any(|a| lang.eq_ignore_ascii_case(a)))
		.map(|(_, target)| *target)
}

/// Check if language is in the alias table.
#[inline]
fn is_known_alias(lang: &str) -> bool {
	LANG_ALIASES
		.iter()
		.any(|(aliases, _)| aliases.iter().any(|a| lang.eq_ignore_ascii_case(a)))
}

/// Compute the color index for a single scope (uncached).
#[inline]
fn compute_scope_color(s: Scope) -> usize {
	let m = get_scope_matchers();

	// Comment (index 0)
	if m.comment.is_prefix_of(s) {
		return 0;
	}

	// Diff inserted (index 9)
	if m.markup_inserted.is_prefix_of(s) {
		return 9;
	}

	// Diff deleted (index 10)
	if m.markup_deleted.is_prefix_of(s) {
		return 10;
	}

	// Diff header/range -> keyword (index 1)
	if m.meta_diff_header.is_prefix_of(s) || m.meta_diff_range.is_prefix_of(s) {
		return 1;
	}

	// String (index 4)
	if m.string.is_prefix_of(s)
		|| m.constant_character.is_prefix_of(s)
		|| m.meta_string.is_prefix_of(s)
	{
		return 4;
	}

	// Number (index 5)
	if m.constant_numeric.is_prefix_of(s) || m.constant_integer.is_prefix_of(s) {
		return 5;
	}

	// Keyword (index 1)
	if m.keyword.is_prefix_of(s)
		|| m.storage_type.is_prefix_of(s)
		|| m.storage_modifier.is_prefix_of(s)
	{
		return 1;
	}

	// Function (index 2)
	if m.entity_name_function.is_prefix_of(s)
		|| m.support_function.is_prefix_of(s)
		|| m.meta_function_call.is_prefix_of(s)
		|| m.variable_function.is_prefix_of(s)
	{
		return 2;
	}

	// Type (index 6)
	if m.entity_name_type.is_prefix_of(s)
		|| m.support_type.is_prefix_of(s)
		|| m.support_class.is_prefix_of(s)
		|| m.entity_name_class.is_prefix_of(s)
		|| m.entity_name_struct.is_prefix_of(s)
		|| m.entity_name_enum.is_prefix_of(s)
		|| m.entity_name_interface.is_prefix_of(s)
		|| m.entity_name_trait.is_prefix_of(s)
	{
		return 6;
	}

	// Operator (index 7)
	if m.keyword_operator.is_prefix_of(s) || m.punctuation_accessor.is_prefix_of(s) {
		return 7;
	}

	// Punctuation (index 8)
	if m.punctuation.is_prefix_of(s) {
		return 8;
	}

	// Variable (index 3)
	if m.variable.is_prefix_of(s) || m.entity_name.is_prefix_of(s) || m.meta_path.is_prefix_of(s) {
		return 3;
	}

	// Generic constant -> number (index 5)
	if m.constant.is_prefix_of(s) {
		return 5;
	}

	// No match
	usize::MAX
}

/// Determine the semantic color category from a scope stack.
/// Uses per-scope caching to avoid repeated prefix checks.
#[inline]
fn scope_to_color_index(scope: &ScopeStack) -> usize {
	SCOPE_COLOR_CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();

		// Walk from innermost to outermost scope
		for s in scope.as_slice().iter().rev() {
			let color_idx = *cache.entry(*s).or_insert_with(|| compute_scope_color(*s));
			if color_idx != usize::MAX {
				return color_idx;
			}
		}

		usize::MAX
	})
}

/// Find the appropriate syntax for a language name.
fn find_syntax<'a>(ss: &'a SyntaxSet, lang: &str) -> Option<&'a SyntaxReference> {
	// Direct name/token match (syntect APIs are case-insensitive)
	if let Some(syn) = ss.find_syntax_by_token(lang) {
		return Some(syn);
	}

	// Extension-based match
	if let Some(syn) = ss.find_syntax_by_extension(lang) {
		return Some(syn);
	}

	// Alias lookup for languages not in syntect's default set
	let alias = find_alias(lang)?;

	ss.find_syntax_by_name(alias)
		.or_else(|| ss.find_syntax_by_token(alias))
}

/// Color palette as array for quick indexing.
fn palette<'a>(colors: &Colors<'a>) -> [&'a str; 11] {
	[
		colors.comment,                // 0
		colors.keyword,                // 1
		colors.function,               // 2
		colors.variable,               // 3
		colors.string,                 // 4
		colors.number,                 // 5
		colors.r#type,                 // 6
		colors.operator,               // 7
		colors.punctuation,            // 8
		colors.inserted.unwrap_or(""), // 9
		colors.deleted.unwrap_or(""),  // 10
	]
}

/// Highlight code and return ANSI-colored lines.
///
/// # Arguments
/// * `code` - The source code to highlight
/// * `lang` - Language identifier (e.g., "rust", "typescript", "python")
/// * `colors` - Theme colors as ANSI escape sequences
///
/// # Returns
/// Highlighted code with ANSI color codes, or the original code if highlighting
/// fails.
pub fn highlight_code(code: &str, lang: Option<&str>, colors: &Colors<'_>) -> String {
	let Some(lang) = lang else {
		return code.to_owned();
	};
	let ss = get_syntax_set();
	let Some(syntax) = find_syntax(ss, lang) else {
		return code.to_owned();
	};

	let mut parse_state = ParseState::new(syntax);
	let mut scope_stack = ScopeStack::new();
	let mut result = String::with_capacity(code.len() * 2);
	highlight_into(code, ss, &mut parse_state, &mut scope_stack, &palette(colors), &mut result);
	result
}

/// Highlight `code` line by line, advancing `parse_state`/`scope_stack` and
/// appending ANSI-colored output to `result`. Because syntect parses strictly
/// forward, feeding a text in chunks of whole lines produces byte-identical
/// output to feeding it at once — the contract [`HighlightStream`] relies on.
fn highlight_into(
	code: &str,
	ss: &SyntaxSet,
	parse_state: &mut ParseState,
	scope_stack: &mut ScopeStack,
	palette: &[&str; 11],
	result: &mut String,
) {
	for line in syntect::util::LinesWithEndings::from(code) {
		let Ok(ops) = parse_state.parse_line(line, ss) else {
			// Parse error - append unhighlighted line and continue
			result.push_str(line);
			continue;
		};

		let mut prev_end = 0;
		for (offset, op) in ops {
			let offset = offset.min(line.len());

			// Output text BEFORE this operation using current scope
			if offset > prev_end {
				let text = &line[prev_end..offset];
				let color_idx = scope_to_color_index(scope_stack);

				if color_idx < palette.len() && !palette[color_idx].is_empty() {
					result.push_str(palette[color_idx]);
					result.push_str(text);
					result.push_str("\x1b[39m");
				} else {
					result.push_str(text);
				}
			}
			prev_end = offset;

			// Now apply scope operation for NEXT segment
			match op {
				ScopeStackOp::Push(scope) => {
					scope_stack.push(scope);
				},
				ScopeStackOp::Pop(count) => {
					for _ in 0..count {
						scope_stack.pop();
					}
				},
				ScopeStackOp::Restore | ScopeStackOp::Clear(_) | ScopeStackOp::Noop => {},
			}
		}

		// Output remaining text with current scope
		if prev_end < line.len() {
			let text = &line[prev_end..];
			let color_idx = scope_to_color_index(scope_stack);

			if color_idx < palette.len() && !palette[color_idx].is_empty() {
				result.push_str(palette[color_idx]);
				result.push_str(text);
				result.push_str("\x1b[39m");
			} else {
				result.push_str(text);
			}
		}
	}
}

/// Warm the immutable embedded grammars and scope matchers.
pub fn warm_highlighter() {
	let _ = get_syntax_set();
	let _ = get_scope_matchers();
}

/// Incremental highlighting over newline-complete chunks; the last may be
/// partial. Palettes are borrowed per push so hosts can apply current theme
/// colors.
pub struct HighlightStream {
	state: Option<(ParseState, ScopeStack)>,
}

impl HighlightStream {
	pub fn new(lang: Option<&str>) -> Self {
		let state = lang
			.and_then(|value| find_syntax(get_syntax_set(), value))
			.map(|syntax| (ParseState::new(syntax), ScopeStack::new()));
		Self { state }
	}

	pub const fn supported(&self) -> bool {
		self.state.is_some()
	}

	pub fn push(&mut self, chunk: &str, colors: &Colors<'_>) -> String {
		let Some((parse_state, scope_stack)) = self.state.as_mut() else {
			return chunk.to_owned();
		};
		let mut result = String::with_capacity(chunk.len() * 2);
		highlight_into(
			chunk,
			get_syntax_set(),
			parse_state,
			scope_stack,
			&palette(colors),
			&mut result,
		);
		result
	}
}

/// Check if a language is supported for highlighting.
/// Returns true if the language has either direct support or a fallback
/// mapping.
pub fn supports_language(lang: &str) -> bool {
	if is_known_alias(lang) {
		return true;
	}

	// Fall back to direct syntax lookup
	let ss = get_syntax_set();
	find_syntax(ss, lang).is_some()
}
/// Get list of supported languages.
pub fn get_supported_languages() -> Vec<String> {
	let ss = get_syntax_set();
	ss.syntaxes().iter().map(|s| s.name.clone()).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_colors() -> Colors<'static> {
		Colors {
			comment:     "<c>",
			keyword:     "<k>",
			function:    "<f>",
			variable:    "<v>",
			string:      "<s>",
			number:      "<n>",
			r#type:      "<t>",
			operator:    "<o>",
			punctuation: "<p>",
			inserted:    None,
			deleted:     None,
		}
	}

	#[test]
	fn stream_chunks_match_whole_text() {
		let colors = test_colors();
		let code = "def f():\n\t\"\"\"doc\n\tstring\"\"\"\n\treturn 1\n";
		let whole = highlight_code(code, Some("python"), &colors);

		let mut stream = HighlightStream::new(Some("python"));
		let mut chunked = String::new();
		for chunk in ["def f():\n", "\t\"\"\"doc\n\tstring\"\"\"\n", "\treturn 1\n"] {
			chunked.push_str(&stream.push(chunk, &colors));
		}
		assert_eq!(chunked, whole);
	}

	#[test]
	fn highlights_nix_vendored_syntax() {
		assert!(get_supported_languages().contains(&"Nix".to_string()));
		assert!(supports_language("nix"));

		let out = highlight_code(
			"{ pkgs ? import <nixpkgs> {} }:\nlet message = \"hello\"; in pkgs.writeText \"msg\" \
			 message # greeting\n",
			Some("nix"),
			&test_colors(),
		);
		assert!(out.contains("<k>let"));
		assert!(out.contains("<s>hello"));
		assert!(out.contains("<c># greeting"));
	}

	#[test]
	fn highlights_mermaid_vendored_syntax() {
		assert!(get_supported_languages().contains(&"Mermaid".to_string()));
		assert!(supports_language("mermaid"));
		assert!(supports_language("mmd"));

		let out = highlight_code(
			"graph TD\n  A[\"Start\"] --> B\n  %% note\n",
			Some("mermaid"),
			&test_colors(),
		);
		assert!(out.contains("<k>graph"));
		assert!(out.contains("<s>Start"));
		assert!(out.contains("<k>-->"));
		assert!(out.contains("<c> note"));
	}
	/// Regression: with the JavaScript grammar, TS type annotations
	/// (generic return types, arrow-type params) corrupted parser state, and a
	/// later template literal left an unterminated string scope that painted
	/// the rest of the file as a string (issue seen in the git TUI diff pane).
	#[test]
	fn typescript_template_literal_does_not_leak_string_state() {
		assert!(get_supported_languages().contains(&"TypeScript".to_string()));
		assert!(supports_language("ts"));
		assert!(supports_language("tsx"));

		let code = "function icons(): Record<\"close\" | \"hunk\", string> {\n\treturn \
		            {};\n}\nfunction chip(label: string): string {\n\tconst text = ` ${label} \
		            `;\n\treturn text;\n}\nconst zzz = 1;\n";
		let out = highlight_code(code, Some("ts"), &test_colors());
		let last = out.lines().last().unwrap();
		assert!(last.contains("<k>const"), "trailing code lost keyword highlighting: {last}");
		assert!(last.contains("<n>1"), "trailing code lost number highlighting: {last}");
		assert!(!last.contains("<s>const"), "string scope leaked past template literal: {last}");
	}
}
