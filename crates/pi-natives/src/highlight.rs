//! Thin N-API syntax highlighting bridge over the shared Rust engine.
use harvest_tui::highlight as core;
use napi::{JsString, Result};
use napi_derive::napi;

use crate::{
	js::{self, InlineStr},
	task,
};

/// One theme colour: an ANSI escape sequence such as `\x1b[38;2;255;0;0m`.
///
/// Decoded inline, so a whole palette crosses the boundary without touching
/// the heap. The longest sequence a theme can produce sets attributes plus
/// truecolor foreground and background — `\x1b[1;3;4;38;2;255;255;255;48;2;
/// 255;255;255m`, 42 bytes — which the 47 usable bytes cover.
pub type Color = InlineStr<48>;

/// Theme colors for syntax highlighting.
/// Each color is an ANSI escape sequence (e.g., "\x1b[38;2;255;0;0m").
#[derive(Debug)]
#[napi(object)]
pub struct HighlightColors {
	/// ANSI color for comments.
	#[napi(ts_type = "string")]
	pub comment:     Color,
	/// ANSI color for keywords.
	#[napi(ts_type = "string")]
	pub keyword:     Color,
	/// ANSI color for function names.
	#[napi(ts_type = "string")]
	pub function:    Color,
	/// ANSI color for variables and identifiers.
	#[napi(ts_type = "string")]
	pub variable:    Color,
	/// ANSI color for string literals.
	#[napi(ts_type = "string")]
	pub string:      Color,
	/// ANSI color for numeric literals.
	#[napi(ts_type = "string")]
	pub number:      Color,
	/// ANSI color for type identifiers.
	#[napi(ts_type = "string")]
	pub r#type:      Color,
	/// ANSI color for operators.
	#[napi(ts_type = "string")]
	pub operator:    Color,
	/// ANSI color for punctuation tokens.
	#[napi(ts_type = "string")]
	pub punctuation: Color,
	/// ANSI color for diff inserted lines.
	#[napi(ts_type = "string")]
	pub inserted:    Option<Color>,
	/// ANSI color for diff deleted lines.
	#[napi(ts_type = "string")]
	pub deleted:     Option<Color>,
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
#[napi]
pub fn highlight_code(
	code: JsString,
	lang: Option<JsString>,
	colors: HighlightColors,
) -> Result<String> {
	let code = js::utf8(code)?;
	let lang = lang.map(js::utf8).transpose()?;
	Ok(core::highlight_code(&code, lang.as_deref(), &palette(&colors)))
}

fn palette(colors: &HighlightColors) -> core::Colors<'_> {
	core::Colors {
		comment:     &colors.comment,
		keyword:     &colors.keyword,
		function:    &colors.function,
		variable:    &colors.variable,
		string:      &colors.string,
		number:      &colors.number,
		r#type:      &colors.r#type,
		operator:    &colors.operator,
		punctuation: &colors.punctuation,
		inserted:    colors.inserted.as_deref(),
		deleted:     colors.deleted.as_deref(),
	}
}

/// Warm syntax grammars and scope matchers on the native worker pool.
#[napi]
pub fn warm_highlighter() -> task::Promise<()> {
	task::blocking("highlight.warm", (), move |_| {
		core::warm_highlighter();
		Ok(())
	})
}

/// Stateful incremental syntax highlighter for streamed code.
///
/// Carries syntect parser state across [`HighlightStream::push`] calls so
/// chunked highlighting of a growing buffer is byte-identical to highlighting
/// the concatenated text in one call. Feed newline-terminated complete lines;
/// only the final push may omit the trailing newline. An unresolved language
/// echoes input unchanged.
#[napi]
pub struct HighlightStream {
	state:  core::HighlightStream,
	colors: HighlightColors,
}
#[napi]
impl HighlightStream {
	/// Create a stream for `lang`; an unknown language yields a passthrough.
	#[napi(constructor)]
	pub fn new(lang: Option<JsString>, colors: HighlightColors) -> Result<Self> {
		let lang = lang.map(js::utf8).transpose()?;
		Ok(Self { state: core::HighlightStream::new(lang.as_deref()), colors })
	}

	/// Whether the language resolved to a grammar; `false` means passthrough.
	#[napi(getter)]
	pub const fn supported(&self) -> bool {
		self.state.supported()
	}

	/// Highlight the next chunk and advance parser state.
	#[napi]
	pub fn push(&mut self, chunk: JsString) -> Result<String> {
		let chunk = js::utf8(chunk)?;
		Ok(self.state.push(&chunk, &palette(&self.colors)))
	}
}

/// Check if a language is supported for highlighting.
/// Returns true if the language has either direct support or a fallback
/// mapping.
#[napi]
pub fn supports_language(lang: JsString) -> Result<bool> {
	Ok(core::supports_language(&js::utf8(lang)?))
}

/// Get list of supported languages.
#[napi]
pub fn get_supported_languages() -> Vec<String> {
	core::get_supported_languages()
}
