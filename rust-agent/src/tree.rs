//! Directory trees for `read` listings (`buildDirectoryTree` +
//! `assembleTree`): one workspace scan, children ranked newest-first with a
//! per-directory cap that keeps the oldest entry after a `… N more` marker,
//! and size/age columns.
use std::{
	cmp::Ordering,
	collections::HashMap,
	path::Path,
	time::{SystemTime, UNIX_EPOCH},
};

use pi_walker::{
	FileType,
	workspace::{WorkspaceQuery, list_workspace},
};

use crate::output::format_bytes;

/// Rendered tree.
pub(crate) struct DirectoryTree {
	pub rendered:    String,
	pub truncated:   bool,
	pub total_lines: usize,
}

struct Node {
	name:     String,
	is_dir:   bool,
	mtime_ms: f64,
	size:     f64,
	depth:    usize,
	children: Vec<Self>,
	dropped:  usize,
}

struct Line {
	label: String,
	depth: usize,
	root:  bool,
	size:  Option<String>,
	age:   Option<String>,
}

/// `formatAge`.
fn format_age(seconds: u64) -> String {
	if seconds == 0 {
		return String::new();
	}
	let mins = seconds / 60;
	let hours = mins / 60;
	let days = hours / 24;
	let (weeks, months) = (days / 7, days / 30);
	if months > 0 {
		format!("{months}mo ago")
	} else if weeks > 0 {
		format!("{weeks}w ago")
	} else if days > 0 {
		format!("{days}d ago")
	} else if hours > 0 {
		format!("{hours}h ago")
	} else if mins > 0 {
		format!("{mins}m ago")
	} else {
		"just now".to_owned()
	}
}

/// CLDR root-collation order of ASCII punctuation and symbols (non-ignorable).
const PUNCTUATION_ORDER: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// Primary weight of one character: whitespace, punctuation (CLDR order),
/// digits, then letters case-insensitively; anything else after, by code point.
fn primary_weight(ch: char) -> (u8, u32) {
	if ch.is_whitespace() {
		(0, ch as u32)
	} else if let Some(index) = PUNCTUATION_ORDER.find(ch) {
		(1, index as u32)
	} else if ch.is_ascii_digit() {
		(2, ch as u32)
	} else if ch.is_alphabetic() {
		(3, ch.to_lowercase().next().unwrap_or(ch) as u32)
	} else {
		(4, ch as u32)
	}
}

/// Approximates ICU `localeCompare` for file names: primary weights first,
/// then lowercase before uppercase at the first case difference.
fn locale_compare(a: &str, b: &str) -> Ordering {
	let primary = a
		.chars()
		.map(primary_weight)
		.cmp(b.chars().map(primary_weight));
	primary.then_with(|| {
		a.chars()
			.zip(b.chars())
			.find(|(x, y)| x != y)
			.map_or(Ordering::Equal, |(x, y)| x.is_uppercase().cmp(&y.is_uppercase()))
	})
}

/// Newest first, then by name (`localeCompare`).
fn by_recency(a: &Node, b: &Node) -> Ordering {
	b.mtime_ms
		.partial_cmp(&a.mtime_ms)
		.unwrap_or(Ordering::Equal)
		.then_with(|| locale_compare(&a.name, &b.name))
}

fn utf16_len(text: &str) -> usize {
	text.encode_utf16().count()
}

fn pad_end(text: &str, width: usize) -> String {
	let mut out = text.to_owned();
	out.extend(std::iter::repeat_n(' ', width.saturating_sub(utf16_len(text))));
	out
}

fn render(node: &Node, age: &dyn Fn(f64) -> String, out: &mut Vec<Line>) {
	if node.depth == 0 {
		out.push(Line { label: node.name.clone(), depth: 0, root: true, size: None, age: None });
	} else {
		out.push(Line {
			label: format!(
				"{}- {}{}",
				"  ".repeat(node.depth),
				node.name,
				if node.is_dir { "/" } else { "" }
			),
			depth: node.depth,
			root:  false,
			size:  (!node.is_dir).then(|| format_bytes(node.size as usize)),
			age:   Some(age(node.mtime_ms)),
		});
	}
	if node.dropped == 0 {
		for child in &node.children {
			render(child, age, out);
		}
		return;
	}
	let (oldest, recent) = node
		.children
		.split_last()
		.map_or((None, &node.children[..]), |(last, rest)| (Some(last), rest));
	for child in recent {
		render(child, age, out);
	}
	let depth = node.depth + 1;
	out.push(Line {
		label: format!("{}- … {} more", "  ".repeat(depth), node.dropped),
		depth,
		root: false,
		size: None,
		age: None,
	});
	if let Some(oldest) = oldest {
		render(oldest, age, out);
	}
}

/// `applyLineCap`: drop the deepest trailing entries first; root and its
/// children survive.
fn apply_line_cap(lines: Vec<Line>, cap: Option<usize>) -> (Vec<Line>, usize) {
	let Some(cap) = cap.filter(|&cap| lines.len() > cap) else {
		return (lines, 0);
	};
	let target = cap.saturating_sub(1).max(1);
	let mut removable: Vec<(usize, usize)> = lines
		.iter()
		.enumerate()
		.filter(|(_, line)| !line.root && line.depth > 1)
		.map(|(index, line)| (index, line.depth))
		.collect();
	removable.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
	removable.truncate(lines.len() - target);
	if removable.is_empty() {
		return (lines, 0);
	}
	let removed: std::collections::HashSet<usize> =
		removable.iter().map(|(index, _)| *index).collect();
	let mut kept: Vec<Line> = lines
		.into_iter()
		.enumerate()
		.filter(|(index, _)| !removed.contains(index))
		.map(|(_, line)| line)
		.collect();
	kept.push(Line {
		label: format!("[…{}ln elided…]", removed.len()),
		depth: 0,
		root:  false,
		size:  None,
		age:   None,
	});
	(kept, removed.len())
}

fn format_lines(lines: &[Line]) -> String {
	let width = lines
		.iter()
		.map(|line| utf16_len(&line.label))
		.max()
		.unwrap_or(0);
	lines
		.iter()
		.map(|line| match &line.age {
			None => line.label.clone(),
			Some(age) => {
				let size = pad_end(line.size.as_deref().unwrap_or(""), 8);
				format!("{}{size}  {}", pad_end(&line.label, width + 2), pad_end(age, 4))
					.trim_end()
					.to_owned()
			},
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// Scan `root` (hidden files shown, gitignore not consulted, standard
/// non-source directories pruned) and render the tree.
pub(crate) fn build(
	root: &Path,
	max_depth: usize,
	per_dir_limit: Option<usize>,
	root_limit: Option<usize>,
	line_cap: Option<usize>,
) -> DirectoryTree {
	let query = WorkspaceQuery {
		root: root.to_path_buf(),
		max_depth,
		include_hidden: true,
		gitignore: false,
		collect_agents_md: false,
	};
	let Ok(listing) = list_workspace(&query, || Ok::<(), std::convert::Infallible>(())) else {
		return DirectoryTree { rendered: String::new(), truncated: false, total_lines: 0 };
	};
	let mut by_parent: HashMap<String, Vec<Node>> = HashMap::new();
	for entry in listing.entries {
		let (parent, name) = entry
			.path
			.rsplit_once('/')
			.map_or(("", entry.path.as_str()), |(parent, name)| (parent, name));
		let node = Node {
			name:     name.to_owned(),
			is_dir:   entry.file_type == FileType::Dir,
			mtime_ms: entry.mtime.unwrap_or(0.0),
			size:     entry.size.unwrap_or(0.0),
			depth:    if parent.is_empty() {
				1
			} else {
				parent.split('/').count() + 1
			},
			children: Vec::new(),
			dropped:  0,
		};
		by_parent.entry(parent.to_owned()).or_default().push(node);
	}
	let mut truncated = listing.truncated;
	let mut root_node = Node {
		name:     ".".to_owned(),
		is_dir:   true,
		mtime_ms: 0.0,
		size:     0.0,
		depth:    0,
		children: Vec::new(),
		dropped:  0,
	};
	fn attach(
		node: &mut Node,
		path: &str,
		by_parent: &mut HashMap<String, Vec<Node>>,
		per_dir: Option<usize>,
		root_limit: Option<usize>,
		truncated: &mut bool,
	) {
		let mut all = by_parent.remove(path).unwrap_or_default();
		all.sort_by(by_recency);
		let limit = if node.depth == 0 { root_limit } else { per_dir };
		match limit {
			Some(limit) if all.len() > limit => {
				node.dropped = all.len() - limit;
				*truncated = true;
				if limit <= 1 {
					all.truncate(limit);
				} else {
					let oldest = all.pop();
					all.truncate(limit - 1);
					all.extend(oldest);
				}
			},
			_ => {},
		}
		node.children = all;
		for child in &mut node.children {
			if child.is_dir {
				let child_path = if path.is_empty() {
					child.name.clone()
				} else {
					format!("{path}/{}", child.name)
				};
				attach(child, &child_path, by_parent, per_dir, root_limit, truncated);
			}
		}
	}
	attach(&mut root_node, "", &mut by_parent, per_dir_limit, root_limit, &mut truncated);
	let now_ms = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0.0, |elapsed| elapsed.as_millis() as f64);
	let age =
		move |mtime_ms: f64| format_age(((now_ms - mtime_ms).max(0.0) / 1000.0).floor() as u64);
	let mut lines = Vec::new();
	render(&root_node, &age, &mut lines);
	let (lines, elided) = apply_line_cap(lines, line_cap);
	DirectoryTree {
		rendered:    format_lines(&lines),
		truncated:   truncated || elided > 0,
		total_lines: lines.len(),
	}
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	fn touch(root: &Path, path: &str, age_secs: u64) {
		let path = root.join(path);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(&path, "abc").unwrap();
		let time = SystemTime::now() - std::time::Duration::from_secs(age_secs);
		fs::File::options()
			.write(true)
			.open(&path)
			.unwrap()
			.set_modified(time)
			.unwrap();
	}

	#[test]
	fn children_rank_newest_first_and_caps_keep_the_oldest() {
		let root = tempfile::tempdir().unwrap();
		for (index, age) in [10u64, 400, 7200, 90_000, 3_000_000].iter().enumerate() {
			touch(root.path(), &format!("f{index}.txt"), *age);
		}
		let tree = build(root.path(), 2, Some(3), Some(3), None);
		let labels: Vec<&str> = tree
			.rendered
			.lines()
			.map(|line| line.split_whitespace().take(2).last().unwrap_or(""))
			.collect();
		assert_eq!(labels, [".", "f0.txt", "f1.txt", "…", "f4.txt"]);
		assert!(tree.rendered.contains("- … 2 more"), "{}", tree.rendered);
		assert!(tree.truncated);
		assert!(
			tree
				.rendered
				.lines()
				.nth(1)
				.unwrap()
				.ends_with("3B        just now"),
			"{}",
			tree.rendered
		);
		assert!(tree.rendered.lines().last().unwrap().ends_with("1mo ago"), "{}", tree.rendered);
	}

	#[test]
	fn name_ties_follow_icu_collation() {
		let mut names =
			vec!["tool-prompt-usage.ts", "Tool.md", "tool_io.py", "tool.md", "tool2.rs", "a b"];
		names.sort_by(|a, b| locale_compare(a, b));
		assert_eq!(names, [
			"a b",
			"tool_io.py",
			"tool-prompt-usage.ts",
			"tool.md",
			"Tool.md",
			"tool2.rs"
		]);
	}

	#[test]
	fn ages_follow_the_legacy_buckets() {
		assert_eq!(format_age(0), "");
		assert_eq!(format_age(59), "just now");
		assert_eq!(format_age(61), "1m ago");
		assert_eq!(format_age(86_400 * 8), "1w ago");
		assert_eq!(format_age(86_400 * 31), "1mo ago");
	}
}
