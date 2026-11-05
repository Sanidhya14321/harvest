//! Bounded native Markdown display using CommonMark parsing and the shared
//! terminal width/highlighting engines. Rendering never changes stored text.
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::{
	highlight::{Colors, highlight_code},
	sanitize,
	text::DEFAULT_TAB_WIDTH,
};

pub const MAX_MARKDOWN_BYTES: usize = 256 * 1024;
pub const MAX_MARKDOWN_ROWS: usize = 4096;
const MAX_CODE_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_WIDTH: usize = 4096;
const MAX_NESTING: usize = 64;
const MAX_TABLE_COLUMNS: usize = 128;
const MAX_TABLE_ROWS: usize = 256;
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";

fn bounded_prefix(value: &str, limit: usize) -> &str {
	let mut end = value.len().min(limit);
	while !value.is_char_boundary(end) {
		end -= 1;
	}
	&value[..end]
}

fn safe(value: &str) -> String {
	sanitize::shorten_path(&sanitize::replace_tabs(value))
}

#[derive(Default)]
struct Table {
	rows:      Vec<Vec<String>>,
	row:       Vec<String>,
	cell:      String,
	truncated: bool,
}
impl Table {
	fn cell(&mut self) {
		if self.row.len() < MAX_TABLE_COLUMNS {
			self.row.push(std::mem::take(&mut self.cell));
		} else {
			self.truncated = true;
		}
		self.cell.clear();
	}

	fn row(&mut self) {
		if self.rows.len() < MAX_TABLE_ROWS {
			self.rows.push(std::mem::take(&mut self.row));
		} else {
			self.truncated = true;
		}
		self.row.clear();
	}
}

struct Code {
	language: String,
	text:     String,
}
struct Display {
	width:     usize,
	rows:      Vec<String>,
	bytes:     usize,
	line:      String,
	quote:     usize,
	lists:     Vec<Option<u64>>,
	marker:    Option<String>,
	styles:    Vec<&'static str>,
	links:     Vec<String>,
	code:      Option<Code>,
	table:     Option<Table>,
	exhausted: bool,
}
impl Display {
	fn push(&mut self, row: String) {
		if self.rows.len() >= MAX_MARKDOWN_ROWS.saturating_sub(1)
			|| self.bytes.saturating_add(row.len()) > MAX_OUTPUT_BYTES.saturating_sub(128)
		{
			self.exhausted = true;
			return;
		}
		self.bytes += row.len();
		self.rows.push(row);
	}

	fn blank(&mut self) {
		self.flush();
		if self.rows.last().is_some_and(|row| !row.is_empty()) {
			self.push(String::new());
		}
	}

	fn emit(&mut self, value: &str) {
		let quote = "> ".repeat(self.quote.min(MAX_NESTING));
		let indent = "  ".repeat(self.lists.len().saturating_sub(1).min(MAX_NESTING));
		let marker = self.marker.take().unwrap_or_default();
		let prefix = format!("{quote}{indent}");
		let prefix_width = sanitize::visible_width(&prefix);
		let marker_width = sanitize::visible_width(&marker);
		let width = self
			.width
			.saturating_sub(prefix_width + marker_width)
			.max(1);
		let units: Vec<_> = value.encode_utf16().collect();
		let wrapped = crate::text::wrap_text_with_ansi(&units, width as u32, DEFAULT_TAB_WIDTH);
		for (index, row) in wrapped.into_iter().enumerate() {
			if self.exhausted {
				break;
			}
			let lead = if index == 0 {
				marker.clone()
			} else {
				" ".repeat(marker_width)
			};
			let line = format!("{prefix}{lead}{}{RESET}", String::from_utf16_lossy(&row));
			self.push(sanitize::truncate_to_width(&line, self.width));
		}
	}

	fn flush(&mut self) {
		if self.line.is_empty() {
			return;
		}
		let line = std::mem::take(&mut self.line);
		self.emit(&line);
	}

	fn style(&mut self, style: &'static str) {
		if self.styles.len() >= MAX_NESTING {
			self.exhausted = true;
			return;
		}
		self.styles.push(style);
		self.line.push_str(style);
	}

	fn unstyle(&mut self) {
		self.styles.pop();
		self.line.push_str(RESET);
		for style in &self.styles {
			self.line.push_str(style);
		}
	}

	fn finish_table(&mut self, table: Table) {
		let count = table.rows.iter().map(Vec::len).max().unwrap_or(0);
		if count == 0 {
			return;
		}
		let budget = self.width.saturating_sub(count.saturating_sub(1) * 3);
		let mut widths = vec![1; count];
		for row in &table.rows {
			for (index, cell) in row.iter().enumerate() {
				widths[index] = widths[index].max(sanitize::visible_width(cell).min(48));
			}
		}
		while widths.iter().sum::<usize>() > budget.max(count) {
			let Some((index, width)) = widths.iter().enumerate().max_by_key(|(_, width)| **width)
			else {
				break;
			};
			if *width <= 1 {
				break;
			}
			widths[index] -= 1;
		}
		for (index, row) in table.rows.iter().enumerate() {
			let cells: Vec<_> = (0..count)
				.map(|column| {
					let value = row.get(column).map_or("", String::as_str);
					let cell = sanitize::truncate_to_width(value, widths[column]);
					format!(
						"{cell}{}",
						" ".repeat(widths[column].saturating_sub(sanitize::visible_width(&cell)))
					)
				})
				.collect();
			let line = cells.join(" │ ");
			self.emit(&if index == 0 {
				format!("{BOLD}{line}{RESET}")
			} else {
				line
			});
			if index == 0 {
				self.emit(
					&widths
						.iter()
						.map(|width| "─".repeat(*width))
						.collect::<Vec<_>>()
						.join("─┼─"),
				);
			}
			if self.exhausted {
				break;
			}
		}
		if table.truncated {
			self.emit("[table display truncated]");
		}
		self.blank();
	}

	fn event(&mut self, event: Event<'_>) {
		if let Some(table) = self.table.as_mut() {
			match event {
				Event::End(TagEnd::Table) => {
					if let Some(table) = self.table.take() {
						self.finish_table(table);
					}
				},
				Event::End(TagEnd::TableCell) => table.cell(),
				Event::End(TagEnd::TableHead | TagEnd::TableRow) => table.row(),
				Event::Text(value)
				| Event::Code(value)
				| Event::Html(value)
				| Event::InlineHtml(value)
				| Event::InlineMath(value)
				| Event::DisplayMath(value) => table.cell.push_str(&safe(&value)),
				Event::SoftBreak | Event::HardBreak => table.cell.push(' '),
				Event::TaskListMarker(done) => table.cell.push_str(if done { "[x] " } else { "[ ] " }),
				_ => {},
			}
			return;
		}
		match event {
			Event::Start(Tag::CodeBlock(kind)) => {
				self.blank();
				let language = match kind {
					CodeBlockKind::Fenced(info) => info
						.split_whitespace()
						.next()
						.unwrap_or("")
						.chars()
						.take(64)
						.collect(),
					CodeBlockKind::Indented => String::new(),
				};
				self.code = Some(Code { language, text: String::new() });
			},
			Event::End(TagEnd::CodeBlock) => {
				if let Some(code) = self.code.take() {
					if !code.language.is_empty() {
						self.emit(&format!("{DIM}{}{RESET}", code.language));
					}
					let colors = Colors {
						keyword: "\x1b[35m",
						comment: DIM,
						string: "\x1b[32m",
						number: CYAN,
						inserted: Some("\x1b[32m"),
						deleted: Some("\x1b[31m"),
						..Default::default()
					};
					let highlighted = highlight_code(
						bounded_prefix(&code.text, MAX_CODE_BYTES),
						Some(&code.language),
						&colors,
					);
					for line in highlighted.lines() {
						self.emit(line);
						if self.exhausted {
							break;
						}
					}
					if code.text.len() > MAX_CODE_BYTES {
						self.emit("[code block display truncated]");
					}
				}
				self.blank();
			},
			Event::Text(value) => {
				if let Some(code) = &mut self.code {
					code.text.push_str(&safe(&value));
				} else {
					self.line.push_str(&safe(&value));
				}
			},
			Event::Start(Tag::Heading { level, .. }) => {
				self.blank();
				self
					.line
					.push_str(&format!("{BOLD}{CYAN}{} ", "#".repeat(level as usize)));
			},
			Event::End(TagEnd::Heading(_)) => {
				self.line.push_str(RESET);
				self.blank();
			},
			Event::Start(Tag::Paragraph) => {},
			Event::End(TagEnd::Paragraph | TagEnd::HtmlBlock) => self.blank(),
			Event::Start(Tag::BlockQuote(_)) => {
				self.flush();
				self.quote += 1;
				if self.quote > MAX_NESTING {
					self.exhausted = true;
				}
			},
			Event::End(TagEnd::BlockQuote(_)) => {
				self.flush();
				self.quote = self.quote.saturating_sub(1);
				self.blank();
			},
			Event::Start(Tag::List(start)) => {
				self.flush();
				if self.lists.len() >= MAX_NESTING {
					self.exhausted = true;
				} else {
					self.lists.push(start);
				}
			},
			Event::End(TagEnd::List(_)) => {
				self.flush();
				self.lists.pop();
				self.blank();
			},
			Event::Start(Tag::Item) => {
				self.flush();
				self.marker = Some(match self.lists.last_mut() {
					Some(Some(number)) => {
						let marker = format!("{number}. ");
						*number = number.saturating_add(1);
						marker
					},
					_ => "• ".into(),
				});
			},
			Event::End(TagEnd::Item) => {
				self.flush();
				self.marker = None;
			},
			Event::Start(Tag::Emphasis) => self.style("\x1b[3m"),
			Event::Start(Tag::Strong) => self.style(BOLD),
			Event::Start(Tag::Strikethrough) => self.style("\x1b[9m"),
			Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => self.unstyle(),
			Event::Start(Tag::Link { dest_url, .. }) => {
				if self.links.len() < MAX_NESTING {
					self.links.push(safe(&dest_url));
					self.style(CYAN);
				} else {
					self.exhausted = true;
				}
			},
			Event::End(TagEnd::Link) => {
				self.unstyle();
				if let Some(url) = self.links.pop() {
					if !url.is_empty() {
						self.line.push_str(&format!(" ({url})"));
					}
				}
			},
			Event::Start(Tag::Image { dest_url, .. }) => {
				self.line.push_str("[image: ");
				self.links.push(safe(&dest_url));
			},
			Event::End(TagEnd::Image) => {
				if let Some(url) = self.links.pop() {
					self.line.push_str(&format!(" ({url})]"));
				}
			},
			Event::Code(value) => {
				self.line.push_str("\x1b[33m");
				self.line.push_str(&safe(&value));
				self.line.push_str(RESET);
				for style in &self.styles {
					self.line.push_str(style);
				}
			},
			Event::SoftBreak => self.line.push(' '),
			Event::HardBreak => self.flush(),
			Event::Rule => {
				self.blank();
				self.emit(&"─".repeat(self.width));
				self.blank();
			},
			Event::TaskListMarker(done) => self.line.push_str(if done { "[x] " } else { "[ ] " }),
			Event::Html(value) | Event::InlineHtml(value) => self.line.push_str(&value),
			Event::FootnoteReference(value) => self.line.push_str(&format!("[{}]", safe(&value))),
			Event::Start(Tag::FootnoteDefinition(label)) => {
				self.blank();
				self.line.push_str(&format!("[{}]: ", safe(&label)));
			},
			Event::End(TagEnd::FootnoteDefinition) => self.blank(),
			Event::Start(Tag::Table(_)) => {
				self.blank();
				self.table = Some(Table::default());
			},
			Event::InlineMath(value) => {
				self.line.push('$');
				self.line.push_str(&safe(&value));
				self.line.push('$');
			},
			Event::DisplayMath(value) => {
				self.blank();
				self.emit(&format!("$${value}$$"));
				self.blank();
			},
			_ => {},
		}
	}
}

/// Render untrusted Markdown to bounded ANSI rows using shared Unicode widths.
/// Re-rendering an incomplete streaming prefix uses exactly this same pipeline.
#[must_use]
pub fn render(source: &str, width: usize) -> Vec<String> {
	let input = bounded_prefix(source, MAX_MARKDOWN_BYTES);
	let sanitized = sanitize::shorten_path(&sanitize::replace_tabs(input));
	let mut display = Display {
		width:     width.clamp(1, MAX_WIDTH),
		rows:      Vec::new(),
		bytes:     0,
		line:      String::new(),
		quote:     0,
		lists:     Vec::new(),
		marker:    None,
		styles:    Vec::new(),
		links:     Vec::new(),
		code:      None,
		table:     None,
		exhausted: false,
	};
	let options = Options::ENABLE_TABLES
		| Options::ENABLE_STRIKETHROUGH
		| Options::ENABLE_TASKLISTS
		| Options::ENABLE_FOOTNOTES
		| Options::ENABLE_MATH;
	for event in Parser::new_ext(&sanitized, options) {
		display.event(event);
		if display.line.len() > MAX_OUTPUT_BYTES {
			display.exhausted = true;
		}
		if display.exhausted {
			break;
		}
	}
	display.flush();
	while display.rows.last().is_some_and(String::is_empty) {
		display.rows.pop();
	}
	if display.exhausted || input.len() < source.len() {
		display
			.rows
			.push(sanitize::truncate_to_width("[Markdown display truncated]", display.width));
	}
	display.rows
}
