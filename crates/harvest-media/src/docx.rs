//! Bounded DOCX conversion, independent of N-API and application UI.
use std::{
	collections::{BTreeMap, BTreeSet},
	fmt::Write as _,
	io::{Cursor, Read},
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use harvest_runtime::cancel::CancelToken;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const INPUT_LIMIT: usize = 64 * 1024 * 1024;
const EXPANDED_LIMIT: usize = 256 * 1024 * 1024;
const PART_LIMIT: usize = 64 * 1024 * 1024;
const XML_LIMIT: usize = 16 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const NODE_LIMIT: u32 = 200_000;
const DEPTH_LIMIT: usize = 64;
const ENTRY_LIMIT: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
	pub style_map:                 Vec<String>,
	pub include_default_style_map: bool,
}
impl Default for Options {
	fn default() -> Self {
		Self { style_map: Vec::new(), include_default_style_map: true }
	}
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Diagnostic {
	pub r#type:  String,
	pub message: String,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Conversion {
	pub value:    String,
	pub messages: Vec<Diagnostic>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Image {
	pub content_type: String,
	pub alt_text:     String,
	pub base64:       String,
}
/// A temporary callback bridge can resolve image attributes then finish in
/// Rust.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
	pub html:     String,
	pub messages: Vec<Diagnostic>,
	pub images:   Vec<Image>,
}

fn escape(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
}
fn attr_escape(s: &str) -> String {
	escape(s).replace('"', "&quot;")
}
fn check_output(s: &str) -> Result<()> {
	ensure!(s.len() <= OUTPUT_LIMIT, "DOCX output exceeds 64 MiB");
	Ok(())
}
fn marker(index: usize) -> String {
	format!("<harvest-docx-image index=\"{index}\" />")
}
impl Prepared {
	pub fn finish(
		&self,
		attributes: Option<&[BTreeMap<String, String>]>,
		cancel: &CancelToken,
	) -> Result<Conversion> {
		check_output(&self.html)?;
		ensure!(self.images.len() <= 4096, "too many DOCX images");
		if let Some(attrs) = attributes {
			ensure!(attrs.len() == self.images.len(), "DOCX image callback count mismatch");
		}
		let mut value = self.html.clone();
		for (index, image) in self.images.iter().enumerate() {
			cancel.heartbeat()?;
			let default = BTreeMap::from([
				("alt".into(), image.alt_text.clone()),
				("src".into(), format!("data:{};base64,{}", image.content_type, image.base64)),
			]);
			let attrs = attributes.map_or(&default, |items| &items[index]);
			let mut rendered = String::from("<img");
			for (name, content) in attrs {
				ensure!(
					!name.is_empty()
						&& name
							.bytes()
							.all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b)),
					"invalid DOCX image attribute name"
				);
				write!(rendered, " {name}=\"{}\"", attr_escape(content))?;
				check_output(&rendered)?;
			}
			rendered.push_str(" />");
			let token = marker(index);
			ensure!(value.matches(&token).count() == 1, "invalid DOCX image placeholder");
			ensure!(
				value.len().saturating_add(rendered.len()) <= OUTPUT_LIMIT,
				"DOCX output exceeds 64 MiB"
			);
			value = value.replacen(&token, &rendered, 1);
		}
		cancel.heartbeat()?;
		Ok(Conversion { value, messages: self.messages.clone() })
	}
}
#[derive(Clone, Debug)]
struct Element {
	name:     String,
	attrs:    BTreeMap<String, String>,
	children: Vec<Node>,
}
#[derive(Clone, Debug)]
enum Node {
	Element(Element),
	Text(String),
}
impl Element {
	fn elements(&self) -> impl Iterator<Item = &Self> {
		self.children.iter().filter_map(|n| {
			if let Node::Element(e) = n {
				Some(e)
			} else {
				None
			}
		})
	}

	fn child(&self, name: &str) -> Option<&Self> {
		self.elements().find(|e| e.name == name)
	}

	fn attr(&self, key: &str) -> Option<&str> {
		self.attrs.get(key).map(String::as_str)
	}

	fn descendant(&self, name: &str) -> Option<&Self> {
		self.elements().find_map(|e| {
			if e.name == name {
				Some(e)
			} else {
				e.descendant(name)
			}
		})
	}
}
fn xml(source: &[u8], cancel: &CancelToken) -> Result<Element> {
	ensure!(source.len() <= XML_LIMIT, "DOCX XML exceeds 16 MiB");
	cancel.heartbeat()?;
	let text = std::str::from_utf8(source).context("DOCX XML is not UTF-8")?;
	let doc = roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions {
		nodes_limit: NODE_LIMIT,
		..Default::default()
	})
	.context("invalid DOCX XML")?;
	fn copy(node: roxmltree::Node<'_, '_>, depth: usize, cancel: &CancelToken) -> Result<Element> {
		cancel.heartbeat()?;
		ensure!(depth <= DEPTH_LIMIT, "DOCX XML depth exceeds 64");
		let mut children = Vec::new();
		for child in node.children() {
			if child.is_element() {
				children.push(Node::Element(copy(child, depth + 1, cancel)?));
			} else if child.is_text() {
				children.push(Node::Text(child.text().unwrap_or_default().into()));
			}
		}
		Ok(Element {
			name: node.tag_name().name().into(),
			attrs: node
				.attributes()
				.map(|a| (a.name().into(), a.value().into()))
				.collect(),
			children,
		})
	}
	copy(doc.root_element(), 0, cancel)
}
#[derive(Clone, Default)]
struct Formatting {
	bold:     Option<bool>,
	italic:   Option<bool>,
	strike:   Option<bool>,
	vertical: Option<String>,
}
impl Formatting {
	fn from(node: Option<&Element>) -> Self {
		let prop = |name: &str| node.and_then(|n| n.child(name));
		let enabled = |name: &str| {
			prop(name).map(|n| {
				!matches!(
					n.attr("val")
						.unwrap_or("true")
						.to_ascii_lowercase()
						.as_str(),
					"0" | "false" | "off" | "none"
				)
			})
		};
		Self {
			bold:     enabled("b"),
			italic:   enabled("i"),
			strike:   enabled("strike").or_else(|| enabled("dstrike")),
			vertical: prop("vertAlign")
				.and_then(|n| n.attr("val"))
				.filter(|s| matches!(*s, "subscript" | "superscript"))
				.map(str::to_owned),
		}
	}

	fn merge(self, other: Self) -> Self {
		Self {
			bold:     other.bold.or(self.bold),
			italic:   other.italic.or(self.italic),
			strike:   other.strike.or(self.strike),
			vertical: other.vertical.or(self.vertical),
		}
	}

	fn render(&self, mut value: String) -> String {
		for (enabled, tag) in [
			(self.strike == Some(true), "s"),
			(self.vertical.as_deref() == Some("subscript"), "sub"),
			(self.vertical.as_deref() == Some("superscript"), "sup"),
			(self.italic == Some(true), "em"),
			(self.bold == Some(true), "strong"),
		] {
			if enabled {
				value = format!("<{tag}>{value}</{tag}>");
			}
		}
		value
	}
}
#[derive(Clone)]
struct Numbering {
	id:    String,
	level: usize,
}
fn numbering(node: Option<&Element>) -> Option<Numbering> {
	let n = node?.child("numPr")?;
	let id = n.child("numId")?.attr("val")?;
	if id == "0" {
		return None;
	}
	Some(Numbering {
		id:    id.into(),
		level: n
			.child("ilvl")
			.and_then(|e| e.attr("val"))
			.and_then(|v| v.parse().ok())
			.unwrap_or(0),
	})
}
#[derive(Clone)]
struct Style {
	name:       String,
	based_on:   Option<String>,
	formatting: Formatting,
	numbering:  Option<Numbering>,
	default:    bool,
}
struct CustomStyle {
	kind: String,
	name: String,
	tag:  String,
}
struct Relationship {
	target:   String,
	external: bool,
}
struct Paragraph {
	html: String,
	tag:  String,
	list: Option<(usize, bool)>,
}
enum Block {
	Paragraph(Paragraph),
	Html(String),
}
struct Converter<'a> {
	entries:        BTreeMap<String, Vec<u8>>,
	relationships:  BTreeMap<String, Relationship>,
	types:          BTreeMap<String, String>,
	styles:         BTreeMap<String, Style>,
	numbering:      BTreeMap<String, BTreeMap<usize, bool>>,
	footnotes:      BTreeMap<String, Element>,
	used_notes:     Vec<String>,
	warned:         BTreeSet<String>,
	messages:       Vec<Diagnostic>,
	custom:         Vec<CustomStyle>,
	default_styles: bool,
	images:         Vec<Image>,
	image_bytes:    usize,
	cancel:         &'a CancelToken,
}
impl Converter<'_> {
	fn style_chain(&self, id: Option<&str>) -> Result<Vec<&Style>> {
		let mut result = Vec::new();
		let mut seen = BTreeSet::new();
		let mut id = id;
		while let Some(key) = id {
			self.cancel.heartbeat()?;
			ensure!(result.len() <= DEPTH_LIMIT, "DOCX style inheritance exceeds 64");
			if !seen.insert(key) {
				break;
			}
			let Some(style) = self.styles.get(key) else {
				break;
			};
			result.push(style);
			id = style.based_on.as_deref();
		}
		Ok(result)
	}

	fn style_format(&self, id: Option<&str>) -> Result<Formatting> {
		Ok(self
			.style_chain(id)?
			.into_iter()
			.rev()
			.fold(Formatting::default(), |f, s| f.merge(s.formatting.clone())))
	}

	fn style_numbering(&self, id: Option<&str>) -> Result<Option<Numbering>> {
		Ok(self
			.style_chain(id)?
			.into_iter()
			.find_map(|s| s.numbering.clone()))
	}

	fn custom_tag(&self, id: Option<&str>, kind: &str) -> Option<String> {
		let style = self.styles.get(id?)?;
		self
			.custom
			.iter()
			.find(|s| s.kind == kind && s.name == style.name)
			.map(|s| s.tag.clone())
	}

	fn warn(&mut self, id: &str, kind: &str) {
		if self.warned.insert(format!("{kind}:{id}")) {
			let name = self.styles.get(id).map_or(id, |s| s.name.as_str());
			self.messages.push(Diagnostic {
				r#type:  "warning".into(),
				message: format!("Unrecognised {kind} style: '{name}' (Style ID: {id})"),
			});
		}
	}

	fn paragraph_tag(&mut self, id: Option<&str>) -> String {
		let Some(id) = id else {
			return "p".into();
		};
		if let Some(tag) = self.custom_tag(Some(id), "p") {
			return tag;
		}
		let name = self.styles.get(id).map_or(id, |s| s.name.as_str());
		if self.default_styles {
			let name = name.to_ascii_lowercase();
			if let Some(level) = name
				.strip_prefix("heading")
				.map(str::trim)
				.filter(|s| matches!(*s, "1" | "2" | "3" | "4" | "5" | "6"))
			{
				return format!("h{level}");
			}
		}
		if name.to_lowercase() != "normal" && !self.styles.get(id).is_some_and(|s| s.default) {
			self.warn(id, "paragraph");
		}
		"p".into()
	}

	fn image(&mut self, element: &Element) -> Result<String> {
		self.cancel.heartbeat()?;
		let Some(rel) = element
			.descendant("blip")
			.and_then(|e| e.attr("embed"))
			.and_then(|id| self.relationships.get(id))
		else {
			return Ok(String::new());
		};
		if rel.external {
			return Ok(String::new());
		}
		let path = relationship_path(&rel.target)?;
		let Some(bytes) = self.entries.get(&path) else {
			self.messages.push(Diagnostic {
				r#type:  "warning".into(),
				message: format!("Could not find image {path}"),
			});
			return Ok(String::new());
		};
		self.image_bytes = self
			.image_bytes
			.checked_add(bytes.len().saturating_add(2) / 3 * 4)
			.context("DOCX image size overflow")?;
		ensure!(
			self.image_bytes <= OUTPUT_LIMIT && self.images.len() < 4096,
			"DOCX images exceed output limit"
		);
		let props = element.descendant("docPr");
		let alt_text = props
			.and_then(|e| e.attr("descr").or_else(|| e.attr("title")))
			.unwrap_or_default()
			.into();
		let ext = path
			.rsplit('.')
			.next()
			.unwrap_or_default()
			.to_ascii_lowercase();
		let fallback = match ext.as_str() {
			"bmp" => "image/bmp",
			"gif" => "image/gif",
			"jpg" | "jpeg" => "image/jpeg",
			"png" => "image/png",
			"svg" => "image/svg+xml",
			"tif" | "tiff" => "image/tiff",
			"webp" => "image/webp",
			_ => "application/octet-stream",
		};
		let content_type = self
			.types
			.get(&path)
			.or_else(|| self.types.get(&format!(".{ext}")))
			.map_or(fallback, String::as_str)
			.into();
		let index = self.images.len();
		self
			.images
			.push(Image { content_type, alt_text, base64: STANDARD.encode(bytes) });
		Ok(marker(index))
	}

	fn note(&mut self, e: &Element) -> String {
		let Some(id) = e.attr("id").filter(|id| self.footnotes.contains_key(*id)) else {
			return String::new();
		};
		let ordinal = if let Some(pos) = self.used_notes.iter().position(|n| n == id) {
			pos + 1
		} else {
			self.used_notes.push(id.into());
			self.used_notes.len()
		};
		let id = attr_escape(id);
		format!("<sup><a href=\"#footnote-{id}\" id=\"footnote-ref-{id}\">[{ordinal}]</a></sup>")
	}

	fn inline(&mut self, e: &Element, depth: usize) -> Result<String> {
		ensure!(depth <= DEPTH_LIMIT, "DOCX rendering depth exceeds 64");
		self.cancel.heartbeat()?;
		let mut html = String::new();
		for child in e.elements() {
			match child.name.as_str() {
				"r" => html.push_str(&self.run(child)?),
				"hyperlink" => {
					let contents = self.inline(child, depth + 1)?;
					let target = child
						.attr("id")
						.and_then(|id| self.relationships.get(id))
						.map(|r| r.target.clone())
						.or_else(|| child.attr("anchor").map(|a| format!("#{a}")));
					if let Some(target) = target {
						write!(html, "<a href=\"{}\">{contents}</a>", attr_escape(&target))?;
					} else {
						html.push_str(&contents);
					}
				},
				"del" => {},
				_ => html.push_str(&self.inline(child, depth + 1)?),
			}
			check_output(&html)?;
		}
		Ok(html)
	}

	fn run(&mut self, e: &Element) -> Result<String> {
		let props = e.child("rPr");
		let id = props
			.and_then(|p| p.child("rStyle"))
			.and_then(|p| p.attr("val"));
		let custom = self.custom_tag(id, "r");
		let mut formatting = self.style_format(id)?.merge(Formatting::from(props));
		if let Some(id) = id.filter(|_| custom.is_none()) {
			let name = self.styles.get(id).map_or(id, |s| s.name.as_str());
			if self.default_styles && name.eq_ignore_ascii_case("strong") {
				formatting.bold = Some(true);
			} else {
				self.warn(id, "run");
			}
		}
		let mut value = String::new();
		for child in e.elements() {
			self.cancel.heartbeat()?;
			match child.name.as_str() {
				"t" | "instrText" => {
					for node in &child.children {
						if let Node::Text(text) = node {
							value.push_str(&escape(text));
						}
					}
				},
				"tab" => value.push('\t'),
				"br" => {
					if child.attr("type") != Some("page") {
						value.push_str("<br />");
					}
				},
				"noBreakHyphen" => value.push('‑'),
				"softHyphen" => value.push('\u{ad}'),
				"drawing" | "pict" => value.push_str(&self.image(child)?),
				"footnoteReference" => value.push_str(&self.note(child)),
				_ => {},
			}
			check_output(&value)?;
		}
		let html = formatting.render(value);
		Ok(if let Some(tag) = custom {
			format!("<{tag}>{html}</{tag}>")
		} else {
			html
		})
	}

	fn blocks(&mut self, e: &Element, depth: usize) -> Result<Vec<Block>> {
		ensure!(depth <= DEPTH_LIMIT, "DOCX rendering depth exceeds 64");
		self.cancel.heartbeat()?;
		let mut blocks = Vec::new();
		for child in e.elements() {
			match child.name.as_str() {
				"p" => {
					let props = child.child("pPr");
					let id = props
						.and_then(|p| p.child("pStyle"))
						.and_then(|p| p.attr("val"));
					let tag = self.paragraph_tag(id);
					let numbering = match numbering(props) {
						Some(n) => Some(n),
						None => self.style_numbering(id)?,
					};
					let html = self.inline(child, 0)?;
					if !html.is_empty() || numbering.is_some() {
						let list = numbering.map(|n| {
							(
								n.level,
								self
									.numbering
									.get(&n.id)
									.and_then(|levels| levels.get(&n.level))
									.copied()
									.unwrap_or(true),
							)
						});
						blocks.push(Block::Paragraph(Paragraph { html, tag, list }));
					}
				},
				"tbl" => blocks.push(Block::Html(self.table(child, depth + 1)?)),
				"sdt" | "customXml" => blocks.extend(self.blocks(child, depth + 1)?),
				_ => {},
			}
		}
		Ok(blocks)
	}

	fn table(&mut self, e: &Element, depth: usize) -> Result<String> {
		struct Cell {
			html:    String,
			columns: usize,
			rows:    usize,
		}
		let mut cells = Vec::<Cell>::new();
		let mut rows = Vec::<Vec<usize>>::new();
		let mut active = BTreeMap::<usize, usize>::new();
		for row in e.elements().filter(|n| n.name == "tr") {
			self.cancel.heartbeat()?;
			let mut columns = Vec::new();
			let mut next = BTreeMap::new();
			let mut column = 0;
			let mut extended = BTreeSet::new();
			for cell in row.elements().filter(|n| n.name == "tc") {
				let props = cell.child("tcPr");
				let span = props
					.and_then(|p| p.child("gridSpan"))
					.and_then(|p| p.attr("val"))
					.and_then(|s| s.parse::<usize>().ok())
					.filter(|s| *s > 0)
					.unwrap_or(1);
				ensure!(span <= 4096 && column + span <= 4096, "DOCX table exceeds 4096 columns");
				let merge = props.and_then(|p| p.child("vMerge"));
				let blocks = self.blocks(cell, depth + 1)?;
				let html = render_blocks(&blocks)?;
				if merge.is_some_and(|m| m.attr("val") != Some("restart"))
					&& let Some(&origin) = active.get(&column)
				{
					if extended.insert(origin) {
						cells[origin].rows += 1;
					}
					for offset in 0..span {
						next.insert(column + offset, origin);
					}
					column += span;
					continue;
				}
				let index = cells.len();
				cells.push(Cell { html, columns: span, rows: 1 });
				columns.push(index);
				if merge.is_some_and(|m| m.attr("val") == Some("restart")) {
					for offset in 0..span {
						next.insert(column + offset, index);
					}
				}
				column += span;
			}
			rows.push(columns);
			active = next;
		}
		let mut html = String::from("<table>");
		for row in rows {
			html.push_str("<tr>");
			for index in row {
				let cell = &cells[index];
				html.push_str("<td");
				if cell.columns > 1 {
					write!(html, " colspan=\"{}\"", cell.columns)?;
				}
				if cell.rows > 1 {
					write!(html, " rowspan=\"{}\"", cell.rows)?;
				}
				html.push('>');
				html.push_str(&cell.html);
				html.push_str("</td>");
				check_output(&html)?;
			}
			html.push_str("</tr>");
		}
		html.push_str("</table>");
		Ok(html)
	}

	fn notes(&mut self) -> Result<String> {
		if self.used_notes.is_empty() {
			return Ok(String::new());
		}
		let mut html = String::from("<ol>");
		let mut index = 0;
		while index < self.used_notes.len() {
			self.cancel.heartbeat()?;
			let id = self.used_notes[index].clone();
			index += 1;
			let Some(note) = self.footnotes.get(&id).cloned() else {
				continue;
			};
			let blocks = self.blocks(&note, 0)?;
			let mut contents = render_blocks(&blocks)?;
			let backlink = format!("<a href=\"#footnote-ref-{}\">↑</a>", attr_escape(&id));
			if let Some(pos) = contents.rfind("</p>") {
				contents.insert_str(pos, &format!(" {backlink}"));
			} else {
				contents.push_str(&backlink);
			}
			write!(html, "<li id=\"footnote-{}\">{contents}</li>", attr_escape(&id))?;
			check_output(&html)?;
		}
		html.push_str("</ol>");
		Ok(html)
	}
}
fn relationship_path(target: &str) -> Result<String> {
	let mut parts = if target.starts_with('/') {
		Vec::new()
	} else {
		vec!["word"]
	};
	for part in target.split('/') {
		match part {
			"" | "." => {},
			".." => {
				ensure!(parts.pop().is_some(), "DOCX relationship escapes archive");
			},
			p => parts.push(p),
		}
	}
	Ok(parts.join("/"))
}
fn render_blocks(blocks: &[Block]) -> Result<String> {
	fn list(items: &[Block], index: &mut usize, level: usize, depth: usize) -> Result<String> {
		ensure!(depth <= DEPTH_LIMIT, "DOCX list depth exceeds 64");
		let Block::Paragraph(first) = &items[*index] else {
			unreachable!()
		};
		let ordered = first.list.context("missing list")?.1;
		let tag = if ordered { "ol" } else { "ul" };
		let mut html = format!("<{tag}>");
		while *index < items.len() {
			let Block::Paragraph(item) = &items[*index] else {
				break;
			};
			let Some((item_level, item_ordered)) = item.list else {
				break;
			};
			if item_level != level || item_ordered != ordered {
				break;
			}
			html.push_str("<li>");
			html.push_str(&item.html);
			*index += 1;
			while let Some(Block::Paragraph(next)) = items.get(*index) {
				let Some((next_level, _)) = next.list.filter(|(l, _)| *l > level) else {
					break;
				};
				html.push_str(&list(items, index, next_level, depth + 1)?);
				check_output(&html)?;
			}
			html.push_str("</li>");
			check_output(&html)?;
		}
		write!(html, "</{tag}>")?;
		Ok(html)
	}
	let mut html = String::new();
	let mut index = 0;
	while index < blocks.len() {
		match &blocks[index] {
			Block::Html(content) => {
				html.push_str(content);
				index += 1;
			},
			Block::Paragraph(p) => {
				if let Some((level, _)) = p.list {
					html.push_str(&list(blocks, &mut index, level, 0)?);
				} else {
					write!(html, "<{}>{}</{}>", p.tag, p.html, p.tag)?;
					index += 1;
				}
			},
		}
		check_output(&html)?;
	}
	Ok(html)
}
/// Decode only archive members; never extract to disk or resolve external URLs.
pub fn prepare(input: &[u8], options: &Options, cancel: &CancelToken) -> Result<Prepared> {
	cancel.heartbeat()?;
	ensure!(input.len() <= INPUT_LIMIT, "DOCX input exceeds 64 MiB");
	ensure!(
		options.style_map.iter().map(String::len).sum::<usize>() <= 64 * 1024,
		"DOCX style map exceeds 64 KiB"
	);
	let mut archive = zip::ZipArchive::new(Cursor::new(input)).context("invalid DOCX ZIP")?;
	ensure!(archive.len() <= ENTRY_LIMIT, "DOCX ZIP exceeds 4096 entries");
	let mut total = 0usize;
	let mut entries = BTreeMap::new();
	for i in 0..archive.len() {
		cancel.heartbeat()?;
		let mut file = archive.by_index(i)?;
		if file.is_dir() {
			continue;
		}
		let name = file.name()?.into_owned();
		ensure!(
			name.len() <= 4096
				&& !name.contains('\\')
				&& !name.starts_with('/')
				&& !name.split('/').any(|p| matches!(p, ".." | ".")),
			"invalid DOCX member path"
		);
		let size = usize::try_from(file.size()).context("DOCX member size overflow")?;
		ensure!(size <= PART_LIMIT, "DOCX member exceeds 64 MiB");
		total = total.checked_add(size).context("DOCX size overflow")?;
		ensure!(total <= EXPANDED_LIMIT, "DOCX expansion exceeds 256 MiB");
		let mut bytes = Vec::new();
		let mut chunk = vec![0; 64 * 1024];
		loop {
			cancel.heartbeat()?;
			let count = file.read(&mut chunk)?;
			if count == 0 {
				break;
			}
			ensure!(
				bytes.len().saturating_add(count) <= size,
				"DOCX ZIP member exceeds declared size"
			);
			bytes.extend_from_slice(&chunk[..count]);
		}
		ensure!(bytes.len() == size, "truncated DOCX ZIP member");
		ensure!(entries.insert(name, bytes).is_none(), "duplicate DOCX ZIP member");
	}
	let parse = |name: &str| -> Result<Option<Element>> {
		entries
			.get(name)
			.map(|bytes| xml(bytes, cancel))
			.transpose()
	};
	let document = parse("word/document.xml")?.context("Invalid DOCX: missing word/document.xml")?;
	let body = document
		.child("body")
		.context("Invalid DOCX: missing document body")?;
	let mut relationships = BTreeMap::new();
	if let Some(root) = parse("word/_rels/document.xml.rels")? {
		for e in root.elements() {
			if let (Some(id), Some(target)) = (e.attr("Id"), e.attr("Target")) {
				relationships.insert(id.into(), Relationship {
					target:   target.into(),
					external: e.attr("TargetMode") == Some("External"),
				});
			}
		}
	}
	let mut types = BTreeMap::new();
	if let Some(root) = parse("[Content_Types].xml")? {
		for e in root.elements() {
			if let Some(ct) = e.attr("ContentType") {
				if e.name == "Default" {
					if let Some(ext) = e.attr("Extension") {
						types.insert(format!(".{}", ext.to_ascii_lowercase()), ct.into());
					}
				} else if let Some(part) = e.attr("PartName") {
					types.insert(part.trim_start_matches('/').into(), ct.into());
				}
			}
		}
	}
	let mut styles = BTreeMap::new();
	if let Some(root) = parse("word/styles.xml")? {
		for e in root.elements().filter(|n| n.name == "style") {
			if let Some(id) = e
				.attr("styleId")
				.filter(|_| matches!(e.attr("type"), Some("paragraph" | "character")))
			{
				styles.insert(id.into(), Style {
					name:       e
						.child("name")
						.and_then(|n| n.attr("val"))
						.unwrap_or(id)
						.into(),
					based_on:   e
						.child("basedOn")
						.and_then(|n| n.attr("val"))
						.map(str::to_owned),
					formatting: Formatting::from(e.child("rPr")),
					numbering:  numbering(e.child("pPr")),
					default:    matches!(e.attr("default"), Some("1" | "true")),
				});
			}
		}
	}
	let mut numbers = BTreeMap::new();
	if let Some(root) = parse("word/numbering.xml")? {
		let mut abstracts = BTreeMap::new();
		for e in root.elements().filter(|n| n.name == "abstractNum") {
			if let Some(id) = e.attr("abstractNumId") {
				let mut levels = BTreeMap::new();
				for level in e.elements().filter(|n| n.name == "lvl") {
					let index = level.attr("ilvl").and_then(|s| s.parse().ok()).unwrap_or(0);
					levels.insert(
						index,
						level.child("numFmt").and_then(|n| n.attr("val")) != Some("bullet"),
					);
				}
				abstracts.insert(id.to_owned(), levels);
			}
		}
		for e in root.elements().filter(|n| n.name == "num") {
			if let (Some(id), Some(a)) =
				(e.attr("numId"), e.child("abstractNumId").and_then(|n| n.attr("val")))
			{
				numbers.insert(id.to_owned(), abstracts.get(a).cloned().unwrap_or_default());
			}
		}
	}
	let mut footnotes = BTreeMap::new();
	if let Some(root) = parse("word/footnotes.xml")? {
		for e in root.elements().filter(|n| n.name == "footnote") {
			if let Some(id) = e.attr("id").filter(|id| !id.starts_with('-')) {
				footnotes.insert(id.into(), e.clone());
			}
		}
	}
	let pattern = Regex::new(
		r#"(?i)^\s*(p|r)\s*\[style-name\s*=\s*['"]([^'"]+)['"]\]\s*=>\s*([A-Za-z0-9_-]+)"#,
	)?;
	let custom = options
		.style_map
		.iter()
		.flat_map(|s| s.lines())
		.filter_map(|line| pattern.captures(line))
		.map(|m| CustomStyle { kind: m[1].to_lowercase(), name: m[2].into(), tag: m[3].into() })
		.collect();
	let mut context = Converter {
		entries,
		relationships,
		types,
		styles,
		numbering: numbers,
		footnotes,
		used_notes: Vec::new(),
		warned: BTreeSet::new(),
		messages: Vec::new(),
		custom,
		default_styles: options.include_default_style_map,
		images: Vec::new(),
		image_bytes: 0,
		cancel,
	};
	let blocks = context.blocks(body, 0)?;
	let mut html = render_blocks(&blocks)?;
	html.push_str(&context.notes()?);
	check_output(&html)?;
	cancel.heartbeat()?;
	Ok(Prepared { html, messages: context.messages, images: context.images })
}
pub fn convert(input: &[u8], options: &Options, cancel: &CancelToken) -> Result<Conversion> {
	prepare(input, options, cancel)?.finish(None, cancel)
}
/// Native application text extraction deliberately omits embedded image
/// payloads.
pub fn markdown(input: &[u8], cancel: &CancelToken) -> Result<String> {
	let prepared = prepare(input, &Options::default(), cancel)?;
	let attrs: Vec<_> = prepared
		.images
		.iter()
		.map(|image| {
			BTreeMap::from([
				("alt".into(), image.alt_text.clone()),
				("src".into(), "docx-image-omitted".into()),
			])
		})
		.collect();
	let html = prepared.finish(Some(&attrs), cancel)?.value;
	let result =
		crate::html::html_to_markdown(&html, crate::html::HtmlToMarkdownOptions::default())?;
	cancel.heartbeat()?;
	Ok(result)
}
