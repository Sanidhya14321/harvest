use super::*;

fn lines(parts: &[&str]) -> String {
	parts.join("\n")
}

fn set(values: &[usize]) -> HashSet<usize> {
	values.iter().copied().collect()
}

#[test]
fn keeps_new_item_additions_while_ignoring_moves_and_edits() {
	let diff = lines(&[
		"diff --git a/packages/example/CHANGELOG.md b/packages/example/CHANGELOG.md",
		"--- a/packages/example/CHANGELOG.md",
		"+++ b/packages/example/CHANGELOG.md",
		"@@ -10,0 +11,2 @@",
		"+",
		"+- Added after the latest tag in a released section.",
		"@@ -20 +22 @@",
		"-- Moved historical entry.",
		"+- Moved historical entry.",
		"@@ -30 +32,2 @@",
		"-- Historical entry with old wording.",
		"+- Historical entry with new wording.",
		"+- Another brand-new item in the same hunk.",
	]);
	let found = promotable_lines(&diff);
	assert_eq!(found.get("packages/example/CHANGELOG.md"), Some(&BTreeSet::from([12, 33])));
}

#[test]
fn does_not_promote_items_from_newly_added_release_sections() {
	let diff = lines(&[
		"diff --git a/packages/example/CHANGELOG.md b/packages/example/CHANGELOG.md",
		"--- a/packages/example/CHANGELOG.md",
		"+++ b/packages/example/CHANGELOG.md",
		"@@ -1,0 +1,8 @@",
		"+# Changelog",
		"+",
		"+## [1.0.0] - 2026-01-01",
		"+",
		"+### Fixed",
		"+",
		"+- Released fix.",
		"+- Another released fix.",
	]);
	assert!(promotable_lines(&diff).is_empty());
}

#[test]
fn promotes_released_additions_and_merges_duplicate_headings() {
	let content = lines(&[
		"# Changelog",
		"",
		"## [Unreleased]",
		"### Fixed",
		"",
		"- Existing fix.",
		"",
		"### Fixed",
		"",
		"- Second fix.",
		"",
		"## [1.0.0] - 2026-01-01",
		"",
		"### Added",
		"",
		"- Historical addition.",
		"- New addition in released section.",
		"",
		"### Fixed",
		"",
		"- Historical fix.",
		"- New fix in released section.",
		"",
	]);
	let result = fix_content(&content, &set(&[17, 22]), &HashSet::new());
	assert_eq!((result.promoted_items, result.merged_duplicate_headings), (2, 1));
	assert_eq!(
		result.content,
		lines(&[
			"# Changelog",
			"",
			"## [Unreleased]",
			"",
			"### Added",
			"",
			"- New addition in released section.",
			"",
			"### Fixed",
			"",
			"- Existing fix.",
			"- Second fix.",
			"- New fix in released section.",
			"",
			"## [1.0.0] - 2026-01-01",
			"",
			"### Added",
			"",
			"- Historical addition.",
			"",
			"### Fixed",
			"",
			"- Historical fix.",
			"",
		])
	);
}

#[test]
fn drops_unreleased_items_already_released_verbatim() {
	let content = lines(&[
		"# Changelog",
		"",
		"## [Unreleased]",
		"",
		"### Added",
		"",
		"- Brand-new unreleased feature.",
		"- Added fullscreen settings mouse-event handling.",
		"",
		"## [1.1.0] - 2026-02-01",
		"",
		"### Added",
		"",
		"- Added fullscreen settings mouse-event handling.",
		"",
	]);
	let result = fix_content(&content, &HashSet::new(), &HashSet::new());
	assert_eq!((result.dropped_released_duplicates, result.promoted_items), (1, 0));
	assert_eq!(
		result.content,
		lines(&[
			"# Changelog",
			"",
			"## [Unreleased]",
			"",
			"### Added",
			"",
			"- Brand-new unreleased feature.",
			"",
			"## [1.1.0] - 2026-02-01",
			"",
			"### Added",
			"",
			"- Added fullscreen settings mouse-event handling.",
			"",
		])
	);
}

#[test]
fn recovers_unreleased_by_dropping_historically_released_bullets() {
	let content = lines(&[
		"# Changelog",
		"",
		"## [Unreleased]",
		"",
		"### Fixed",
		"",
		"- Historical fix still stranded in Unreleased.",
		"- Brand-new fix.",
		"",
		"## [1.1.0] - 2026-02-01",
		"",
	]);
	let historical = HashSet::from(["- Historical fix still stranded in Unreleased.".to_owned()]);
	let result = fix_content(&content, &HashSet::new(), &historical);
	assert_eq!(result.dropped_released_duplicates, 1);
	assert_eq!(
		result.content,
		lines(&[
			"# Changelog",
			"",
			"## [Unreleased]",
			"",
			"### Fixed",
			"",
			"- Brand-new fix.",
			"",
			"## [1.1.0] - 2026-02-01",
			"",
		])
	);
}

#[test]
fn compacts_blank_separators_and_leaves_clean_files_untouched() {
	let spaced = lines(&[
		"# Changelog",
		"",
		"## [Unreleased]",
		"",
		"### Fixed",
		"",
		"- First fix.",
		"",
		"- Second fix.",
		"",
		"## [1.1.0] - 2026-02-01",
		"",
	]);
	assert_eq!(
		fix_content(&spaced, &HashSet::new(), &HashSet::new()).content,
		lines(&[
			"# Changelog",
			"",
			"## [Unreleased]",
			"",
			"### Fixed",
			"",
			"- First fix.",
			"- Second fix.",
			"",
			"## [1.1.0] - 2026-02-01",
			"",
		])
	);
	let clean = lines(&[
		"# Changelog",
		"",
		"## [Unreleased]",
		"",
		"### Added",
		"",
		"- Only an unreleased feature.",
		"",
		"## [1.1.0] - 2026-02-01",
		"",
		"### Added",
		"",
		"- A released feature.",
		"",
	]);
	let result = fix_content(&clean, &HashSet::new(), &HashSet::new());
	assert_eq!((result.dropped_released_duplicates, result.content), (0, clean));
}

const LINK: &str = "Older entries are archived in \
                    [packages/foo/CHANGELOG.md@abc123def456](https://github.com/harvest/harvest/blob/\
                    abc123def456abc123def456abc123def456abc1/packages/foo/CHANGELOG.md).";

fn four_sections() -> String {
	lines(&[
		"# Changelog",
		"",
		"## [Unreleased]",
		"",
		"### Added",
		"",
		"- New feature.",
		"",
		"## [1.2.0] - 2026-03-01",
		"",
		"### Fixed",
		"",
		"- Fix three.",
		"",
		"## [1.1.0] - 2026-02-01",
		"",
		"### Fixed",
		"",
		"- Fix two.",
		"",
		"## [1.0.0] - 2026-01-01",
		"",
		"### Fixed",
		"",
		"- Fix one.",
		"",
	])
}

#[test]
fn collapse_drops_oldest_sections_until_the_budget_fits_and_keeps_two() {
	let content = four_sections();
	let footer = LINK.len() + 2;
	let kept = format!("{}\n", &content[..content.find("\n\n## [1.0.0]").unwrap()]);
	let (collapsed, releases) = collapse_tail(&content, LINK, kept.len() + footer);
	assert_eq!((collapsed.as_str(), releases), (kept.as_str(), 1));

	let (collapsed, releases) = collapse_tail(&content, LINK, 16);
	assert_eq!(releases, 2);
	assert!(collapsed.contains("## [1.2.0]") && !collapsed.contains("## [1.1.0]"));

	assert_eq!(collapse_tail(&content, LINK, 1024 * 1024), (content, 0));
}

#[test]
fn archive_footer_is_split_from_the_body() {
	let content = four_sections();
	assert_eq!(
		split_archive_footer(&format!("{content}\n{LINK}\n")),
		(content.clone(), Some(LINK.to_owned()))
	);
	assert_eq!(split_archive_footer(&content), (content, None));
}

struct Repo(tempfile::TempDir);

impl Repo {
	fn new() -> Self {
		let repo = Self(tempfile::tempdir().unwrap());
		repo.git(&["init", "-q", "-b", "main"]);
		repo
	}

	fn path(&self) -> &Path {
		self.0.path()
	}

	fn git(&self, args: &[&str]) -> String {
		let output = Command::new("git")
			.args(args)
			.current_dir(self.path())
			.env("GIT_CONFIG_GLOBAL", "/dev/null")
			.env("GIT_CONFIG_SYSTEM", "/dev/null")
			.env("GIT_AUTHOR_NAME", "t")
			.env("GIT_AUTHOR_EMAIL", "t@t")
			.env("GIT_COMMITTER_NAME", "t")
			.env("GIT_COMMITTER_EMAIL", "t@t")
			.output()
			.unwrap();
		assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
		String::from_utf8(output.stdout).unwrap()
	}

	fn commit(&self, content: &str, message: &str) {
		let path = self.path().join("packages/foo/CHANGELOG.md");
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, content).unwrap();
		self.git(&["add", "-A"]);
		self.git(&["commit", "-q", "-m", message]);
	}

	fn options(&self, write: bool, max_bytes: usize) -> Options<'_> {
		Options { root: Some(self.path()), since: None, write, recover: false, max_bytes }
	}
}

#[test]
fn clog_baseline_stops_a_recovered_released_bullet_from_being_promoted() {
	let repo = Repo::new();
	repo.commit(
		"# Changelog\n\n## [Unreleased]\n\n## [1.0.0] - 2025-01-01\n\n### Fixed\n\n- Old released \
		 bullet.\n",
		"release",
	);
	repo.git(&["tag", "v1.0.0"]);
	repo.commit(
		"# Changelog\n\n## [Unreleased]\n\n## [1.0.0] - 2025-01-01\n\n### Fixed\n\n- Old released \
		 bullet.\n- Recovered bullet.\n",
		"recover",
	);

	let (since, changed) = run(&repo.options(false, MAX_CHANGELOG_BYTES)).unwrap();
	assert_eq!(since, "v1.0.0");
	assert_eq!(changed[0].promoted_items, 1);

	repo.git(&["update-ref", BASELINE_REF, "HEAD"]);
	let (since, changed) = run(&repo.options(false, MAX_CHANGELOG_BYTES)).unwrap();
	assert_eq!(since, BASELINE_REF);
	assert!(changed.is_empty());
}

#[test]
fn oversized_changelogs_collapse_behind_an_archive_link_idempotently() {
	let repo = Repo::new();
	let content = four_sections().replace("- Fix one.", &format!("- Fix one. {}", "x".repeat(400)));
	repo.commit(&content, "release");
	repo.git(&["tag", "v1.2.0"]);
	let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
	let link = archive_link("packages/foo/CHANGELOG.md", &head);
	let max_bytes = content.find("\n\n## [1.0.0]").unwrap() + 1 + link.len() + 2;

	let (_, changed) = run(&repo.options(true, max_bytes)).unwrap();
	assert_eq!(changed, vec![Changed {
		path: "packages/foo/CHANGELOG.md".into(),
		promoted_items: 0,
		merged_duplicate_headings: 0,
		dropped_released_duplicates: 0,
		removed_empty_headings: 0,
		collapsed_releases: 1,
	}]);
	let written = fs::read_to_string(repo.path().join("packages/foo/CHANGELOG.md")).unwrap();
	assert!(!written.contains("## [1.0.0]") && written.contains("## [1.1.0]"));
	assert!(written.ends_with(&format!("\n{link}\n")) && written.len() <= max_bytes);

	let (_, changed) = run(&repo.options(true, max_bytes)).unwrap();
	assert!(changed.is_empty());
	assert_eq!(fs::read_to_string(repo.path().join("packages/foo/CHANGELOG.md")).unwrap(), written);
}
