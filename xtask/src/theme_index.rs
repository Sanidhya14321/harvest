//! Embedded theme registry generation for the retained frontend.
use std::{
	fmt::Write,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;

const DIRECTORY: &str = "packages/coding-agent/src/modes/theme/defaults";
const OUTPUT: &str = "packages/coding-agent/src/modes/theme/defaults/index.ts";
const ENTRY_LIMIT: usize = 4096;

#[derive(Parser)]
#[command(name = "cargo xtask theme-index")]
struct Arguments {
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	check:     bool,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("theme-index", arguments)? else {
		return Ok(());
	};
	let (changed, count) = generate(&args.workspace, args.check)?;
	if changed {
		println!("Updated {OUTPUT} with {count} themes.");
	} else {
		println!("{OUTPUT} is in sync ({count} themes).");
	}
	Ok(())
}

fn generate(root: &Path, check: bool) -> Result<(bool, usize)> {
	let fs = JailedFs::open(root)?;
	let mut files = Vec::new();
	for entry in fs.entries(Path::new(DIRECTORY), ENTRY_LIMIT)? {
		let Some(stem) = entry.name.strip_suffix(".json") else {
			continue;
		};
		if !entry.is_file || entry.is_symlink {
			bail!("embedded theme assets must be regular files");
		}
		if stem.is_empty()
			|| entry
				.name
				.chars()
				.any(|ch| ch.is_control() || "/\\#?%".contains(ch))
		{
			bail!("embedded theme asset name is empty or contains path/URL separators");
		}
		files.push(entry.name);
	}
	// Preserve the legacy JavaScript UTF-16 ordering, including non-BMP names.
	files.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
	let content = render(&files)?;
	let changed = crate::generated::publish(
		&fs,
		Path::new(OUTPUT),
		content.as_bytes(),
		check,
		"cargo xtask theme-index",
	)?;
	Ok((changed, files.len()))
}

fn render(files: &[String]) -> Result<String> {
	if files.is_empty() {
		return Ok("export const defaultThemes = {};\n".to_owned());
	}
	let mut output = String::new();
	for (index, file) in files.iter().enumerate() {
		let path = serde_json::to_string(&format!("./{file}"))?;
		writeln!(output, "import theme_{index} from {path} with {{ type: \"json\" }};")?;
	}
	output.push_str("\nexport const defaultThemes = {\n");
	for (index, file) in files.iter().enumerate() {
		let name = file
			.strip_suffix(".json")
			.context("theme extension is missing")?;
		// Computed keys retain __proto__ as an own data property; private
		// bindings never depend on identifiers or filename-normalization aliases.
		writeln!(output, "\t[{}]: theme_{index},", serde_json::to_string(name)?)?;
	}
	output.push_str("};\n");
	Ok(output)
}

#[cfg(test)]
mod tests {
	use super::*;
	fn root() -> Result<tempfile::TempDir> {
		let root = tempfile::tempdir()?;
		std::fs::create_dir_all(root.path().join(DIRECTORY))?;
		Ok(root)
	}
	#[test]
	fn sorted_assets_generate_distinct_import_bindings_and_preserve_their_public_names() -> Result<()>
	{
		let root = root()?;
		for file in ["light_mode.json", "light-mode.json", "default.json", "__proto__.json"] {
			std::fs::write(root.path().join(DIRECTORY).join(file), b"{}")?;
		}
		std::fs::write(root.path().join(DIRECTORY).join("notes.md"), b"ignored")?;
		generate(root.path(), false)?;
		assert_eq!(
			std::fs::read_to_string(root.path().join(OUTPUT))?,
			concat!(
				"import theme_0 from \"./__proto__.json\" with { type: \"json\" };\n",
				"import theme_1 from \"./default.json\" with { type: \"json\" };\n",
				"import theme_2 from \"./light-mode.json\" with { type: \"json\" };\n",
				"import theme_3 from \"./light_mode.json\" with { type: \"json\" };\n",
				"\nexport const defaultThemes = {\n",
				"\t[\"__proto__\"]: theme_0,\n\t[\"default\"]: theme_1,\n",
				"\t[\"light-mode\"]: theme_2,\n\t[\"light_mode\"]: theme_3,\n};\n",
			)
		);
		Ok(())
	}
	#[test]
	fn check_mode_preserves_stale_output_until_generation_removes_deleted_assets() -> Result<()> {
		let root = root()?;
		let asset = root.path().join(DIRECTORY).join("obsolete.json");
		std::fs::write(&asset, b"{}")?;
		assert!(generate(root.path(), true).is_err());
		assert!(!root.path().join(OUTPUT).exists());
		generate(root.path(), false)?;
		let old = std::fs::read(root.path().join(OUTPUT))?;
		std::fs::remove_file(asset)?;
		assert!(generate(root.path(), true).is_err());
		assert_eq!(std::fs::read(root.path().join(OUTPUT))?, old);
		generate(root.path(), false)?;
		assert_eq!(
			std::fs::read_to_string(root.path().join(OUTPUT))?,
			"export const defaultThemes = {};\n"
		);
		assert!(!generate(root.path(), true)?.0);
		Ok(())
	}
	#[test]
	fn unicode_names_keep_javascript_code_unit_order() -> Result<()> {
		let root = root()?;
		for file in ["Ｘ.json", "𐐀.json"] {
			std::fs::write(root.path().join(DIRECTORY).join(file), b"{}")?;
		}
		generate(root.path(), false)?;
		let expected = render(&["𐐀.json".to_owned(), "Ｘ.json".to_owned()])?;
		assert_eq!(std::fs::read_to_string(root.path().join(OUTPUT))?, expected);
		Ok(())
	}
	#[test]
	fn malformed_asset_names_and_directory_assets_cannot_replace_the_registry() -> Result<()> {
		for name in [".json", "bad#fragment.json", "directory.json"] {
			let root = root()?;
			let asset = root.path().join(DIRECTORY).join(name);
			if name == "directory.json" {
				std::fs::create_dir(asset)?;
			} else {
				std::fs::write(asset, b"{}")?;
			}
			std::fs::write(root.path().join(OUTPUT), b"original")?;
			assert!(generate(root.path(), false).is_err());
			assert_eq!(std::fs::read(root.path().join(OUTPUT))?, b"original");
		}
		Ok(())
	}
	#[cfg(unix)]
	#[test]
	fn an_escaping_theme_directory_cannot_rewrite_an_outside_registry() -> Result<()> {
		let root = tempfile::tempdir()?;
		let outside = tempfile::tempdir()?;
		let target = root.path().join(DIRECTORY);
		std::fs::create_dir_all(target.parent().unwrap())?;
		std::fs::write(outside.path().join("theme.json"), b"{}")?;
		std::fs::write(outside.path().join("index.ts"), b"outside")?;
		std::os::unix::fs::symlink(outside.path(), target)?;
		assert!(generate(root.path(), false).is_err());
		assert_eq!(std::fs::read(outside.path().join("index.ts"))?, b"outside");
		Ok(())
	}
}
