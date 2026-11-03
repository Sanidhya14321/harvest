use std::fs;

use super::*;

fn run(root: &Path, args: &str) -> Result<String> {
	let args: Args = serde_json::from_str(args)?;
	Ok(grep(root, &args, &EditStore::new(), &CancelToken::default())?.render())
}

fn workspace() -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir_all(root.path().join("src/nested")).unwrap();
	fs::write(root.path().join("src/a.rs"), "one\nneedle a\nthree\n").unwrap();
	fs::write(root.path().join("src/nested/b.rs"), "needle b\n").unwrap();
	fs::write(root.path().join("top.txt"), "x\nneedle top\n").unwrap();
	root
}

#[test]
fn directory_scopes_group_files_under_folded_headers_with_tags() -> Result<()> {
	let root = workspace();
	let out = run(root.path(), r#"{"pattern":"needle","path":"src"}"#)?;
	let lines: Vec<&str> = out.lines().collect();
	assert_eq!(lines[0], "# src/");
	assert!(lines[1].starts_with("## a.rs#"), "{out}");
	assert_eq!(&lines[2..5], [" 1:one", "*2:needle a", " 3:three"]);
	assert_eq!(lines[5], "");
	assert_eq!(lines[6], "## nested/");
	assert!(lines[7].starts_with("### b.rs#"), "{out}");
	assert_eq!(lines[8], "*1:needle b");
	Ok(())
}

#[test]
fn single_file_scopes_use_a_bracketed_header_and_line_ranges_filter() -> Result<()> {
	let root = workspace();
	let out = run(root.path(), r#"{"pattern":"e","path":"src/a.rs:2-3"}"#)?;
	let lines: Vec<&str> = out.lines().collect();
	assert!(lines[0].starts_with("[src/a.rs#"), "{out}");
	assert_eq!(&lines[1..], ["*2:needle a", "*3:three"]);
	Ok(())
}

#[test]
fn pagination_windows_files_and_reports_the_next_skip() -> Result<()> {
	let root = tempfile::tempdir()?;
	for index in 0..25 {
		fs::write(root.path().join(format!("f{index:02}.txt")), "hit\n")?;
	}
	let out = run(root.path(), r#"{"pattern":"hit"}"#)?;
	assert!(
		out.ends_with(
			"Showing files 1-20 of 25. Use skip=20 for the next page, or narrow paths/pattern."
		),
		"{out}"
	);
	let out = run(root.path(), r#"{"pattern":"hit","skip":20}"#)?;
	assert!(out.starts_with("# f20.txt#"), "{out}");
	assert_eq!(
		run(root.path(), r#"{"pattern":"hit","skip":40}"#)?,
		"No more results (25 files total; skip=40 is past the end)"
	);
	Ok(())
}

#[test]
fn semicolon_lists_skip_missing_entries_and_reject_bad_input() -> Result<()> {
	let root = workspace();
	let out = run(root.path(), r#"{"pattern":"needle","path":"top.txt; nope"}"#)?;
	assert!(out.ends_with("\n\nSkipped missing paths: nope"), "{out}");
	let error = run(root.path(), r#"{"pattern":"needle","path":"nope; gone"}"#).unwrap_err();
	assert_eq!(error.to_string(), "Path not found: nope, gone");
	let error = run(root.path(), r#"{"pattern":"   "}"#).unwrap_err();
	assert_eq!(error.to_string(), "Pattern must not be empty");
	let error = run(root.path(), r#"{"pattern":"x","path":"src/*.rs:1-2"}"#).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("requires a single file, not a glob")
	);
	let error = run(root.path(), r#"{"pattern":"x","path":"src:1-2"}"#).unwrap_err();
	assert!(error.to_string().contains("src:1-2 is a directory"));
	Ok(())
}

#[test]
fn per_file_caps_keep_hot_files_from_crowding_out_others() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::write(root.path().join("hot.txt"), "hit\n".repeat(50))?;
	fs::write(root.path().join("cold.txt"), "hit\n")?;
	let out = run(root.path(), r#"{"pattern":"hit"}"#)?;
	assert_eq!(out.matches('*').count(), 21, "{out}");
	Ok(())
}

#[test]
fn grep_skips_symlinked_directories() -> Result<()> {
	let outside = tempfile::tempdir()?;
	fs::write(outside.path().join("secret.txt"), "outside-secret\n")?;
	let root = tempfile::tempdir()?;
	#[cfg(unix)]
	std::os::unix::fs::symlink(outside.path(), root.path().join("link"))?;
	assert_eq!(run(root.path(), r#"{"pattern":"outside-secret"}"#)?, "No matches found");
	assert!(run(root.path(), r#"{"pattern":"outside-secret","path":"link"}"#).is_err());
	Ok(())
}

#[test]
fn archive_members_are_searched_in_memory_under_their_archive_path() -> Result<()> {
	use std::io::Write as _;
	let root = workspace();
	let mut out = std::io::Cursor::new(Vec::new());
	let mut zip = zip::ZipWriter::new(&mut out);
	zip.start_file("lib/x.rs", zip::write::SimpleFileOptions::default())?;
	zip.write_all(b"one\nneedle zip\n")?;
	zip.start_file("bin", zip::write::SimpleFileOptions::default())?;
	zip.write_all(b"needle\0")?;
	zip.finish()?;
	fs::write(root.path().join("pkg.zip"), out.into_inner())?;
	assert_eq!(
		run(root.path(), r#"{"pattern":"needle","path":"pkg.zip:lib/x.rs"}"#)?,
		" 1|one\n*2|needle zip"
	);
	let both = run(root.path(), r#"{"pattern":"needle","path":"top.txt; pkg.zip:bin"}"#)?;
	assert!(
		both.ends_with(
			"Skipped archive entries (search supports text members only): pkg.zip:bin (binary \
			 archive entry)"
		),
		"{both}"
	);
	let error = run(root.path(), r#"{"pattern":"needle","path":"pkg.zip:bin"}"#)
		.unwrap_err()
		.to_string();
	assert!(
		error.starts_with("Cannot search archive member(s): pkg.zip:bin (binary archive entry)."),
		"{error}"
	);
	Ok(())
}
