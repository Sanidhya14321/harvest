//! Bounded lexical section retrieval and one-hop, scope-safe graph expansion.

use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
	time::Instant,
};

use anyhow::{Result, bail};
use serde::Serialize;

use crate::{
	BrainStore, Document, MAX_CONTEXT_CHARS, MAX_FILE_BYTES, MAX_FILES, MAX_INDEX_BYTES, MAX_PAGES,
	MAX_SECTIONS, RetrievalContext, Scope, parse_markdown,
};

#[derive(Debug, Clone)]
struct Section {
	document: usize,
	ordinal:  usize,
	heading:  String,
	content:  String,
	terms:    BTreeMap<String, usize>,
	length:   usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievedPage {
	pub section_index: usize,
	pub id:            String,
	pub scope:         Scope,
	pub path:          String,
	pub title:         String,
	pub heading:       String,
	pub content:       String,
	pub score:         f64,
}

#[derive(Debug, Default)]
pub struct BrainIndex {
	documents:    Vec<Document>,
	pages:        Vec<Section>,
	postings:     BTreeMap<String, Vec<usize>>,
	refreshed_at: Option<Instant>,
	pub warnings: Vec<String>,
}

impl BrainIndex {
	pub fn refresh(&mut self, stores: &[&BrainStore]) -> Result<()> {
		let _snapshots: Vec<_> = stores
			.iter()
			.map(|store| store.snapshot_lock())
			.collect::<Result<_>>()?;
		let mut documents = Vec::new();
		let mut bytes_total = 0_usize;
		let mut file_count = 0_usize;
		self.warnings.clear();
		for store in stores {
			if store.has_pending_recovery()? {
				bail!("brain has pending recovery; recover before indexing");
			}
			let state = store.state()?;
			for source in store.sources() {
				let mut paths = Vec::new();
				let mut visited = 0;
				scan(store, &source, &mut paths, &mut visited, 0, &mut self.warnings)?;
				for path in paths {
					if file_count >= MAX_FILES || bytes_total >= MAX_INDEX_BYTES {
						self.warnings.push(
							"brain index capacity reached; dependency inventory may be incomplete".into(),
						);
						break;
					}
					file_count += 1;
					let bytes = match store.fs.read(&path, MAX_FILE_BYTES) {
						Ok(bytes) => bytes,
						Err(error) => {
							self.warnings.push(format!("{}: {error}", path.display()));
							continue;
						},
					};
					if bytes_total.saturating_add(bytes.len()) > MAX_INDEX_BYTES {
						self.warnings.push(
							"brain aggregate content capacity reached; dependency inventory may be \
							 incomplete"
								.into(),
						);
						continue;
					}
					bytes_total += bytes.len();
					match std::str::from_utf8(&bytes)
						.map_err(anyhow::Error::from)
						.and_then(|text| {
							let id_path = path.strip_prefix(&source).unwrap_or(&path);
							let mut document =
								parse_markdown(store.scope, &crate::document_path(id_path), text)?;
							document.path = crate::document_path(&path);
							Ok(document)
						}) {
						Ok(mut document) => {
							if document.metadata.ownership == crate::Ownership::Generated {
								if crate::validate_content(&document.content, store.scope).is_err() {
									continue;
								}
								match state.records.get(&document.metadata.id) {
									Some(receipt)
										if receipt.file_hash == harvest_runtime::content_hash(&bytes) =>
									{
										document.metadata.status = receipt.status;
										if state.ledger.events.values().any(|event| {
											event.record_hash.as_deref() == Some(&receipt.fingerprint)
												&& document.metadata.conditions.is_subset(&event.conditions)
												&& matches!(
													event.kind,
													crate::EvidenceKind::RejectedApplication
														| crate::EvidenceKind::Supersession
												)
										}) {
											document.metadata.status = crate::RecordStatus::Quarantined;
										}
									},
									_ => {
										document.metadata.ownership = crate::Ownership::Authored;
										if !state.records.contains_key(&document.metadata.id) {
											document.metadata.status = crate::RecordStatus::Quarantined;
										}
									},
								}
							}
							documents.push(document);
						},
						Err(error) => self.warnings.push(format!("{}: {error}", path.display())),
					}
				}
			}
		}
		self.replace_documents(documents)?;
		self.refreshed_at = Some(Instant::now());
		Ok(())
	}

	/// Useful to SDK consumers supplying already-authorized Markdown sources.
	pub fn replace_documents(&mut self, documents: Vec<Document>) -> Result<()> {
		if documents.len() > MAX_FILES
			|| documents
				.iter()
				.map(|document| document.content.len())
				.sum::<usize>()
				> MAX_INDEX_BYTES
		{
			bail!("brain source capacity exceeded");
		}
		let mut ids = BTreeSet::new();
		let mut duplicates = BTreeSet::new();
		for document in &documents {
			if !ids.insert(document.scoped_id()) {
				duplicates.insert(document.scoped_id());
			}
		}
		self.documents = documents
			.into_iter()
			.filter(|document| !duplicates.contains(&document.scoped_id()))
			.collect();
		for id in duplicates {
			self
				.warnings
				.push(format!("duplicate scoped ID {id}; both definitions were skipped"));
		}
		self.pages.clear();
		self.postings.clear();
		// Allocate section capacity across every document. Early excluded documents
		// must not consume all pages and erase later eligible scopes.
		let section_limit = (MAX_PAGES / self.documents.len().max(1)).clamp(1, MAX_SECTIONS);
		for (document_index, document) in self.documents.iter().enumerate() {
			for (ordinal, (heading, content)) in
				sections(&document.content, &document.metadata.title, section_limit)
					.into_iter()
					.enumerate()
			{
				if self.pages.len() >= MAX_PAGES {
					break;
				}
				let mut tokens = tokenize(&document.metadata.title);
				if heading != document.metadata.title {
					tokens.extend(tokenize(&heading));
				}
				tokens.extend(tokenize(&content));
				let mut terms = BTreeMap::new();
				for token in &tokens {
					*terms.entry(token.clone()).or_insert(0) += 1;
				}
				self.pages.push(Section {
					ordinal,
					document: document_index,
					heading,
					content,
					terms,
					length: tokens.len(),
				});
			}
		}
		for (index, page) in self.pages.iter().enumerate() {
			for term in page.terms.keys() {
				self.postings.entry(term.clone()).or_default().push(index);
			}
		}
		Ok(())
	}

	pub fn documents(&self) -> &[Document] {
		&self.documents
	}

	/// Dependencies protect generated records even when their owners are
	/// authored or in another scope.
	pub fn required_dependencies(&self) -> BTreeSet<String> {
		self
			.documents
			.iter()
			.flat_map(|document| {
				document.metadata.depends_on.iter().map(move |target| {
					if target.starts_with("project:") || target.starts_with("user:") {
						target.clone()
					} else {
						format!("{}:{target}", document.scope.prefix())
					}
				})
			})
			.collect()
	}

	pub fn retrieve(&self, query: &str, context: &RetrievalContext) -> Vec<RetrievedPage> {
		let terms: BTreeSet<_> = tokenize(query).into_iter().collect();
		if terms.is_empty() {
			return Vec::new();
		}
		// Eligible corpus comes first: excluded pages must not displace matches.
		let eligible: Vec<_> = self
			.pages
			.iter()
			.enumerate()
			.filter(|(_, page)| context.allows(&self.documents[page.document]))
			.collect();
		let average = eligible.iter().map(|(_, page)| page.length).sum::<usize>() as f64
			/ eligible.len().max(1) as f64;
		let corpus_size = eligible.len() as f64;
		let eligible_ids: BTreeSet<_> = eligible.iter().map(|(index, _)| *index).collect();
		let mut frequencies = BTreeMap::new();
		let mut matching = BTreeSet::new();
		for term in &terms {
			let postings = self
				.postings
				.get(term)
				.map(Vec::as_slice)
				.unwrap_or_default();
			let selected: Vec<_> = postings
				.iter()
				.copied()
				.filter(|index| eligible_ids.contains(index))
				.collect();
			frequencies.insert(term, selected.len());
			matching.extend(selected);
		}
		let mut ranked = Vec::new();
		for index in matching {
			let page = &self.pages[index];
			let mut score = 0.0;
			for term in &terms {
				let frequency = *page.terms.get(term).unwrap_or(&0) as f64;
				if frequency == 0.0 {
					continue;
				}
				let matches = frequencies[term] as f64;
				let n = corpus_size;
				let idf = ((n - matches + 0.5) / (matches + 0.5)).ln_1p();
				let denominator =
					1.5_f64.mul_add(0.25 + 0.75 * page.length as f64 / average.max(1.0), frequency);
				score += idf * frequency * 2.5 / denominator;
			}
			if score > 0.0 && score.is_finite() {
				ranked.push((index, score));
			}
		}
		ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		ranked.truncate(4);
		let seeds: BTreeSet<_> = ranked
			.iter()
			.map(|(index, _)| self.pages[*index].document)
			.collect();
		let mut targets = BTreeSet::new();
		for index in seeds {
			let document = &self.documents[index];
			for target in document
				.metadata
				.depends_on
				.iter()
				.chain(&document.metadata.relates_to)
			{
				targets.insert(if target.starts_with("project:") || target.starts_with("user:") {
					target.clone()
				} else {
					format!("{}:{target}", document.scope.prefix())
				});
			}
		}
		for (index, page) in self.pages.iter().enumerate() {
			if ranked.len() >= 8 {
				break;
			}
			let document = &self.documents[page.document];
			if context.allows(document)
				&& targets.contains(&document.scoped_id())
				&& !ranked.iter().any(|(found, _)| *found == index)
			{
				ranked.push((index, 0.0));
			}
		}
		let budget = context
			.max_chars
			.unwrap_or(MAX_CONTEXT_CHARS)
			.min(MAX_CONTEXT_CHARS);
		let mut remaining = budget;
		let mut output = Vec::new();
		for (index, score) in ranked {
			let page = &self.pages[index];
			let document = &self.documents[page.document];
			let mut result = RetrievedPage {
				section_index: page.ordinal,
				id: document.scoped_id(),
				scope: document.scope,
				path: document.path.clone(),
				title: document.metadata.title.clone(),
				heading: page.heading.clone(),
				content: String::new(),
				score,
			};
			let overhead = render_page(&result).chars().count();
			if overhead >= remaining {
				break;
			}
			result.content = page.content.chars().take(remaining - overhead).collect();
			remaining = remaining.saturating_sub(render_page(&result).chars().count());
			output.push(result);
		}
		output
	}

	pub fn context_block(&self, query: &str, context: &RetrievalContext) -> String {
		let prefix = include_str!("prompts/reference.md").trim();
		let suffix = "\n</harvest-brain-reference>";
		let budget = context
			.max_chars
			.unwrap_or(MAX_CONTEXT_CHARS)
			.min(MAX_CONTEXT_CHARS);
		let overhead = prefix.chars().count() + suffix.chars().count() + 1;
		if budget <= overhead {
			return String::new();
		}
		let mut bounded = context.clone();
		bounded.max_chars = Some(budget - overhead);
		let pages = self.retrieve(query, &bounded);
		if pages.is_empty() {
			return String::new();
		}
		let mut output = format!("{prefix}\n");
		for page in pages {
			output.push_str(&render_page(&page));
		}
		output.push_str(suffix);
		output
	}
}

fn render_page(page: &RetrievedPage) -> String {
	format!(
		"\n[{} · {} · {}]\n{}\n",
		page.id,
		page.path.replace(char::is_control, " "),
		page.heading.replace(char::is_control, " "),
		page.content
	)
}

fn scan(
	store: &BrainStore,
	path: &Path,
	paths: &mut Vec<PathBuf>,
	visited: &mut usize,
	depth: usize,
	warnings: &mut Vec<String>,
) -> Result<()> {
	if depth > 16 || *visited >= 2048 || paths.len() >= MAX_FILES {
		warnings
			.push("brain traversal capacity reached; dependency inventory may be incomplete".into());
		return Ok(());
	}
	*visited += 1;
	if !store.fs.exists(path)? {
		return Ok(());
	}
	let entries = match store.fs.entries(path, 2048) {
		Ok(entries) => entries,
		Err(error) => {
			warnings.push(format!("{}: {error}", path.display()));
			return Ok(());
		},
	};
	for entry in entries {
		if entry.is_symlink || entry.name.starts_with('.') {
			continue;
		}
		let child = path.join(entry.name);
		if entry.is_directory {
			scan(store, &child, paths, visited, depth + 1, warnings)?;
		} else if entry.is_file
			&& child.extension().is_some_and(|extension| extension == "md")
			&& entry.size <= MAX_FILE_BYTES as u64
		{
			paths.push(child);
		}
		if paths.len() >= MAX_FILES {
			warnings
				.push("brain file capacity reached; dependency inventory may be incomplete".into());
			break;
		}
	}
	Ok(())
}

pub fn tokenize(text: &str) -> Vec<String> {
	text
		.split(|character: char| {
			!(character.is_alphanumeric() || character == '_' || character == '-')
		})
		.filter(|term| term.chars().count() >= 2)
		.map(str::to_lowercase)
		.collect()
}

fn sections(text: &str, title: &str, limit: usize) -> Vec<(String, String)> {
	let mut output = Vec::new();
	let mut heading = title.to_owned();
	let mut lines = Vec::new();
	let mut fence: Option<(char, usize)> = None;
	for line in text.lines() {
		let trimmed = line.trim_start();
		if let Some((character, length)) = fence {
			let count = trimmed
				.chars()
				.take_while(|found| *found == character)
				.count();
			if count >= length && trimmed[count..].trim().is_empty() {
				fence = None;
			}
		} else {
			let character = trimmed.chars().next().unwrap_or(' ');
			let count = trimmed
				.chars()
				.take_while(|found| *found == character)
				.count();
			if (character == '`' || character == '~') && count >= 3 {
				fence = Some((character, count));
			} else if let Some(next) = trimmed
				.strip_prefix("## ")
				.or_else(|| trimmed.strip_prefix("### "))
				.filter(|_| output.len() < limit - 1)
			{
				if !lines.is_empty() {
					output.push((heading.clone(), lines.join("\n")));
					lines.clear();
				}
				next.trim().clone_into(&mut heading);
			}
		}
		if output.len() >= limit - 1 {
			lines.push(line);
			continue;
		}
		lines.push(line);
	}
	if !lines.is_empty() {
		output.push((heading, lines.join("\n")));
	}
	output.truncate(limit);
	output
}
