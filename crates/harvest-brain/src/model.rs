//! Shared record types and bounded policy. Scope is assigned by the host root.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_FILES: usize = 512;
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PAGES: usize = 4096;
pub const MAX_SECTIONS: usize = 128;
pub const MAX_CONTEXT_CHARS: usize = 8000;
pub const MAX_CANDIDATE_BYTES: usize = 8192;
pub const MAX_GENERATED_RECORDS: usize = 256;
pub const MAX_GENERATED_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ARCHIVE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_STATE_BYTES: usize = 4 * 1024 * 1024;
pub const GRACE_SECONDS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
	Project,
	User,
}

impl Scope {
	pub const fn prefix(self) -> &'static str {
		match self {
			Self::Project => "project",
			Self::User => "user",
		}
	}
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
	Preference,
	Fact,
	#[default]
	Pattern,
	Antipattern,
	Skill,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
	#[default]
	Authored,
	Generated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
	#[default]
	Active,
	Candidate,
	Archived,
	Quarantined,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
	pub schema_version: Option<u32>,
	pub id:             String,
	pub title:          String,
	pub name:           String,
	pub description:    String,
	pub kind:           RecordKind,
	pub ownership:      Ownership,
	pub status:         RecordStatus,
	pub pinned:         bool,
	pub safety:         bool,
	pub superseded:     bool,
	pub conditions:     BTreeSet<String>,
	pub evidence:       BTreeSet<String>,
	#[serde(deserialize_with = "string_list")]
	pub depends_on:     Vec<String>,
	#[serde(deserialize_with = "string_list")]
	pub relates_to:     Vec<String>,
	#[serde(deserialize_with = "string_list")]
	pub supersedes:     Vec<String>,
	#[serde(deserialize_with = "string_list")]
	pub contradicts:    Vec<String>,
	pub created_at:     u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
	pub title:      String,
	pub kind:       RecordKind,
	pub content:    String,
	pub conditions: BTreeSet<String>,
	pub evidence:   BTreeSet<String>,
	#[serde(default)]
	#[serde(deserialize_with = "string_list")]
	pub depends_on: Vec<String>,
	#[serde(default)]
	#[serde(deserialize_with = "string_list")]
	pub relates_to: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Document {
	pub scope:    Scope,
	pub path:     String,
	pub metadata: Metadata,
	pub content:  String,
}

impl Document {
	pub fn scoped_id(&self) -> String {
		format!("{}:{}", self.scope.prefix(), self.metadata.id)
	}
}

#[derive(Debug, Clone, Default)]
pub struct RetrievalContext {
	pub tags:      BTreeSet<String>,
	pub scopes:    BTreeSet<Scope>,
	pub max_chars: Option<usize>,
}

impl RetrievalContext {
	pub fn allows(&self, document: &Document) -> bool {
		(self.scopes.is_empty() || self.scopes.contains(&document.scope))
			&& document.metadata.conditions.is_subset(&self.tags)
			&& document.metadata.status == RecordStatus::Active
			&& !document.metadata.superseded
	}
}

fn string_list<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
	#[derive(Deserialize)]
	#[serde(untagged)]
	enum OneOrMany {
		One(String),
		Many(Vec<String>),
		Null,
	}
	Ok(match OneOrMany::deserialize(deserializer)? {
		OneOrMany::One(value) => vec![value],
		OneOrMany::Many(values) => values,
		OneOrMany::Null => Vec::new(),
	})
}
