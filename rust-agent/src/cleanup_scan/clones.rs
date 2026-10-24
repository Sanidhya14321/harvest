use std::{
	collections::{BTreeMap, BTreeSet},
	sync::LazyLock,
};

use anyhow::{Result, ensure};
use harvest_runtime::{cancel::CancelToken, content_hash};
use regex::Regex;

use super::{CloneCluster, FileInfo};
const WINDOW: usize = 7;
static STRINGS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
	[r#"`(?:[^`\\]|\\.)*`"#, r#""(?:[^"\\]|\\.)*""#, r"'(?:[^'\\]|\\.)*'"]
		.into_iter()
		.map(|pattern| Regex::new(pattern).expect("static literal normalization"))
		.collect()
});
static NUMBERS: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?-u:\b)[0-9][0-9._]*(?-u:\b)").expect("static number normalization")
});
static IMPORTS: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"^(import\b|export \{[^}]*\} from|export \* from)")
		.expect("static import normalization")
});
static COMMENT: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(&format!(r"(^|{})//.*$", crate::source_text::JS_SPACE))
		.expect("static comment normalization")
});
static SPACE: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(&format!("{}+", crate::source_text::JS_SPACE)).expect("static space normalization")
});
fn significant(text: &str, cancellation: &CancelToken) -> Result<Vec<(String, usize)>> {
	let mut result = Vec::new();
	let mut block = false;
	for (index, raw) in text.split('\n').enumerate() {
		cancellation.heartbeat()?;
		let mut line = raw;
		if block {
			let Some(end) = line.find("*/") else {
				continue;
			};
			line = &line[end + 2..];
			block = false;
		}
		if let Some(start) = line.find("/*")
			&& !line[start..].contains("*/")
		{
			line = &line[..start];
			block = true;
		}
		let without_comment = COMMENT.replace(line, "$1");
		let mut line = pi_edit::text::js_trim(&without_comment).to_owned();
		for pattern in STRINGS.iter() {
			line = pattern.replace_all(&line, "\"S\"").into_owned();
		}
		line = NUMBERS.replace_all(&line, "0").into_owned();
		line = SPACE.replace_all(&line, " ").into_owned();
		if line.encode_utf16().count() < 6 || IMPORTS.is_match(&line) {
			continue;
		}
		result.push((line, index + 1));
	}
	Ok(result)
}
fn root(parents: &mut [usize], value: usize) -> usize {
	let mut current = value;
	while parents[current] != current {
		current = parents[current];
	}
	let mut next = value;
	while parents[next] != next {
		let previous = parents[next];
		parents[next] = current;
		next = previous;
	}
	current
}
fn union(parents: &mut [usize], a: usize, b: usize) {
	let a = root(parents, a);
	let b = root(parents, b);
	if a != b {
		parents[a] = b;
	}
}
pub(super) fn detect(files: &[FileInfo], cancellation: &CancelToken) -> Result<Vec<CloneCluster>> {
	let mut significant_files = Vec::new();
	let mut positions = Vec::new();
	let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();

	for (file_index, file) in files.iter().enumerate() {
		let lines = significant(&file.text, cancellation)?;

		for window in 0..lines.len().saturating_sub(WINDOW - 1) {
			cancellation.heartbeat()?;
			ensure!(positions.len() < 1_000_000, "clone windows exceed one million");
			let joined = lines[window..window + WINDOW]
				.iter()
				.map(|(line, _)| line.as_str())
				.collect::<Vec<_>>()
				.join("\n");
			// Fingerprint is a bucket selector. Equality is verified below, so a
			// collision never turns unrelated source into a clone candidate.
			buckets
				.entry(content_hash(joined.as_bytes()))
				.or_default()
				.push(positions.len());
			positions.push((file_index, window));
		}
		significant_files.push(lines);
	}
	let mut parents: Vec<_> = (0..positions.len()).collect();
	let mut matched = BTreeSet::new();
	for bucket in buckets.values().filter(|bucket| bucket.len() >= 2) {
		cancellation.heartbeat()?;
		let mut groups: BTreeMap<Vec<&str>, Vec<usize>> = BTreeMap::new();
		for position in bucket {
			let (file, window) = positions[*position];
			groups
				.entry(
					significant_files[file][window..window + WINDOW]
						.iter()
						.map(|(line, _)| line.as_str())
						.collect(),
				)
				.or_default()
				.push(*position);
		}
		for group in groups.values().filter(|group| group.len() >= 2) {
			for position in group {
				matched.insert(*position);
				union(&mut parents, group[0], *position);
			}
		}
	}
	for position in &matched {
		let next = *position + 1;
		if matched.contains(&next) && positions[*position].0 == positions[next].0 {
			union(&mut parents, *position, next);
		}
	}
	let mut clusters: BTreeMap<usize, BTreeMap<usize, Vec<usize>>> = BTreeMap::new();
	for position in matched {
		let (file, window) = positions[position];
		clusters
			.entry(root(&mut parents, position))
			.or_default()
			.entry(file)
			.or_default()
			.push(window);
	}
	let mut result = Vec::new();
	for cluster in clusters.values_mut() {
		cancellation.heartbeat()?;
		let mut regions = Vec::new();
		let mut count = 0;
		for (file, windows) in cluster {
			windows.sort_unstable();
			let mut start = windows[0];
			let mut previous = start;
			let mut flush = |start: usize, end: usize| {
				let lines = &significant_files[*file];
				regions.push(format!(
					"{}:{}-{}",
					files[*file].rel,
					lines[start].1,
					lines[end + WINDOW - 1].1
				));
				count += end + WINDOW - start;
			};
			for window in windows.iter().skip(1) {
				if *window > previous + WINDOW {
					flush(start, previous);
					start = *window;
				}
				previous = *window;
			}
			flush(start, previous);
		}
		if regions.len() >= 2 && count >= WINDOW * 2 {
			result.push(CloneCluster { duplicated_sig_lines: count, regions });
		}
	}
	result.sort_by(|a, b| {
		b.duplicated_sig_lines
			.cmp(&a.duplicated_sig_lines)
			.then_with(|| a.regions.cmp(&b.regions))
	});
	Ok(result)
}
