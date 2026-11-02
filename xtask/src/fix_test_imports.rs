//! Codemod: rewrite relative test imports that reach into a package's `src/`
//! into the package's public subpath import.
//!
//! `../src/format` → `@harvest/pi-utils/format`; `../src/index` → the bare
//! package name. Only specifiers resolving onto a module file under
//! `<pkg>/src/` are touched; asset imports (`.json`, `.md`, …) have no public
//! subpath and are reported instead.
use std::{
	collections::HashMap,
	fmt::Write as _,
	fs,
	path::{Component, Path, PathBuf},
	sync::LazyLock,
};

use anyhow::Result;
use clap::Parser;
use regex::{Captures, Regex};

const MODULE_EXTS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const FILE_LIMIT: u64 = 8 * 1024 * 1024;
// `from "x"`, `import("x")`, `require("x")`, `import "x"` with a relative
// specifier; the two alternatives stand in for a matching-quote backreference.
static SPEC: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
		r#"(\bfrom\s*|\bimport\s*\(\s*|\brequire\s*\(\s*|\bimport\s+)(?:"(\.\.?/[^"']*)"|'(\.\.?/[^"']*)')"#,
	)
	.expect("valid regex")
});

#[derive(Parser)]
#[command(name = "cargo xtask fix-test-imports")]
/// Rewrite relative test imports into package subpath imports (dry run by
/// default).
struct Arguments {
	/// Apply the changes.
	#[arg(long)]
	write:     bool,
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
}

/// Lexically normalize like Node's `path.resolve` (no symlink resolution).
fn normalize(path: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for component in path.components() {
		match component {
			Component::ParentDir => {
				out.pop();
			},
			Component::CurDir => {},
			other => out.push(other),
		}
	}
	out
}

struct Resolver {
	root:     PathBuf,
	packages: HashMap<PathBuf, Option<(PathBuf, String)>>,
}

impl Resolver {
	fn package(&mut self, start: &Path) -> Option<(PathBuf, String)> {
		let mut dir = start.to_path_buf();
		let mut visited = Vec::new();
		let found = loop {
			if !dir.starts_with(&self.root) {
				break None;
			}
			if let Some(cached) = self.packages.get(&dir) {
				break cached.clone();
			}
			visited.push(dir.clone());
			let name = fs::read(dir.join("package.json"))
				.ok()
				.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
				.and_then(|json| json.get("name")?.as_str().map(str::to_owned))
				.filter(|name| !name.is_empty());
			if let Some(name) = name {
				break Some((dir.clone(), name));
			}
			if !dir.pop() {
				break None;
			}
		};
		for path in visited {
			self.packages.insert(path, found.clone());
		}
		found
	}
}

/// Resolve a relative specifier to the file it loads: the literal path, the TS
/// file behind a `.js`-style specifier, appended extensions, then `index`.
fn resolve_target(from: &Path, spec: &str) -> Option<PathBuf> {
	let base = normalize(&from.parent()?.join(spec));
	let text = base.to_string_lossy().into_owned();
	let mut candidates = vec![base.clone()];
	if let Some(stem) = ["js", "jsx", "mjs", "cjs"].iter().find_map(|ext| {
		let suffix = format!(".{ext}");
		(text.len() > suffix.len() && text[text.len() - suffix.len()..].eq_ignore_ascii_case(&suffix))
			.then(|| text[..text.len() - suffix.len()].to_owned())
	}) {
		candidates
			.extend(["ts", "tsx", "mts", "cts"].map(|ext| PathBuf::from(format!("{stem}.{ext}"))));
	}
	candidates.extend(MODULE_EXTS.map(|ext| PathBuf::from(format!("{text}.{ext}"))));
	candidates.extend(MODULE_EXTS.map(|ext| base.join(format!("index.{ext}"))));
	candidates.into_iter().find(|candidate| candidate.is_file())
}

#[derive(Debug, PartialEq, Eq)]
struct Rewrite {
	content: String,
	changes: Vec<(String, String)>,
	skipped: Vec<String>,
}

fn rewrite(resolver: &mut Resolver, file: &Path, source: &str) -> Rewrite {
	let mut changes = Vec::new();
	let mut skipped = Vec::new();
	let content = SPEC.replace_all(source, |captures: &Captures| {
		let full = captures[0].to_owned();
		let (quote, spec) = match (captures.get(2), captures.get(3)) {
			(Some(spec), _) => ('"', spec.as_str()),
			(_, Some(spec)) => ('\'', spec.as_str()),
			_ => return full,
		};
		let Some(target) = resolve_target(file, spec) else {
			return full;
		};
		let Some((root, name)) = resolver.package(target.parent().unwrap_or(&target)) else {
			return full;
		};
		let Ok(sub) = target.strip_prefix(root.join("src")) else {
			return full;
		};
		let ext = target
			.extension()
			.map(|ext| ext.to_string_lossy().to_lowercase())
			.unwrap_or_default();
		if !MODULE_EXTS.contains(&ext.as_str()) {
			skipped.push(spec.to_owned());
			return full;
		}
		let sub = sub
			.components()
			.map(|part| part.as_os_str().to_string_lossy())
			.collect::<Vec<_>>()
			.join("/");
		let mut sub = sub[..sub.len() - ext.len() - 1].to_owned();
		if sub.len() >= 6 && sub[sub.len() - 6..].eq_ignore_ascii_case("/index") {
			sub.truncate(sub.len() - 6);
		}
		if sub == "index" {
			sub.clear();
		}
		let new_spec = if sub.is_empty() {
			name
		} else {
			format!("{name}/{sub}")
		};
		if new_spec == spec {
			return full;
		}
		changes.push((spec.to_owned(), new_spec.clone()));
		format!("{}{quote}{new_spec}{quote}", &captures[1])
	});
	Rewrite { content: content.into_owned(), changes, skipped }
}

/// `packages/*/{test,tests}/**/*.{ts,tsx}`, skipping dot entries and symlinks.
fn test_files(root: &Path) -> Result<Vec<PathBuf>> {
	fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
		for entry in fs::read_dir(dir)? {
			let entry = entry?;
			let name = entry.file_name();
			if name.to_string_lossy().starts_with('.') {
				continue;
			}
			let kind = entry.file_type()?;
			let path = entry.path();
			if kind.is_dir() {
				walk(&path, out)?;
			} else if kind.is_file()
				&& path
					.extension()
					.is_some_and(|ext| ext == "ts" || ext == "tsx")
			{
				out.push(path);
			}
		}
		Ok(())
	}
	let mut files = Vec::new();
	for package in fs::read_dir(root.join("packages"))? {
		let package = package?;
		if !package.file_type()?.is_dir() || package.file_name().to_string_lossy().starts_with('.') {
			continue;
		}
		for dir in ["test", "tests"] {
			let dir = package.path().join(dir);
			if fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir()) {
				walk(&dir, &mut files)?;
			}
		}
	}
	// Match the legacy JavaScript string sort (UTF-16 units), not path-component
	// order.
	files.sort_by(|a, b| {
		a.to_string_lossy()
			.encode_utf16()
			.cmp(b.to_string_lossy().encode_utf16())
	});
	files.dedup();
	Ok(files)
}

fn display(root: &Path, path: &Path) -> String {
	path
		.strip_prefix(root)
		.unwrap_or(path)
		.to_string_lossy()
		.into_owned()
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("fix-test-imports", arguments)? else {
		return Ok(());
	};
	print!("{}", run(&args.workspace, args.write)?);
	Ok(())
}

fn run(workspace: &Path, write: bool) -> Result<String> {
	let root = normalize(&std::path::absolute(workspace)?);
	let mut resolver = Resolver { root: root.clone(), packages: HashMap::new() };
	let (mut total, mut changed) = (0, 0);
	let mut skipped_assets = Vec::new();
	let mut report = String::new();
	for file in test_files(&root)? {
		if fs::metadata(&file)?.len() > FILE_LIMIT {
			continue;
		}
		let source = fs::read_to_string(&file)?;
		let result = rewrite(&mut resolver, &file, &source);
		let relative = display(&root, &file);
		skipped_assets.extend(
			result
				.skipped
				.iter()
				.map(|spec| format!("{relative}: {spec}")),
		);
		if result.changes.is_empty() {
			continue;
		}
		changed += 1;
		total += result.changes.len();
		writeln!(report, "\n{relative}")?;
		for (from, to) in &result.changes {
			writeln!(report, "  {from}  ->  {to}")?;
		}
		if write {
			fs::write(&file, result.content)?;
		}
	}
	writeln!(
		report,
		"\n{} {total} rewrite(s) across {changed} file(s).",
		if write { "Applied" } else { "Would apply" }
	)?;
	if !skipped_assets.is_empty() {
		writeln!(
			report,
			"\nSkipped {} asset import(s) into src (no public subpath):",
			skipped_assets.len()
		)?;
		for skipped in skipped_assets {
			writeln!(report, "  {skipped}")?;
		}
	}
	if !write && total > 0 {
		report.push_str("\nRe-run with --write to apply.\n");
	}
	Ok(report)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn put(root: &Path, path: &str, text: &str) {
		let path = root.join(path);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, text).unwrap();
	}

	#[test]
	fn rewrites_module_imports_and_reports_assets() -> Result<()> {
		let dir = tempfile::tempdir()?;
		let root = dir.path();
		put(root, "packages/u/package.json", r#"{"name":"@h/u"}"#);
		put(root, "packages/u/src/index.ts", "");
		put(root, "packages/u/src/format.ts", "");
		put(root, "packages/u/src/task/index.ts", "");
		put(root, "packages/u/src/data.json", "{}");
		put(
			root,
			"packages/u/test/a.test.ts",
			"import x from \"../src/format\";\nimport y from '../src/index.js';\nconst t = await \
			 import(\"../src/task\");\nimport d from \"../src/data.json\";\nimport z from \
			 \"./helper\";\n",
		);
		let report = run(root, true)?;
		assert_eq!(
			fs::read_to_string(root.join("packages/u/test/a.test.ts"))?,
			"import x from \"@h/u/format\";\nimport y from '@h/u';\nconst t = await \
			 import(\"@h/u/task\");\nimport d from \"../src/data.json\";\nimport z from \
			 \"./helper\";\n"
		);
		assert!(report.contains("Applied 3 rewrite(s) across 1 file(s)."), "{report}");
		assert!(report.contains("packages/u/test/a.test.ts: ../src/data.json"), "{report}");
		Ok(())
	}

	#[test]
	fn dry_run_leaves_files_untouched() -> Result<()> {
		let dir = tempfile::tempdir()?;
		let root = dir.path();
		put(root, "packages/u/package.json", r#"{"name":"@h/u"}"#);
		put(root, "packages/u/src/a.ts", "");
		put(root, "packages/u/tests/b.ts", "export * from \"../src/a\";\n");
		let report = run(root, false)?;
		assert!(report.ends_with("Re-run with --write to apply.\n"), "{report}");
		assert_eq!(
			fs::read_to_string(root.join("packages/u/tests/b.ts"))?,
			"export * from \"../src/a\";\n"
		);
		Ok(())
	}
}
