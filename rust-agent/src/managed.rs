//! Managed skills and agent presets: an isolated, versioned artifact store
//! under the agent directory plus the `manage_skill` and `presets` tools.
//!
//! Every mutation goes through a [`JailedFs`] rooted at the agent directory,
//! and artifact names/revision ids are allowlist-validated, so nothing can
//! escape `managed-skills/`, `managed-presets/` or `managed-revisions/`.
//! Active revisions are immutable; updates mint new revisions and move the
//! `active.json` pointer.

use std::{
	collections::BTreeSet,
	hash::{BuildHasher, Hasher},
	path::{Path, PathBuf},
	sync::Mutex,
};

use anyhow::{Context, Result, bail};
use harvest_runtime::JailedFs;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serde_yaml_ng::Mapping;

pub const MAX_REVISIONS_PER_ARTIFACT: usize = 20;
pub const MAX_MANAGED_ARTIFACT_BYTES: usize = 64_000;
const READ_LIMIT: usize = 1024 * 1024;
const SKILL_NAME: &str = r"^[a-z0-9][a-z0-9-]{0,63}$";
const PRESET_NAME: &str = r"^[a-z0-9]+(?:-[a-z0-9]+){1,5}$";
const REVISION_ID: &str = r"^rev-[a-z0-9]+(?:-[a-z0-9]+)*$";

/// In-process serialization of all managed mutations (TS: per-artifact chain).
static TRANSACTION: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	Skill,
	Preset,
}

impl Kind {
	const fn label(self) -> &'static str {
		match self {
			Self::Skill => "skill",
			Self::Preset => "preset",
		}
	}

	const fn revision_root(self) -> &'static str {
		match self {
			Self::Skill => "managed-revisions/skills",
			Self::Preset => "managed-revisions/presets",
		}
	}

	fn file(self, name: &str) -> PathBuf {
		match self {
			Self::Skill => Path::new("managed-skills").join(name).join("SKILL.md"),
			Self::Preset => Path::new("managed-presets").join(format!("{name}.md")),
		}
	}
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
	pub id:               String,
	pub task:             String,
	pub expected_outcome: String,
	pub passed:           bool,
	pub summary:          String,
	pub created_at:       String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
	pub id:          String,
	pub kind:        String,
	pub name:        String,
	pub parent:      Option<String>,
	pub content:     String,
	pub description: String,
	pub state:       String,
	pub provenance:  Value,
	pub evaluations: Vec<Evaluation>,
	pub created_at:  String,
}

impl Revision {
	fn eval_status(&self) -> &'static str {
		if self.evaluations.is_empty() {
			"unevaluated"
		} else if self.evaluations.iter().all(|e| e.passed) {
			"passing"
		} else {
			"failing"
		}
	}
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pointer {
	active:     Option<String>,
	updated_at: String,
}

fn now() -> String {
	chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn base36(mut value: u128) -> String {
	const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
	let mut out = Vec::new();
	loop {
		out.push(DIGITS[(value % 36) as usize]);
		value /= 36;
		if value == 0 {
			break;
		}
	}
	out.reverse();
	String::from_utf8(out).unwrap_or_default()
}

fn random_suffix(len: usize) -> String {
	let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
	hasher.write_u128(
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |d| d.as_nanos()),
	);
	let text = base36(u128::from(hasher.finish()));
	text.chars().rev().take(len).collect()
}

fn new_id(prefix: &str, len: usize) -> String {
	let millis = chrono::Utc::now().timestamp_millis().max(0) as u128;
	format!("{prefix}-{}-{}", base36(millis), random_suffix(len))
}

fn matches(pattern: &str, value: &str) -> bool {
	regex::Regex::new(pattern).is_ok_and(|re| re.is_match(value))
}

pub fn sanitize_skill_name(raw: &str) -> Result<String> {
	let name = raw.trim().to_lowercase();
	if !matches(SKILL_NAME, &name) {
		bail!(
			"Invalid skill name \"{raw}\". Use lowercase kebab-case (a-z, 0-9, '-'), starting with a \
			 letter or digit, max 64 chars."
		);
	}
	Ok(name)
}

pub fn sanitize_preset_name(raw: &str) -> Result<String> {
	let name = raw.trim().to_lowercase();
	if !matches(PRESET_NAME, &name) {
		bail!(
			"Invalid preset name \"{raw}\". Use lowercase kebab-case with 2-6 words (e.g. \
			 \"api-docs-writer\")."
		);
	}
	Ok(name)
}

pub fn validate_revision_id(raw: &str) -> Result<String> {
	let trimmed = raw.trim();
	if !matches(REVISION_ID, trimmed) || trimmed.contains("..") {
		bail!(
			"Invalid revision ID \"{raw}\": expected rev-[a-z0-9]+(-[a-z0-9]+)*, no separators, \
			 absolute paths, or traversal."
		);
	}
	Ok(trimmed.to_owned())
}

fn sanitize_description(raw: &str) -> String {
	let cleaned: String = raw
		.chars()
		.map(|c| {
			if c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{feff}') {
				' '
			} else {
				c
			}
		})
		.filter(|c| !matches!(c, '<' | '>' | '`'))
		.collect();
	cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split `---\nyaml\n---\nbody`; content without frontmatter is all body.
pub fn parse_frontmatter(content: &str) -> (Mapping, String) {
	let normalized = content.replace("\r\n", "\n");
	if let Some(rest) = normalized.strip_prefix("---\n")
		&& let Some(end) = rest.find("\n---")
	{
		let yaml = &rest[..end];
		let body = rest[end + 4..]
			.trim_start_matches(['-'])
			.trim_start_matches('\n');
		let mapping = serde_yaml_ng::from_str::<Mapping>(yaml).unwrap_or_default();
		return (mapping, body.to_owned());
	}
	(Mapping::new(), normalized)
}

fn render(frontmatter: &Mapping, body: &str, blank_line: bool) -> Result<String> {
	let yaml = serde_yaml_ng::to_string(frontmatter)?;
	let gap = if blank_line { "\n" } else { "" };
	Ok(format!("---\n{}\n---\n{gap}{}\n", yaml.trim_end(), body.trim()))
}

fn mapping_str(map: &Mapping, key: &str) -> Option<String> {
	map.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

/// The agent-directory jail every managed mutation goes through.
pub struct Store {
	fs:   JailedFs,
	root: PathBuf,
}

impl Store {
	pub fn open() -> Result<Self> {
		let root = crate::config::agent_dir_from(None, |key| std::env::var_os(key))
			.context("cannot resolve the agent directory (HOME is unset)")?;
		Ok(Self { fs: JailedFs::open_or_create(&root)?, root })
	}

	pub fn root(&self) -> &Path {
		&self.root
	}

	fn artifact_dir(kind: Kind, name: &str) -> PathBuf {
		Path::new(kind.revision_root()).join(name)
	}

	fn read_text(&self, path: &Path) -> Result<Option<String>> {
		Ok(self
			.fs
			.read_optional(path, READ_LIMIT)?
			.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
	}

	fn write_json(&self, path: &Path, value: &impl Serialize) -> Result<()> {
		let mut writer = harvest_runtime::limits::BoundedWriter::new(READ_LIMIT);
		let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
		let mut serializer = serde_json::Serializer::with_formatter(&mut writer, formatter);
		value.serialize(&mut serializer)?;
		self.fs.atomic_write(path, &writer.into_inner())
	}

	fn pointer(&self, kind: Kind, name: &str) -> Result<Pointer> {
		let path = Self::artifact_dir(kind, name).join("active.json");
		match self.read_text(&path)? {
			Some(text) => Ok(serde_json::from_str(&text)?),
			None => Ok(Pointer { active: None, updated_at: now() }),
		}
	}

	fn set_pointer(&self, kind: Kind, name: &str, active: &str) -> Result<()> {
		let path = Self::artifact_dir(kind, name).join("active.json");
		self.write_json(&path, &Pointer { active: Some(active.to_owned()), updated_at: now() })
	}

	pub fn revision(&self, kind: Kind, name: &str, rev: &str) -> Result<Option<Revision>> {
		let rev = validate_revision_id(rev)?;
		let path = Self::artifact_dir(kind, name).join(format!("{rev}.json"));
		self
			.read_text(&path)?
			.map(|text| serde_json::from_str(&text).map_err(Into::into))
			.transpose()
	}

	fn put_revision(&self, kind: Kind, revision: &Revision) -> Result<()> {
		let path = Self::artifact_dir(kind, &revision.name).join(format!("{}.json", revision.id));
		self.write_json(&path, revision)
	}

	/// `(active, revisions oldest-first)`.
	pub fn list(&self, kind: Kind, name: &str) -> Result<(Option<String>, Vec<Revision>)> {
		let dir = Self::artifact_dir(kind, name);
		let mut revisions = Vec::new();
		if self.fs.exists(&dir)? {
			for entry in self.fs.entries(&dir, 10_000)? {
				if entry.is_file && entry.name.starts_with("rev-") && entry.name.ends_with(".json") {
					let id = entry.name.trim_end_matches(".json");
					if let Some(rev) = self.revision(kind, name, id)? {
						revisions.push(rev);
					}
				}
			}
		}
		revisions.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
		Ok((self.pointer(kind, name)?.active, revisions))
	}

	fn active_revision(&self, kind: Kind, name: &str) -> Result<Option<Revision>> {
		match self.pointer(kind, name)?.active {
			Some(id) => self.revision(kind, name, &id),
			None => Ok(None),
		}
	}

	fn current_content(&self, kind: Kind, name: &str) -> Result<Option<String>> {
		if let Some(rev) = self.active_revision(kind, name)? {
			return Ok(Some(rev.content));
		}
		self.read_text(&kind.file(name))
	}

	/// First managed mutation of a pre-revision file seeds it into history.
	fn seed(&self, kind: Kind, name: &str) -> Result<()> {
		let (_, revisions) = self.list(kind, name)?;
		if !revisions.is_empty() {
			return Ok(());
		}
		let Some(content) = self.read_text(&kind.file(name))? else {
			return Ok(());
		};
		let (fm, _) = parse_frontmatter(&content);
		let mut rev =
			self.mint(kind, name, content, mapping_str(&fm, "description").unwrap_or_default(), None);
		rev.state = "active".into();
		rev.provenance = json!({ "actor": "seed" });
		self.put_revision(kind, &rev)?;
		self.set_pointer(kind, name, &rev.id)
	}

	fn mint(
		&self,
		kind: Kind,
		name: &str,
		content: String,
		description: String,
		parent: Option<String>,
	) -> Revision {
		Revision {
			id: new_id("rev", 6),
			kind: kind.label().into(),
			name: name.into(),
			parent,
			content,
			description,
			state: "draft".into(),
			provenance: json!({ "actor": "model" }),
			evaluations: Vec::new(),
			created_at: now(),
		}
	}

	fn check_expected(
		&self,
		kind: Kind,
		name: &str,
		expected: Option<&str>,
	) -> Result<Option<String>> {
		let active = self.pointer(kind, name)?.active;
		if let Some(expected) = expected {
			let expected = expected.trim();
			let expected = if matches!(expected, "" | "none" | "null") {
				None
			} else {
				Some(validate_revision_id(expected)?)
			};
			if active != expected {
				bail!(
					"Revision conflict for {} \"{name}\": expected active {}, found {}",
					kind.label(),
					expected.as_deref().unwrap_or("none"),
					active.as_deref().unwrap_or("none")
				);
			}
		}
		Ok(active)
	}

	fn draft(
		&self,
		kind: Kind,
		name: &str,
		content: String,
		description: String,
		expected: Option<&str>,
	) -> Result<Revision> {
		if content.len() > MAX_MANAGED_ARTIFACT_BYTES {
			bail!("Revision content exceeds {MAX_MANAGED_ARTIFACT_BYTES} bytes");
		}
		let active = self.check_expected(kind, name, expected)?;
		let rev = self.mint(kind, name, content, description, active);
		self.put_revision(kind, &rev)?;
		Ok(rev)
	}

	fn activate(&self, kind: Kind, name: &str, mut rev: Revision) -> Result<PathBuf> {
		rev.state = "active".into();
		self.put_revision(kind, &rev)?;
		let file = kind.file(name);
		self.fs.atomic_write(&file, rev.content.as_bytes())?;
		self.set_pointer(kind, name, &rev.id)?;
		self.prune(kind, name)?;
		Ok(self.root.join(file))
	}

	fn prune(&self, kind: Kind, name: &str) -> Result<()> {
		let (active, revisions) = self.list(kind, name)?;
		if revisions.len() <= MAX_REVISIONS_PER_ARTIFACT {
			return Ok(());
		}
		let excess = revisions.len() - MAX_REVISIONS_PER_ARTIFACT;
		for rev in revisions
			.iter()
			.filter(|r| Some(&r.id) != active.as_ref())
			.take(excess)
		{
			self
				.fs
				.remove(&Self::artifact_dir(kind, name).join(format!("{}.json", rev.id)))?;
		}
		Ok(())
	}

	pub fn resolve_target(
		&self,
		kind: Kind,
		name: &str,
		rev: Option<&str>,
		missing: &str,
	) -> Result<String> {
		if let Some(rev) = rev {
			return validate_revision_id(rev);
		}
		let (active, revisions) = self.list(kind, name)?;
		let drafts: Vec<_> = revisions
			.iter()
			.filter(|r| r.state == "draft" && Some(&r.id) != active.as_ref())
			.collect();
		drafts
			.last()
			.copied()
			.or(revisions.last())
			.map(|r| r.id.clone())
			.with_context(|| missing.to_owned())
	}

	pub fn promote(
		&self,
		kind: Kind,
		name: &str,
		rev_id: &str,
		disclose: bool,
	) -> Result<(PathBuf, bool)> {
		let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
		let rev = self
			.revision(kind, name, rev_id)?
			.with_context(|| format!("Revision {rev_id} for {} \"{name}\" not found", kind.label()))?;
		if rev.name != name {
			bail!("Revision {rev_id} does not belong to {} \"{name}\"", kind.label());
		}
		if rev.eval_status() == "failing" {
			bail!(
				"Revision {rev_id} for {} \"{name}\" has failing evaluations; it cannot be promoted. \
				 Fix the content and evaluate a new draft.",
				kind.label()
			);
		}
		let unevaluated = rev.evaluations.is_empty();
		if unevaluated && !disclose {
			bail!(
				"Revision {rev_id} for {} \"{name}\" has no evaluations; pass discloseUnevaluated to \
				 activate unevaluated content explicitly",
				kind.label()
			);
		}
		Ok((self.activate(kind, name, rev)?, unevaluated))
	}

	pub fn rollback(&self, kind: Kind, name: &str, rev_id: &str) -> Result<PathBuf> {
		let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
		let rev = self
			.revision(kind, name, rev_id)?
			.with_context(|| format!("Revision {rev_id} for {} \"{name}\" not found", kind.label()))?;
		if rev.state != "active" {
			bail!(
				"Cannot rollback {} \"{name}\" to revision {rev_id}: it was never active (state {}).",
				kind.label(),
				rev.state
			);
		}
		self.activate(kind, name, rev)
	}

	pub fn delete(&self, kind: Kind, name: &str) -> Result<()> {
		let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
		let file = kind.file(name);
		if !self.fs.exists(&file)? {
			bail!("Managed {} \"{name}\" does not exist.", kind.label());
		}
		self.fs.remove(&file)?;
		// The empty directory may be retained; every removal uses the jail owner.
		Ok(())
	}

	/// Names with a materialized managed file.
	pub fn managed_names(&self, kind: Kind) -> Result<Vec<String>> {
		let dir = Path::new(match kind {
			Kind::Skill => "managed-skills",
			Kind::Preset => "managed-presets",
		});
		if !self.fs.exists(dir)? {
			return Ok(Vec::new());
		}
		let mut names = Vec::new();
		for entry in self.fs.entries(dir, 10_000)? {
			if entry.is_symlink {
				continue;
			}
			let name = match kind {
				Kind::Skill if entry.is_directory => entry.name.clone(),
				Kind::Preset if entry.is_file => match entry.name.strip_suffix(".md") {
					Some(stem) => stem.to_owned(),
					None => continue,
				},
				_ => continue,
			};
			if self.fs.exists(&kind.file(&name))? {
				names.push(name);
			}
		}
		Ok(names)
	}

	pub fn read_managed(&self, kind: Kind, name: &str) -> Result<Option<String>> {
		self.read_text(&kind.file(name))
	}
}

/// No in-process evaluation executor is wired in the Rust host yet; TS gives
/// the same refusal when its runner seam is empty.
fn no_eval_executor(kind: Kind, name: &str) -> anyhow::Error {
	anyhow::anyhow!(
		"No evaluation executor is wired for {} \"{name}\" (owner patch: task-execution seam). The \
		 revision stays unevaluated — unevaluated content is never treated as passed.",
		kind.label()
	)
}

fn revision_rows(active: Option<&String>, revisions: &[Revision]) -> Value {
	Value::Array(
		revisions
			.iter()
			.map(|rev| {
				json!({
					"id": rev.id,
					"state": rev.state,
					"active": Some(&rev.id) == active,
					"evaluations": rev.evaluations.len(),
					"evalStatus": rev.eval_status(),
					"pinned": false,
					"parent": rev.parent,
				})
			})
			.collect(),
	)
}

/// Tool result shape shared by these tools: text plus structured details.
pub struct Outcome {
	pub text:     String,
	pub is_error: bool,
	pub details:  Value,
}

impl Outcome {
	fn ok(text: String, details: Value) -> Self {
		Self { text, is_error: false, details }
	}

	fn error(text: String, details: Value) -> Self {
		Self { text, is_error: true, details }
	}
}

// ---------------------------------------------------------------- skills

/// Authored (non-managed) skill roots; an authored skill always wins in
/// discovery, so managed skills may not claim its name.
fn authored_skill_dirs(cwd: &Path, agent_dir: &Path) -> Vec<PathBuf> {
	let mut dirs = vec![
		agent_dir.join("skills"),
		cwd.join(crate::commands::CONFIG_DIR_NAME).join("skills"),
		cwd.join(".claude").join("skills"),
	];
	if let Some(home) = std::env::var_os("HOME") {
		dirs.push(PathBuf::from(home).join(".claude").join("skills"));
	}
	dirs
}

pub fn is_name_claimed_by_authored_skill(cwd: &Path, agent_dir: &Path, name: &str) -> bool {
	authored_skill_dirs(cwd, agent_dir)
		.iter()
		.any(|dir| dir.join(name).join("SKILL.md").is_file())
}

/// Resolve the real task consumer through authored, bundled, then managed
/// roles. Permission/model fields are refused until this host can enforce them.
pub fn agent_definition(
	cwd: &Path,
	agent_dir: Option<&Path>,
	name: &str,
) -> Result<Option<String>> {
	if name.is_empty()
		|| name.len() > 128
		|| !name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
	{
		bail!("invalid agent name");
	}
	let agent_dir = agent_dir
		.map(Path::to_owned)
		.or_else(|| crate::config::agent_dir_from(None, |key| std::env::var_os(key)))
		.context("cannot resolve agent directory")?;
	for definition in authored_agents(cwd, &agent_dir)? {
		if definition.name == name {
			return checked_agent_body(name, &definition.frontmatter, definition.body).map(Some);
		}
	}
	if let Some(body) = crate::commands::agents::bundled_agent_body(name) {
		return Ok(Some(body));
	}
	if !agent_dir.exists() || sanitize_preset_name(name).is_err() {
		return Ok(None);
	}
	let store = Store { fs: JailedFs::open(&agent_dir)?, root: agent_dir };
	let Some(text) = store.read_managed(Kind::Preset, name)? else {
		return Ok(None);
	};
	let (fm, body) = parse_frontmatter(&text);
	checked_agent_body(name, &fm, body).map(Some)
}

fn checked_agent_body(name: &str, fm: &Mapping, body: String) -> Result<String> {
	for policy in ["tools", "spawns", "model"] {
		if fm.get(policy).is_some_and(|value| !value.is_null()) {
			bail!(
				"Agent {name} configures {policy}; native task policy binding is not yet available \
				 for custom agents."
			);
		}
	}
	if body.trim().is_empty() {
		bail!("Agent {name} has an empty system prompt");
	}
	Ok(body)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn persisted_revision_preserves_instruction_spaces_and_json_escapes() {
		let root = tempfile::tempdir().unwrap();
		let store =
			Store { fs: JailedFs::open(root.path()).unwrap(), root: root.path().to_owned() };
		let content = r#"---
name: precise-edit
description: Preserve  two spaces
---
Use  two spaces and "quoted" paths.
"#;
		let revision = store
			.draft(Kind::Skill, "precise-edit", content.into(), "Preserve  spacing".into(), None)
			.unwrap();
		let file = store
			.activate(Kind::Skill, "precise-edit", revision.clone())
			.unwrap();
		let decoded = store
			.revision(Kind::Skill, "precise-edit", &revision.id)
			.unwrap()
			.unwrap();
		assert_eq!(decoded.content, std::fs::read_to_string(file).unwrap());
		assert!(decoded.content.contains("Preserve  two spaces"));
		assert_eq!(decoded.description, "Preserve  spacing");
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillWrite {
	pub action:          String,
	pub name:            String,
	pub description:     Option<String>,
	pub body:            Option<String>,
	pub expected_active: Option<String>,
}

fn skill_content(
	store: &Store,
	name: &str,
	action: &str,
	description: Option<&str>,
	body: Option<&str>,
) -> Result<(String, String)> {
	let mut description = description.map(str::to_owned);
	let mut body = body.map(str::to_owned);
	if (action == "update" || action == "draft")
		&& (description.is_none() || body.is_none())
		&& let Some(current) = store.current_content(Kind::Skill, name)?
	{
		let (fm, current_body) = parse_frontmatter(&current);
		description.get_or_insert_with(|| mapping_str(&fm, "description").unwrap_or_default());
		body.get_or_insert(current_body);
	}
	let (Some(description), Some(body)) = (description, body) else {
		bail!(
			"\"{action}\" requires both \"description\" and \"body\" (or an existing skill to merge \
			 against)."
		);
	};
	let description = sanitize_description(&description);
	if description.is_empty() {
		bail!("Managed skill \"{name}\" needs a non-empty description.");
	}
	if body.trim().is_empty() {
		bail!("Managed skill \"{name}\" needs a non-empty body.");
	}
	let mut fm = Mapping::new();
	fm.insert("name".into(), name.into());
	fm.insert("description".into(), description.clone().into());
	let content = render(&fm, &body, false)?;
	if content.len() > MAX_MANAGED_ARTIFACT_BYTES {
		bail!(
			"Managed skill is {} bytes; the limit is {MAX_MANAGED_ARTIFACT_BYTES}. Trim the body or \
			 description.",
			content.len()
		);
	}
	Ok((content, description))
}

/// Create/update a managed skill and activate it immediately (unevaluated).
pub fn write_managed_skill(store: &Store, input: &SkillWrite) -> Result<PathBuf> {
	if !matches!(input.action.as_str(), "create" | "update") {
		bail!("managed skill write action must be create or update");
	}
	let name = sanitize_skill_name(&input.name)?;
	let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
	store.seed(Kind::Skill, &name)?;
	let exists = store.fs.exists(&Kind::Skill.file(&name))?;
	if input.action == "create" && exists {
		bail!("Managed skill \"{name}\" already exists. Use action \"update\" to change it.");
	}
	if input.action == "update" && !exists {
		bail!("Managed skill \"{name}\" does not exist. Use action \"create\" to add it.");
	}
	let (content, description) = skill_content(
		store,
		&name,
		&input.action,
		input.description.as_deref(),
		input.body.as_deref(),
	)?;
	let rev =
		store.draft(Kind::Skill, &name, content, description, input.expected_active.as_deref())?;
	store.activate(Kind::Skill, &name, rev)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManageSkillArgs {
	action:               String,
	name:                 String,
	description:          Option<String>,
	body:                 Option<String>,
	revision_id:          Option<String>,
	expected_active:      Option<String>,
	task:                 Option<String>,
	expected_outcome:     Option<String>,
	disclose_unevaluated: Option<bool>,
}

fn relative_managed(store: &Store, path: &Path, dir: &str) -> String {
	path
		.strip_prefix(store.root().join(dir))
		.map_or_else(|_| path.display().to_string(), |p| p.display().to_string())
}

pub fn manage_skill(cwd: &Path, arguments: &str) -> Result<Outcome> {
	let args: ManageSkillArgs = serde_json::from_str(arguments)?;
	let store = Store::open()?;
	let name = args.name.clone();
	match args.action.as_str() {
		"delete" => {
			let safe = sanitize_skill_name(&name)?;
			store.delete(Kind::Skill, &safe)?;
			Ok(Outcome::ok(
				format!("Deleted managed skill \"{name}\"."),
				json!({ "action": "delete", "name": name }),
			))
		},
		"list" => {
			let safe = sanitize_skill_name(&name)?;
			let (active, revisions) = store.list(Kind::Skill, &safe)?;
			let lines: Vec<String> = revisions
				.iter()
				.map(|rev| {
					let mut markers = vec![rev.state.clone()];
					if Some(&rev.id) == active.as_ref() {
						markers.push("active".into());
					}
					markers.push(format!("eval:{}", rev.eval_status()));
					format!(
						"- {} ({}): {} evaluation(s), parent {}",
						rev.id,
						markers.join(", "),
						rev.evaluations.len(),
						rev.parent.as_deref().unwrap_or("none")
					)
				})
				.collect();
			Ok(Outcome::ok(
				format!(
					"Managed skill \"{name}\": active {}, {} revision(s).\n{}\nDraft stages without \
					 touching the live file; evaluate records pass/fail (never promotes); promote \
					 activates (failed blocks, unevaluated needs discloseUnevaluated); rollback \
					 reactivates a prior active revision.",
					active.as_deref().unwrap_or("none"),
					revisions.len(),
					lines.join("\n")
				),
				json!({ "action": "list", "name": name, "active": active, "revisions": revision_rows(active.as_ref(), &revisions) }),
			))
		},
		"draft" => {
			if args.description.is_none() && args.body.is_none() {
				bail!(
					"\"draft\" requires \"description\" and/or \"body\" (omitted fields merge from \
					 current)."
				);
			}
			let safe = sanitize_skill_name(&name)?;
			if is_name_claimed_by_authored_skill(cwd, store.root(), &safe)
				&& store.active_revision(Kind::Skill, &safe)?.is_none()
			{
				return Ok(Outcome::error(
					format!(
						"Cannot draft managed skill \"{name}\": an authored skill of that name already \
						 exists, and managed skills cannot override authored ones. Choose a different \
						 name."
					),
					json!({ "action": "draft", "name": name, "shadowed": true }),
				));
			}
			let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
			store.seed(Kind::Skill, &safe)?;
			let (content, description) = skill_content(
				&store,
				&safe,
				"draft",
				args.description.as_deref(),
				args.body.as_deref(),
			)?;
			let rev = store.draft(
				Kind::Skill,
				&safe,
				content,
				description,
				args.expected_active.as_deref(),
			)?;
			Ok(Outcome::ok(
				format!(
					"Drafted managed skill \"{name}\" as revision {} (parent {}). The live file is \
					 unchanged; evaluate it, then promote to activate.",
					rev.id,
					rev.parent.as_deref().unwrap_or("none")
				),
				json!({ "action": "draft", "name": name, "revisionId": rev.id, "parent": rev.parent }),
			))
		},
		"evaluate" => {
			if args.task.is_none() || args.expected_outcome.is_none() {
				bail!("\"evaluate\" requires both \"task\" and \"expectedOutcome\".");
			}
			let safe = sanitize_skill_name(&name)?;
			let rev = store.resolve_target(
				Kind::Skill,
				&safe,
				args.revision_id.as_deref(),
				&format!("Managed skill \"{name}\" has no revisions. Draft one first."),
			)?;
			store
				.revision(Kind::Skill, &safe, &rev)?
				.with_context(|| format!("Revision {rev} for skill \"{safe}\" not found"))?;
			Err(no_eval_executor(Kind::Skill, &safe))
		},
		"promote" => {
			let safe = sanitize_skill_name(&name)?;
			let rev = store.resolve_target(
				Kind::Skill,
				&safe,
				args.revision_id.as_deref(),
				&format!("Managed skill \"{name}\" has no revisions. Draft one first."),
			)?;
			let (path, disclosed) =
				store.promote(Kind::Skill, &safe, &rev, args.disclose_unevaluated == Some(true))?;
			let suffix = if disclosed {
				" Activated unevaluated content explicitly (no evaluations recorded yet)."
			} else {
				""
			};
			Ok(Outcome::ok(
				format!(
					"Promoted managed skill \"{name}\" to revision {rev} (managed-skills/{}).{suffix}",
					relative_managed(&store, &path, "managed-skills")
				),
				json!({ "action": "promote", "name": name, "revisionId": rev, "disclosedUnevaluated": disclosed }),
			))
		},
		"rollback" => {
			let Some(rev) = args.revision_id.as_deref() else {
				bail!("\"rollback\" requires \"revisionId\". Use action \"list\" to see candidates.");
			};
			let rev = validate_revision_id(rev)?;
			let safe = sanitize_skill_name(&name)?;
			let path = store.rollback(Kind::Skill, &safe, &rev)?;
			Ok(Outcome::ok(
				format!(
					"Rolled back managed skill \"{name}\" to revision {rev} (managed-skills/{}).",
					relative_managed(&store, &path, "managed-skills")
				),
				json!({ "action": "rollback", "name": name, "revisionId": rev }),
			))
		},
		action @ ("create" | "update") => {
			if action == "create" && (args.description.is_none() || args.body.is_none()) {
				bail!("\"create\" requires both \"description\" and \"body\".");
			}
			if action == "update" && args.description.is_none() && args.body.is_none() {
				bail!(
					"\"update\" requires \"description\" and/or \"body\" (omitted fields merge from \
					 current)."
				);
			}
			if action == "create"
				&& is_name_claimed_by_authored_skill(cwd, store.root(), &sanitize_skill_name(&name)?)
			{
				return Ok(Outcome::error(
					format!(
						"Cannot create managed skill \"{name}\": an authored skill of that name already \
						 exists, and managed skills cannot override authored ones. Choose a different \
						 name."
					),
					json!({ "action": "create", "name": name, "shadowed": true }),
				));
			}
			let path = write_managed_skill(&store, &SkillWrite {
				action:          action.to_owned(),
				name:            name.clone(),
				description:     args.description,
				body:            args.body,
				expected_active: args.expected_active,
			})?;
			let verb = if action == "create" {
				"Created"
			} else {
				"Updated"
			};
			Ok(Outcome::ok(
				format!(
					"{verb} managed skill \"{name}\" (managed-skills/{}).",
					relative_managed(&store, &path, "managed-skills")
				),
				json!({ "action": action, "name": name, "evaluated": false, "status": "unevaluated-active" }),
			))
		},
		other => bail!("Unknown manage_skill action \"{other}\"."),
	}
}

// ---------------------------------------------------------------- presets

struct AuthoredAgent {
	name:        String,
	source:      String,
	description: String,
	frontmatter: Mapping,
	body:        String,
}

/// Shared, bounded discovery for preset inspection and the actual task
/// consumer.
fn authored_agents(cwd: &Path, agent_dir: &Path) -> Result<Vec<AuthoredAgent>> {
	let mut roots = vec![
		(cwd.join(crate::commands::CONFIG_DIR_NAME).join("agents"), "project"),
		(cwd.join(".claude/agents"), "project"),
		(agent_dir.join("agents"), "user"),
	];
	if let Some(home) = std::env::var_os("HOME") {
		roots.push((PathBuf::from(home).join(".claude/agents"), "user"));
	}
	let mut seen = BTreeSet::new();
	let mut agents = Vec::new();
	for (root, source) in roots {
		if !root.exists() {
			continue;
		}
		let fs = JailedFs::open(&root)?;
		let mut files = fs.entries(Path::new(""), 1024)?;
		files.sort_by(|a, b| a.name.cmp(&b.name));
		for file in files {
			if !file.is_file || file.is_symlink || !file.name.ends_with(".md") {
				continue;
			}
			let text = String::from_utf8(fs.read(Path::new(&file.name), MAX_MANAGED_ARTIFACT_BYTES)?)?;
			let (frontmatter, body) = parse_frontmatter(&text);
			let name = mapping_str(&frontmatter, "name")
				.unwrap_or_else(|| file.name.trim_end_matches(".md").into());
			if seen.insert(name.clone()) {
				let description = mapping_str(&frontmatter, "description").unwrap_or_default();
				agents.push(AuthoredAgent {
					name,
					source: source.into(),
					description,
					frontmatter,
					body,
				});
			}
		}
	}
	Ok(agents)
}

fn discovered_agents(cwd: &Path, agent_dir: &Path) -> Result<Vec<(String, String, String)>> {
	let mut agents: Vec<_> = authored_agents(cwd, agent_dir)?
		.into_iter()
		.map(|agent| (agent.name, agent.source, agent.description))
		.collect();
	let mut seen: BTreeSet<_> = agents.iter().map(|(name, ..)| name.clone()).collect();
	for name in crate::commands::agents::bundled_agent_names() {
		if seen.insert(name.clone()) {
			agents.push((name, "bundled".into(), String::new()));
		}
	}
	Ok(agents)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetsArgs {
	action:               String,
	name:                 Option<String>,
	description:          Option<String>,
	system_prompt:        Option<String>,
	brief:                Option<String>,
	tools:                Option<Vec<String>>,
	spawns:               Option<String>,
	model:                Option<String>,
	revision_id:          Option<String>,
	expected_active:      Option<String>,
	task:                 Option<String>,
	expected_outcome:     Option<String>,
	disclose_unevaluated: Option<bool>,
}

fn preset_content(
	store: &Store,
	name: &str,
	args: &PresetsArgs,
	update: bool,
) -> Result<(String, String)> {
	let mut fm = Mapping::new();
	let mut body = String::new();
	if update && let Some(current) = store.current_content(Kind::Preset, name)? {
		let parsed = parse_frontmatter(&current);
		fm = parsed.0;
		body = parsed.1;
	}
	fm.insert("name".into(), name.into());
	if let Some(description) = &args.description {
		fm.insert("description".into(), description.trim().into());
	}
	if let Some(prompt) = &args.system_prompt {
		body = prompt.clone();
	}
	if let Some(tools) = &args.tools {
		fm.insert("tools".into(), serde_yaml_ng::to_value(tools)?);
	}
	if let Some(spawns) = &args.spawns {
		fm.insert("spawns".into(), spawns.as_str().into());
	}
	if let Some(model) = &args.model {
		fm.insert("model".into(), model.as_str().into());
	}
	let description = mapping_str(&fm, "description").unwrap_or_default();
	if description.trim().is_empty() {
		bail!("Managed preset \"{name}\" needs a non-empty description.");
	}
	if body.trim().is_empty() {
		bail!("Managed preset \"{name}\" needs a non-empty systemPrompt.");
	}
	// Ordered frontmatter: name, description, then optional policy fields.
	let mut ordered = Mapping::new();
	for key in ["name", "description", "tools", "spawns", "model"] {
		if let Some(value) = fm.remove(key) {
			ordered.insert(key.into(), value);
		}
	}
	ordered.extend(fm);
	let content = render(&ordered, &body, true)?;
	if content.len() > MAX_MANAGED_ARTIFACT_BYTES {
		bail!("Revision content exceeds {MAX_MANAGED_ARTIFACT_BYTES} bytes");
	}
	Ok((content, description))
}

pub fn presets(cwd: &Path, arguments: &str) -> Result<Outcome> {
	let args: PresetsArgs = serde_json::from_str(arguments)?;
	let store = Store::open()?;
	let require_name = || -> Result<String> {
		match &args.name {
			Some(name) => sanitize_preset_name(name),
			None => bail!("\"{}\" requires \"name\".", args.action),
		}
	};
	let agents = discovered_agents(cwd, store.root())?;
	match args.action.as_str() {
		"list" => {
			let mut rows: Vec<String> = agents
				.iter()
				.map(|(name, source, _)| format!("- {name} ({source})"))
				.collect();
			let mut managed = Vec::new();
			for name in store.managed_names(Kind::Preset)? {
				if agents.iter().any(|(n, ..)| *n == name) {
					continue;
				}
				let (active, revisions) = store
					.list(Kind::Preset, &name)
					.unwrap_or((None, Vec::new()));
				rows.push(format!(
					"- {name} (managed, active {}, {} revision(s))",
					active.as_deref().unwrap_or("none"),
					revisions.len()
				));
				managed.push(json!({ "name": name, "source": "managed", "active": active, "revisions": revision_rows(active.as_ref(), &revisions) }));
			}
			Ok(Outcome::ok(
				format!(
					"Agent presets ({}):\n{}\nDraft stages without touching the live file; evaluate \
					 records pass/fail; promote activates (failed blocks, unevaluated needs \
					 discloseUnevaluated); rollback reactivates a prior active revision.",
					rows.len(),
					rows.join("\n")
				),
				json!({ "action": "list", "count": rows.len(), "managed": managed }),
			))
		},
		"inspect" => {
			let name = require_name()?;
			let known = agents.iter().find(|(n, ..)| *n == name);
			let managed = store
				.read_managed(Kind::Preset, &name)?
				.map(|text| parse_frontmatter(&text).0);
			let (active, revisions) = store
				.list(Kind::Preset, &name)
				.unwrap_or((None, Vec::new()));
			if known.is_none() && managed.is_none() && revisions.is_empty() {
				return Ok(Outcome::error(
					format!("Preset \"{name}\" not found."),
					json!({ "action": "inspect", "name": name }),
				));
			}
			let description = managed
				.as_ref()
				.and_then(|fm| mapping_str(fm, "description"))
				.or_else(|| known.map(|(_, _, d)| d.clone()))
				.unwrap_or_else(|| "(no live definition)".into());
			let lines: Vec<String> = revisions
				.iter()
				.map(|rev| {
					format!(
						"- {} ({}{}, eval:{}): {} evaluation(s)",
						rev.id,
						rev.state,
						if Some(&rev.id) == active.as_ref() {
							", active"
						} else {
							""
						},
						rev.eval_status(),
						rev.evaluations.len()
					)
				})
				.collect();
			let source = if managed.is_some() {
				Some("managed".to_owned())
			} else {
				known.map(|(_, s, _)| s.clone())
			};
			Ok(Outcome::ok(
				format!(
					"Preset \"{name}\": {description}\nActive revision: {}; {} revision(s).\n{}",
					active.as_deref().unwrap_or("none"),
					revisions.len(),
					lines.join("\n")
				),
				json!({ "action": "inspect", "name": name, "active": active, "source": source, "revisions": revision_rows(active.as_ref(), &revisions) }),
			))
		},
		"create" => {
			if args.system_prompt.is_none() {
				if args.brief.as_deref().is_none_or(|b| b.trim().is_empty()) {
					bail!("\"create\" requires \"systemPrompt\" or \"brief\".");
				}
				bail!(
					"Architect-generated presets need a model session; pass \"systemPrompt\", \
					 \"description\" and \"name\" explicitly in this host."
				);
			}
			if args.description.is_none() {
				bail!("\"create\" with \"systemPrompt\" requires \"description\".");
			}
			let Some(raw) = args.name.as_deref() else {
				bail!("\"create\" with \"systemPrompt\" requires \"name\".");
			};
			let name = sanitize_preset_name(raw)?;
			if agents.iter().any(|(n, ..)| *n == name) {
				return Ok(Outcome::error(
					format!(
						"Cannot create preset \"{name}\": that name is already claimed by an authored, \
						 discovered, or bundled agent. Managed presets cannot override them. Choose a \
						 different name."
					),
					json!({ "action": "create", "name": name, "collision": true }),
				));
			}
			let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
			if store.fs.exists(&Kind::Preset.file(&name))? {
				bail!("Managed preset \"{name}\" already exists. Use action \"update\" to change it.");
			}
			let (content, description) = preset_content(&store, &name, &args, false)?;
			let rev = store.draft(
				Kind::Preset,
				&name,
				content,
				description,
				args.expected_active.as_deref(),
			)?;
			let file = store.activate(Kind::Preset, &name, rev)?;
			Ok(Outcome::ok(
				format!("Created managed preset \"{name}\" ({}).", file.display()),
				json!({ "action": "create", "name": name, "evaluated": false, "status": "unevaluated-active", "discoveryInvalidated": true }),
			))
		},
		"update" => {
			let name = require_name()?;
			if args.description.is_none()
				&& args.system_prompt.is_none()
				&& args.tools.is_none()
				&& args.spawns.is_none()
				&& args.model.is_none()
			{
				bail!(
					"\"update\" needs at least one of \"description\", \"systemPrompt\", \"tools\", \
					 \"spawns\", \"model\" (omitted policy fields merge from current)."
				);
			}
			if agents.iter().any(|(n, s, _)| *n == name && s != "bundled") {
				return Ok(Outcome::error(
					format!(
						"Cannot update preset \"{name}\": it is claimed by an authored or discovered \
						 agent, which managed presets cannot override."
					),
					json!({ "action": "update", "name": name, "collision": true }),
				));
			}
			let _guard = TRANSACTION.lock().unwrap_or_else(|e| e.into_inner());
			store.seed(Kind::Preset, &name)?;
			if store.current_content(Kind::Preset, &name)?.is_none() {
				bail!("Managed preset \"{name}\" does not exist. Use action \"create\" to add it.");
			}
			let (content, description) = preset_content(&store, &name, &args, true)?;
			let rev = store.draft(
				Kind::Preset,
				&name,
				content,
				description,
				args.expected_active.as_deref(),
			)?;
			let file = store.activate(Kind::Preset, &name, rev)?;
			Ok(Outcome::ok(
				format!("Updated managed preset \"{name}\" ({}).", file.display()),
				json!({ "action": "update", "name": name, "evaluated": false, "status": "unevaluated-active", "discoveryInvalidated": true }),
			))
		},
		"evaluate" => {
			let name = require_name()?;
			if args.task.is_none() || args.expected_outcome.is_none() {
				bail!("\"evaluate\" requires both \"task\" and \"expectedOutcome\".");
			}
			store.resolve_target(
				Kind::Preset,
				&name,
				args.revision_id.as_deref(),
				&format!("Preset \"{name}\" has no revisions. Create one first."),
			)?;
			Err(no_eval_executor(Kind::Preset, &name))
		},
		"promote" => {
			let name = require_name()?;
			let rev = store.resolve_target(
				Kind::Preset,
				&name,
				args.revision_id.as_deref(),
				&format!("Preset \"{name}\" has no revisions. Create one first."),
			)?;
			let (file, disclosed) =
				store.promote(Kind::Preset, &name, &rev, args.disclose_unevaluated == Some(true))?;
			let suffix = if disclosed {
				" Activated unevaluated content explicitly (no evaluations recorded yet)."
			} else {
				""
			};
			Ok(Outcome::ok(
				format!(
					"Promoted managed preset \"{name}\" to revision {rev} ({}).{suffix}",
					file.display()
				),
				json!({ "action": "promote", "name": name, "revisionId": rev, "disclosedUnevaluated": disclosed, "discoveryInvalidated": true }),
			))
		},
		"rollback" => {
			let name = require_name()?;
			let Some(rev) = args.revision_id.as_deref() else {
				bail!(
					"\"rollback\" requires \"revisionId\". Use action \"inspect\" to see candidates."
				);
			};
			let rev = validate_revision_id(rev)?;
			let file = store.rollback(Kind::Preset, &name, &rev)?;
			Ok(Outcome::ok(
				format!(
					"Rolled back managed preset \"{name}\" to revision {rev} ({}).",
					file.display()
				),
				json!({ "action": "rollback", "name": name, "revisionId": rev, "discoveryInvalidated": true }),
			))
		},
		"delete" => {
			let name = require_name()?;
			store.delete(Kind::Preset, &name)?;
			Ok(Outcome::ok(
				format!("Deleted managed preset \"{name}\"."),
				json!({ "action": "delete", "name": name, "discoveryInvalidated": true }),
			))
		},
		other => bail!("Unknown presets action \"{other}\"."),
	}
}
