//! A bounded view of canonical skill Markdown through the existing brain index.
//! No global registry or alternate persisted representation is maintained.
use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobMatcher};
use harvest_brain::{BrainIndex, BrainStore, Document, RecordStatus};
use serde::Serialize;

use crate::{settings::Settings, tools::ToolOutput};

#[derive(Debug)]
pub(crate) struct SkillNotFound(pub String);
impl std::fmt::Display for SkillNotFound {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Skill {} was not found in the enabled catalog", self.0)
	}
}
impl std::error::Error for SkillNotFound {}

#[derive(Clone)]
struct Source {
	root:    PathBuf,
	path:    PathBuf,
	project: bool,
	managed: bool,
	label:   &'static str,
}

struct Skill {
	resource_root: PathBuf,
	name:          String,
	description:   String,
	path:          PathBuf,
	source:        &'static str,
	hidden:        bool,
	body:          String,
}

fn patterns(settings: &Settings, key: &str) -> Result<Vec<GlobMatcher>> {
	let Some(serde_json::Value::Array(values)) = settings.get(key) else {
		return Ok(Vec::new());
	};
	if values.len() > 128 {
		bail!("skill pattern count exceeds the discovery limit");
	}
	values
		.into_iter()
		.map(|value| {
			let text = value.as_str().context("skill patterns must be strings")?;
			if text.len() > 512 {
				bail!("skill pattern exceeds byte limit");
			}
			Ok(Glob::new(text)?.compile_matcher())
		})
		.collect()
}

fn sources(cwd: &Path, agent_dir: &Path, settings: &Settings) -> Result<Vec<Source>> {
	let mut sources = Vec::new();
	for (key, path, label) in [
		("skills.enablePiProject", ".harvest/skills", "project-native"),
		("skills.enableAgentsProject", ".agents/skills", "project-agents"),
		("skills.enableClaudeProject", ".claude/skills", "project-claude"),
	] {
		if settings.bool(key) {
			sources.push(Source {
				root: cwd.into(),
				path: path.into(),
				project: true,
				managed: false,
				label,
			});
		}
	}
	if settings.bool("skills.enablePiUser") {
		sources.push(Source {
			root:    agent_dir.into(),
			path:    "skills".into(),
			project: false,
			managed: false,
			label:   "user-native",
		});
	}
	if let Some(home) = std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.map(PathBuf::from)
	{
		for (key, path, label) in [
			("skills.enableAgentsUser", ".agents/skills", "user-agents"),
			("skills.enableCodexUser", ".codex/skills", "user-codex"),
			("skills.enableClaudeUser", ".claude/skills", "user-claude"),
		] {
			if settings.bool(key) {
				sources.push(Source {
					root: home.clone(),
					path: path.into(),
					project: false,
					managed: false,
					label,
				});
			}
		}
	}
	if let Some(serde_json::Value::Array(custom)) = settings.get("skills.customDirectories") {
		if custom.len() > 32 {
			bail!("too many custom skill directories");
		}
		for path in custom {
			let raw = path
				.as_str()
				.context("custom skill directories must be strings")?;
			if raw.is_empty() || raw.len() > 4096 {
				bail!("invalid custom skill directory");
			}
			let root = crate::commands::paths::resolve_absolute_base(Some(raw))
				.unwrap_or_else(|| cwd.join(raw));
			sources.push(Source {
				root,
				path: PathBuf::new(),
				project: false,
				managed: false,
				label: "custom",
			});
		}
	}
	// Managed sources always come after every enabled authored source.
	sources.push(Source {
		root:    agent_dir.into(),
		path:    "managed-skills".into(),
		project: false,
		managed: true,
		label:   "managed",
	});
	Ok(sources)
}

fn skill_document(document: &Document) -> bool {
	Path::new(&document.path)
		.file_name()
		.is_some_and(|name| name == "SKILL.md")
}

/// Generic reference retrieval must not also expose shadowed/hidden skills.
/// The catalog below owns skill precedence while reusing the same index engine.
pub fn remove_skill_documents(index: &mut BrainIndex) -> Result<()> {
	index.replace_documents(
		index
			.documents()
			.iter()
			.filter(|document| {
				let path = Path::new(&document.path);
				!skill_document(document)
					&& ![
						".harvest/skills",
						".agents/skills",
						".claude/skills",
						".codex/skills",
						"skills",
						"managed-skills",
					]
					.iter()
					.any(|root| path.starts_with(root))
			})
			.cloned()
			.collect(),
	)
}

fn snapshot(cwd: &Path, agent_dir: Option<&Path>, settings: &Settings) -> Result<Vec<Skill>> {
	if !settings.bool("skills.enabled") {
		return Ok(Vec::new());
	}
	let _guard = super::TRANSACTION
		.lock()
		.map_err(|_| anyhow::anyhow!("managed artifact transaction is unavailable"))?;
	let agent_dir =
		crate::config::agent_dir_from(agent_dir.map(Path::to_owned), |key| std::env::var_os(key))
			.context("cannot resolve skill agent directory")?;
	let ignored = patterns(settings, "skills.ignoredSkills")?;
	let included = patterns(settings, "skills.includeSkills")?;
	let disabled: BTreeSet<String> = settings
		.get("disabledExtensions")
		.and_then(|value| value.as_array().cloned())
		.unwrap_or_default()
		.into_iter()
		.filter_map(|value| {
			value
				.as_str()
				.and_then(|text| text.strip_prefix("skill:"))
				.map(str::to_owned)
		})
		.collect();
	let mut seen = BTreeSet::new();
	let mut selected = Vec::new();
	let mut bytes = 0_usize;
	for source in sources(cwd, &agent_dir, settings)? {
		if !source.root.exists() {
			continue;
		}
		let source_fs = harvest_runtime::JailedFs::open(&source.root)?;
		let mut store = if source.project {
			BrainStore::project(&source.root)?
		} else {
			BrainStore::user(&source.root)?
		};
		store.select_sources(vec![source.path.clone()])?;
		let mut index = BrainIndex::default();
		index.refresh(&[&store])?;
		if !index.warnings.is_empty() {
			bail!("skill source {} could not be completely indexed", source.label);
		}
		let managed = if source.managed {
			Some(super::Store {
				fs:   harvest_runtime::JailedFs::open(&source.root)?,
				root: source.root.clone(),
			})
		} else {
			None
		};
		for document in index.documents() {
			if !skill_document(document) || document.metadata.status != RecordStatus::Active {
				continue;
			}
			let name = if document.metadata.name.is_empty() {
				Path::new(&document.path)
					.parent()
					.and_then(Path::file_name)
					.and_then(|name| name.to_str())
					.unwrap_or_default()
					.to_owned()
			} else {
				document.metadata.name.clone()
			};
			if name.is_empty()
				|| name.len() > 128
				|| name == "."
				|| name == ".."
				|| !name.chars().all(|character| {
					character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
				}) {
				continue;
			}
			if source.managed && !super::sanitize_skill_name(&name).is_ok_and(|safe| safe == name) {
				continue;
			}
			if let Some(managed) = &managed {
				let materialized = managed
					.read_managed(super::Kind::Skill, &name)?
					.context("managed skill name does not match its materialization")?;
				let (_, body) = super::parse_frontmatter(&materialized)?;
				if body.trim() != document.content {
					bail!("managed skill changed during discovery; retry with a consistent snapshot");
				}
			}
			if disabled.contains(&name)
				|| ignored.iter().any(|pattern| pattern.is_match(&name))
				|| (!included.is_empty() && !included.iter().any(|pattern| pattern.is_match(&name)))
			{
				continue;
			}
			let description: String = super::sanitize_description(&document.metadata.description)
				.chars()
				.take(512)
				.collect();
			if description.is_empty() {
				continue;
			}
			if !seen.insert(name.clone()) {
				continue;
			}
			bytes = bytes
				.checked_add(document.content.len())
				.context("skill catalog size overflow")?;
			if selected.len() >= harvest_brain::MAX_FILES || bytes > harvest_brain::MAX_INDEX_BYTES {
				bail!("skill catalog exceeds aggregate capacity");
			}
			selected.push(Skill {
				name,
				description,
				path: source_fs.resource_path(&source.root.join(&document.path))?,
				resource_root: source_fs.root().to_owned(),
				source: source.label,
				hidden: document.metadata.hide || document.metadata.disable_model_invocation,
				body: document.content.clone(),
			});
		}
	}
	selected.sort_by(|a, b| {
		a.name
			.to_lowercase()
			.cmp(&b.name.to_lowercase())
			.then(a.name.cmp(&b.name))
			.then(a.path.cmp(&b.path))
	});
	Ok(selected)
}

#[derive(Serialize)]
struct CatalogRow {
	name:        String,
	description: String,
	uri:         String,
}

/// Fresh canonical discovery also makes tool-driven creates/updates visible
/// before the next provider request without a second skills registry.
pub fn context(cwd: &Path, agent_dir: Option<&Path>, settings: &Settings) -> Result<String> {
	let skills = snapshot(cwd, agent_dir, settings)?;
	let mut rows = Vec::new();
	let mut bytes = 512_usize;
	for skill in skills.into_iter().filter(|skill| !skill.hidden) {
		let estimate = (skill.name.len() + skill.description.len()).saturating_mul(6) + 64;
		if bytes + estimate > harvest_brain::MAX_CONTEXT_CHARS {
			break;
		}
		bytes += estimate;
		rows.push(CatalogRow {
			uri:         format!("skill://{}", skill.name),
			name:        skill.name,
			description: skill.description,
		});
	}
	if rows.is_empty() {
		return Ok(String::new());
	}
	Ok(handlebars::Handlebars::new().render_template(
		include_str!("../prompts/skills-catalog.md"),
		&serde_json::json!({"skills":rows}),
	)?)
}

/// Explicit user command discovery shares the same canonical enabled catalog.
/// Hidden/model-disabled skills remain available to an explicit user
/// invocation.
pub(crate) fn commands(
	cwd: &Path,
	directory: Option<&Path>,
	settings: &Settings,
	prefix: &str,
) -> Result<Vec<(String, String)>> {
	if !settings.bool("skills.enabled") || !settings.bool("skills.enableSkillCommands") {
		return Ok(Vec::new());
	}
	let name_prefix = prefix.strip_prefix("/skill:").unwrap_or("");
	Ok(snapshot(cwd, directory, settings)?
		.into_iter()
		.filter(|skill| skill.name.starts_with(name_prefix))
		.take(100)
		.map(|skill| (format!("/skill:{}", skill.name), skill.description))
		.collect())
}

/// Managed writers use the same enabled, validated authored selection as the
/// model catalog instead of guessing a name from a directory basename.
pub fn is_authored_name_claimed(
	cwd: &Path,
	agent_dir: &Path,
	settings: &Settings,
	name: &str,
) -> Result<bool> {
	Ok(snapshot(cwd, Some(agent_dir), settings)?
		.into_iter()
		.any(|skill| skill.name == name && skill.source != "managed"))
}

/// Return the already validated snapshot body; never check one path then
/// reopen an untrusted lexical path outside its source capability.
pub fn read(
	cwd: &Path,
	agent_dir: Option<&Path>,
	settings: &Settings,
	uri: &str,
) -> Result<ToolOutput> {
	let resource = uri
		.strip_prefix("skill://")
		.context("expected a skill:// resource")?;
	if resource.len() > 4096 || resource.contains(['\\', '%', '?', '#', '\0']) {
		bail!("invalid skill resource path");
	}
	let (name, relative) = resource.split_once('/').unwrap_or((resource, ""));
	if name.is_empty() || name == "." || name == ".." {
		bail!("native skill resources require an exact skill name");
	}
	if !relative.is_empty()
		&& relative
			.split('/')
			.any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
	{
		bail!("skill companion path must be relative without traversal");
	}
	let skill = snapshot(cwd, agent_dir, settings)?
		.into_iter()
		.find(|skill| skill.name == name)
		.ok_or_else(|| anyhow::Error::new(SkillNotFound(name.to_owned())))?;
	if !relative.is_empty() {
		// The selected skill grants read access only to its own directory.
		// Reuse the capability filesystem, pin the canonical target and never
		// issue numbered-line provenance for these virtual resources.
		let base = skill
			.path
			.parent()
			.context("skill has no resource directory")?;
		let fs = harvest_runtime::JailedFs::open(&skill.resource_root)?;
		let base = fs.resource_path(base)?;
		let target = fs.resource_path(&base.join(relative))?;
		if !target.starts_with(&base) {
			bail!("skill companion resolves outside its directory");
		}
		let kind = fs.file_type(&target)?;
		let content = if kind.is_dir() {
			let entries = fs.entries(&target, 1024)?;
			let mut output = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_TOOL_BYTES);
			let entries = entries
				.iter()
				.map(|entry| {
					serde_json::json!({
						"name":entry.name,"isDirectory":entry.is_directory,"isFile":entry.is_file,
						"isSymlink":entry.is_symlink,"size":entry.size,
					})
				})
				.collect::<Vec<_>>();
			serde_json::to_writer(&mut output, &entries)?;
			String::from_utf8(output.into_inner())?
		} else if kind.is_file() {
			String::from_utf8(fs.read(&target, harvest_runtime::MAX_TOOL_BYTES)?)
				.context("skill companion is not UTF-8 text")?
		} else {
			bail!("skill resource must be a regular file or directory");
		};
		return Ok(ToolOutput {
			content,
			is_error: false,
			details: Some(
				serde_json::json!({"name":skill.name,"path":target,"source":skill.source,"virtualResource":true,"editProvenance":false}),
			),
		});
	}
	if skill.body.len() > harvest_runtime::MAX_TOOL_BYTES {
		bail!("skill body exceeds tool response limit");
	}
	Ok(ToolOutput {
		is_error: false,
		content:  skill.body,
		details:  Some(
			serde_json::json!({"name":skill.name,"description":skill.description,"path":skill.path,"source":skill.source,"hidden":skill.hidden}),
		),
	})
}
