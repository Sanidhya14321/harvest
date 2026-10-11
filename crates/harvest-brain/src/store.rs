//! Recoverable generated-record transactions. Unowned files are never deleted.

use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, RootLock, content_hash};
use serde::{Deserialize, Serialize};

use crate::{
	Candidate, Document, EvidenceKind, EvidenceLedger, GRACE_SECONDS, Grade, MAX_ARCHIVE_BYTES,
	MAX_GENERATED_BYTES, MAX_GENERATED_RECORDS, MAX_STATE_BYTES, Metadata, Ownership, RecordKind,
	RecordStatus, SCHEMA_VERSION, Scope, grade, record_fingerprint, validate_content,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
	pub id:          String,
	pub fingerprint: String,
	pub path:        String,
	pub file_hash:   String,
	pub created_at:  u64,
	pub status:      RecordStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
	pub restoration_id:    String,
	pub receipt:           Receipt,
	pub archive_path:      String,
	pub reason:            String,
	pub archived_at:       u64,
	pub bytes:             usize,
	#[serde(default)]
	pub evidence_snapshot: Vec<crate::EvidenceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainState {
	pub version:  u32,
	pub ledger:   EvidenceLedger,
	pub records:  BTreeMap<String, Receipt>,
	pub archives: BTreeMap<String, ArchiveEntry>,
	pub leases:   BTreeSet<String>,
}

impl Default for BrainState {
	fn default() -> Self {
		Self {
			version:  SCHEMA_VERSION,
			ledger:   EvidenceLedger::default(),
			records:  BTreeMap::new(),
			archives: BTreeMap::new(),
			leases:   BTreeSet::new(),
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Transaction {
	Put { receipt: Receipt, expected_hash: Option<String>, contents: String },
	Archive { entry: ArchiveEntry, contents: String },
	Restore { entry: ArchiveEntry, contents: String },
	Purge { entry: ArchiveEntry },
}

#[derive(Debug)]
pub struct BrainStore {
	pub scope:        Scope,
	pub fs:           JailedFs,
	brain:            PathBuf,
	skills:           PathBuf,
	state_root:       PathBuf,
	selected_sources: Option<Vec<PathBuf>>,
}

#[derive(Debug, Serialize)]
pub struct ProposalResult {
	pub id:     String,
	pub grade:  Grade,
	pub action: String,
}

#[derive(Debug, Serialize)]
pub struct OrganizationEntry {
	pub id:             String,
	pub path:           String,
	pub grade:          Option<Grade>,
	pub action:         String,
	pub reason:         String,
	pub restoration_id: Option<String>,
}

impl BrainStore {
	pub fn project(root: &Path) -> Result<Self> {
		Ok(Self {
			scope:            Scope::Project,
			fs:               JailedFs::open(root)?,
			brain:            ".harvest/brain".into(),
			skills:           ".harvest/skills/learned".into(),
			state_root:       ".harvest/brain-state".into(),
			selected_sources: None,
		})
	}

	pub fn user(agent_dir: &Path) -> Result<Self> {
		Ok(Self {
			scope:            Scope::User,
			fs:               JailedFs::open(agent_dir)?,
			brain:            "brain".into(),
			skills:           "skills/learned".into(),
			state_root:       "brain-state".into(),
			selected_sources: None,
		})
	}

	pub fn sources(&self) -> Vec<PathBuf> {
		if let Some(sources) = &self.selected_sources {
			return sources.clone();
		}
		match self.scope {
			Scope::Project => {
				vec![self.brain.clone(), ".harvest/skills".into(), ".agents/skills".into()]
			},
			Scope::User => vec![self.brain.clone(), "skills".into(), "managed-skills".into()],
		}
	}

	/// Select an explicitly authorized subset of discovery roots. Reads still
	/// use the same pinned workspace capability and durable ownership receipts.
	pub fn select_sources(&mut self, sources: Vec<PathBuf>) -> Result<()> {
		if sources.len() > 32 {
			bail!("too many brain discovery roots");
		}
		self.selected_sources = Some(sources);
		Ok(())
	}

	pub fn snapshot_lock(&self) -> Result<Option<RootLock>> {
		self.fs.lock_existing(&self.state_root.join("write.lock"))
	}

	pub fn has_pending_recovery(&self) -> Result<bool> {
		self.fs.exists(&self.transaction_path())
	}

	fn state_path(&self) -> PathBuf {
		self.state_root.join("state.json")
	}

	fn transaction_path(&self) -> PathBuf {
		self.state_root.join("transaction.json")
	}

	pub fn state(&self) -> Result<BrainState> {
		let Some(bytes) = self.fs.read_optional(&self.state_path(), MAX_STATE_BYTES)? else {
			return Ok(BrainState::default());
		};
		let state: BrainState =
			serde_json::from_slice(&bytes).context("brain state is corrupt; it was preserved")?;
		if state.version != SCHEMA_VERSION || state.ledger.version != SCHEMA_VERSION {
			bail!("unsupported brain state schema");
		}
		Ok(state)
	}

	fn save_state(&self, state: &BrainState) -> Result<()> {
		let bytes = serde_json::to_vec_pretty(state)?;
		if bytes.len() > MAX_STATE_BYTES {
			bail!("brain evidence/state quota reached; admission paused");
		}
		self.fs.atomic_write(&self.state_path(), &bytes)
	}

	fn writer(&self) -> Result<(RootLock, BrainState)> {
		let lock = self.fs.lock(&self.state_root.join("write.lock"))?;
		let mut state = self.state()?;
		if let Some(bytes) = self
			.fs
			.read_optional(&self.transaction_path(), MAX_STATE_BYTES)?
		{
			let transaction: Transaction = serde_json::from_slice(&bytes)
				.context("brain recovery journal is corrupt; it was preserved")?;
			self.apply(&mut state, &transaction)?;
		}
		Ok((lock, state))
	}

	pub fn recover(&self) -> Result<()> {
		let (_lock, _state) = self.writer()?;
		Ok(())
	}

	pub fn record_committed(&self, event: crate::EvidenceEvent) -> Result<()> {
		let (_lock, mut state) = self.writer()?;
		state.ledger.record_committed(event)?;
		self.save_state(&state)
	}

	pub fn lease(&self, id: &str, active: bool) -> Result<()> {
		let (_lock, mut state) = self.writer()?;
		if active {
			state.leases.insert(id.to_owned());
		} else {
			state.leases.remove(id);
		}
		self.save_state(&state)
	}

	fn generated_path(&self, candidate: &Candidate, id: &str) -> String {
		let path = if candidate.kind == RecordKind::Skill {
			self.skills.join(id).join("SKILL.md")
		} else {
			self.brain.join("generated").join(id).with_extension("md")
		};
		path.to_string_lossy().into_owned()
	}

	fn managed_path(&self, path: &str) -> Result<()> {
		let path = self.fs.relative(Path::new(path))?;
		if !(path.starts_with(self.brain.join("generated")) || path.starts_with(&self.skills)) {
			bail!("receipt does not name generated content");
		}
		if path.extension().is_none_or(|extension| extension != "md") {
			bail!("receipt does not name Markdown");
		}
		Ok(())
	}

	pub fn propose(&self, candidate: &Candidate, now: u64) -> Result<ProposalResult> {
		let (_lock, mut state) = self.writer()?;
		let mut proposed = candidate.clone();
		let fingerprint = record_fingerprint(candidate)?;
		let id = format!("learned-{fingerprint}");
		let previous = state.records.get(&id).cloned();
		if let Some(receipt) = &previous {
			let bytes = self.fs.read(Path::new(&receipt.path), MAX_STATE_BYTES)?;
			if content_hash(&bytes) != receipt.file_hash {
				bail!("generated record was manually edited; it is protected");
			}
			let document = parse_markdown(self.scope, &receipt.path, std::str::from_utf8(&bytes)?)?;
			proposed.evidence.extend(document.metadata.evidence);
		}
		let grading = grade(&proposed, self.scope, &state.ledger)?;
		if grading.total < 50.0 {
			return Ok(ProposalResult { id, grade: grading, action: "discarded".into() });
		}
		let path = previous
			.as_ref()
			.map_or_else(|| self.generated_path(candidate, &id), |receipt| receipt.path.clone());
		let created_at = previous.as_ref().map_or(now, |receipt| receipt.created_at);
		let metadata = Metadata {
			schema_version: Some(SCHEMA_VERSION),
			id: id.clone(),
			title: proposed.title.clone(),
			name: id.clone(),
			description: proposed.title.clone(),
			kind: proposed.kind,
			ownership: Ownership::Generated,
			status: grading.status,
			conditions: proposed.conditions.clone(),
			evidence: proposed.evidence.clone(),
			depends_on: proposed.depends_on.clone(),
			relates_to: proposed.relates_to.clone(),
			created_at,
			..Metadata::default()
		};
		let contents = render_markdown(&metadata, &proposed.content)?;
		self.check_capacity(&state, previous.as_ref(), contents.len())?;
		let receipt = Receipt {
			id: id.clone(),
			fingerprint,
			path,
			file_hash: content_hash(contents.as_bytes()),
			created_at,
			status: grading.status,
		};
		let transaction = Transaction::Put {
			receipt,
			expected_hash: previous.as_ref().map(|receipt| receipt.file_hash.clone()),
			contents,
		};
		self.transact(&mut state, &transaction)?;
		Ok(ProposalResult {
			id,
			grade: grading,
			action: if previous.is_some() {
				"merged"
			} else {
				"created"
			}
			.into(),
		})
	}

	fn check_capacity(
		&self,
		state: &BrainState,
		previous: Option<&Receipt>,
		bytes: usize,
	) -> Result<()> {
		if state.records.len() >= MAX_GENERATED_RECORDS && previous.is_none() {
			bail!("generated record quota reached; admission paused");
		}
		let mut total = bytes;
		for receipt in state
			.records
			.values()
			.filter(|receipt| previous.is_none_or(|old| old.id != receipt.id))
		{
			if let Some(existing) = self
				.fs
				.read_optional(Path::new(&receipt.path), MAX_GENERATED_BYTES)?
			{
				total = total.saturating_add(existing.len());
			}
		}
		if total > MAX_GENERATED_BYTES {
			bail!("generated content quota reached; admission paused");
		}
		Ok(())
	}

	fn transact(&self, state: &mut BrainState, transaction: &Transaction) -> Result<()> {
		let mut projected = state.clone();
		match transaction {
			Transaction::Put { receipt, .. } => {
				projected
					.records
					.insert(receipt.id.clone(), receipt.clone());
			},
			Transaction::Archive { entry, .. } => {
				projected.records.remove(&entry.receipt.id);
				projected
					.archives
					.insert(entry.restoration_id.clone(), entry.clone());
			},
			Transaction::Restore { entry, .. } => {
				projected.archives.remove(&entry.restoration_id);
				projected
					.records
					.insert(entry.receipt.id.clone(), entry.receipt.clone());
			},
			Transaction::Purge { entry } => {
				projected.archives.remove(&entry.restoration_id);
			},
		}
		if serde_json::to_vec_pretty(&projected)?.len() > MAX_STATE_BYTES {
			bail!("brain evidence/state quota reached; maintenance paused");
		}
		let journal = serde_json::to_vec(transaction)?;
		if journal.len() > MAX_STATE_BYTES {
			bail!("brain transaction exceeds recovery quota");
		}
		self.fs.atomic_write(&self.transaction_path(), &journal)?;
		self.apply(state, transaction)
	}

	fn expect_content(&self, path: &Path, expected: Option<&str>, after: &str) -> Result<bool> {
		let actual = self
			.fs
			.read_optional(path, MAX_STATE_BYTES)?
			.map(|bytes| content_hash(&bytes));
		if actual.as_deref() == Some(after) {
			return Ok(true);
		}
		if actual.as_deref() != expected {
			bail!("content changed during a brain transaction; preserved for recovery");
		}
		Ok(false)
	}

	fn apply(&self, state: &mut BrainState, transaction: &Transaction) -> Result<()> {
		match transaction {
			Transaction::Put { receipt, expected_hash, contents } => {
				self.managed_path(&receipt.path)?;
				if content_hash(contents.as_bytes()) != receipt.file_hash {
					bail!("transaction content hash mismatch");
				}
				if !self.expect_content(
					Path::new(&receipt.path),
					expected_hash.as_deref(),
					&receipt.file_hash,
				)? {
					self
						.fs
						.atomic_write(Path::new(&receipt.path), contents.as_bytes())?;
				}
				state.records.insert(receipt.id.clone(), receipt.clone());
			},
			Transaction::Archive { entry, contents } => {
				self.managed_path(&entry.receipt.path)?;
				self.archive_path(&entry.archive_path)?;
				if content_hash(contents.as_bytes()) != entry.receipt.file_hash {
					bail!("archive hash mismatch");
				}
				let source = self
					.fs
					.read_optional(Path::new(&entry.receipt.path), MAX_STATE_BYTES)?;
				if source
					.as_ref()
					.is_some_and(|bytes| content_hash(bytes) != entry.receipt.file_hash)
				{
					bail!("record changed before archival; preserved");
				}
				if !self.expect_content(
					Path::new(&entry.archive_path),
					None,
					&entry.receipt.file_hash,
				)? {
					self
						.fs
						.atomic_write(Path::new(&entry.archive_path), contents.as_bytes())?;
				}
				if source.is_some() {
					self.fs.remove(Path::new(&entry.receipt.path))?;
				}
				state.records.remove(&entry.receipt.id);
				state
					.archives
					.insert(entry.restoration_id.clone(), entry.clone());
			},
			Transaction::Restore { entry, contents } => {
				self.managed_path(&entry.receipt.path)?;
				self.archive_path(&entry.archive_path)?;
				if content_hash(contents.as_bytes()) != entry.receipt.file_hash {
					bail!("restoration hash mismatch");
				}
				if !self.expect_content(
					Path::new(&entry.receipt.path),
					None,
					&entry.receipt.file_hash,
				)? {
					self
						.fs
						.atomic_write(Path::new(&entry.receipt.path), contents.as_bytes())?;
				}
				state
					.records
					.insert(entry.receipt.id.clone(), entry.receipt.clone());
				state.archives.remove(&entry.restoration_id);
				// A crash after state publication can safely repeat this removal.
				if self.fs.exists(Path::new(&entry.archive_path))? {
					self.fs.remove(Path::new(&entry.archive_path))?;
				}
			},
			Transaction::Purge { entry } => {
				self.archive_path(&entry.archive_path)?;
				if let Some(bytes) = self
					.fs
					.read_optional(Path::new(&entry.archive_path), MAX_STATE_BYTES)?
				{
					if content_hash(&bytes) != entry.receipt.file_hash {
						bail!("archive changed; purge refused");
					}
					self.fs.remove(Path::new(&entry.archive_path))?;
				}
				state.archives.remove(&entry.restoration_id);
			},
		}
		self.save_state(state)?;
		self.fs.remove(&self.transaction_path())?;
		Ok(())
	}

	fn archive_path(&self, path: &str) -> Result<()> {
		let path = self.fs.relative(Path::new(path))?;
		if !path.starts_with(self.state_root.join("archive")) {
			bail!("archive path escapes archive storage");
		}
		Ok(())
	}

	pub fn organize(&self, now: u64, dry_run: bool) -> Result<Vec<OrganizationEntry>> {
		if !dry_run && self.has_pending_recovery()? {
			self.recover()?;
		}
		let mut index = crate::BrainIndex::default();
		index.refresh(&[self])?;
		if !index.warnings.is_empty() {
			bail!("dependency inventory is incomplete: {}", index.warnings.join("; "));
		}
		self.organize_with_dependencies(now, dry_run, &index.required_dependencies())
	}

	pub fn organize_with_dependencies(
		&self,
		now: u64,
		dry_run: bool,
		required: &BTreeSet<String>,
	) -> Result<Vec<OrganizationEntry>> {
		if !dry_run && self.has_pending_recovery()? {
			self.recover()?;
		}
		let mut inventory = crate::BrainIndex::default();
		inventory.refresh(&[self])?;
		if !inventory.warnings.is_empty() {
			bail!("dependency inventory is incomplete: {}", inventory.warnings.join("; "));
		}
		// Dry runs cannot recover/mutate a journal or create a lock directory.
		let mut state;
		let _lock = if dry_run {
			if self.fs.exists(&self.transaction_path())? {
				bail!("pending recovery; run organize without --dry-run first");
			}
			state = self.state()?;
			None
		} else {
			let (lock, recovered) = self.writer()?;
			state = recovered;
			Some(lock)
		};
		let mut reports = Vec::new();
		for document in inventory
			.documents()
			.iter()
			.filter(|document| !state.records.contains_key(&document.metadata.id))
		{
			reports.push(OrganizationEntry {
				id:             document.metadata.id.clone(),
				path:           document.path.clone(),
				grade:          grade(&document_candidate(document), self.scope, &state.ledger).ok(),
				action:         "protected".into(),
				reason:         "authored or lacks trustworthy ownership/provenance".into(),
				restoration_id: None,
			});
		}
		let mut current_required = required.clone();
		// Recheck generated dependencies after acquiring the writer lock. Another
		// admission may have committed after the caller built its inventory.
		for receipt in state.records.values() {
			if let Some(bytes) = self
				.fs
				.read_optional(Path::new(&receipt.path), crate::MAX_FILE_BYTES)?
			{
				let document = parse_markdown(self.scope, &receipt.path, std::str::from_utf8(&bytes)?)?;
				for target in &document.metadata.depends_on {
					current_required.insert(
						if target.starts_with("project:") || target.starts_with("user:") {
							target.clone()
						} else {
							format!("{}:{target}", self.scope.prefix())
						},
					);
				}
			}
		}
		let receipts: Vec<_> = state.records.values().cloned().collect();
		for receipt in receipts {
			self.managed_path(&receipt.path)?;
			let Some(bytes) = self
				.fs
				.read_optional(Path::new(&receipt.path), MAX_STATE_BYTES)?
			else {
				reports.push(report(
					&receipt,
					None,
					"protected",
					"record is missing; receipt retained",
					None,
				));
				continue;
			};
			if content_hash(&bytes) != receipt.file_hash {
				reports.push(report(
					&receipt,
					None,
					"protected",
					"manual edits changed the content revision",
					None,
				));
				continue;
			}
			let document = parse_markdown(self.scope, &receipt.path, std::str::from_utf8(&bytes)?)?;
			let dependency = current_required.contains(&document.scoped_id());
			let protected = document.metadata.ownership != Ownership::Generated
				|| document.metadata.pinned
				|| document.metadata.safety
				|| state.leases.contains(&receipt.id)
				|| dependency;
			let candidate = document_candidate(&document);
			let grading = grade(&candidate, self.scope, &state.ledger).ok();
			if protected {
				reports.push(report(
					&receipt,
					grading,
					"protected",
					"authored, pinned, safety, leased or required dependency",
					None,
				));
				continue;
			}
			let related: Vec<_> = state
				.ledger
				.events
				.values()
				.filter(|event| {
					event.record_hash.as_deref() == Some(&receipt.fingerprint)
						&& document.metadata.conditions.is_subset(&event.conditions)
				})
				.collect();
			let presentations: BTreeSet<_> = related
				.iter()
				.filter(|event| event.kind == EvidenceKind::Presentation)
				.map(|event| (&event.session_id, &event.task_id))
				.collect();
			let sessions: BTreeSet<_> = presentations
				.iter()
				.map(|(session, _)| session.as_str())
				.collect();
			let useful = related
				.iter()
				.any(|event| event.kind == EvidenceKind::BeneficialApplication);
			let contradicted = related.iter().any(|event| {
				matches!(event.kind, EvidenceKind::RejectedApplication | EvidenceKind::Supersession)
			});
			let unsafe_content = validate_content(&document.content, self.scope).is_err();
			let stale = now.saturating_sub(receipt.created_at) >= GRACE_SECONDS
				&& presentations.len() >= 5
				&& sessions.len() >= 3
				&& !useful;
			if !(unsafe_content || contradicted || document.metadata.superseded || stale) {
				reports.push(report(
					&receipt,
					grading,
					"retained",
					"retirement evidence is insufficient",
					None,
				));
				continue;
			}
			let reason = if unsafe_content {
				"unsafe generated content"
			} else if contradicted {
				"explicit rejection or proven supersession"
			} else if document.metadata.superseded {
				"superseded generated record"
			} else {
				"no confirmed benefit after grace and relevant opportunities"
			};
			let archive_bytes: usize = state.archives.values().map(|entry| entry.bytes).sum();
			if archive_bytes.saturating_add(bytes.len()) > MAX_ARCHIVE_BYTES {
				reports.push(report(
					&receipt,
					grading,
					"paused",
					"archive quota reached; explicit purge is required",
					None,
				));
				continue;
			}
			let restoration_id =
				content_hash(format!("{}:{now}:{}", receipt.id, state.archives.len()).as_bytes());
			let archive_path = self
				.state_root
				.join("archive")
				.join(&restoration_id)
				.with_extension("md")
				.to_string_lossy()
				.into_owned();
			let entry = ArchiveEntry {
				restoration_id: restoration_id.clone(),
				receipt: receipt.clone(),
				archive_path,
				reason: reason.into(),
				archived_at: now,
				bytes: bytes.len(),
				evidence_snapshot: related.iter().map(|event| (*event).clone()).collect(),
			};
			if !dry_run {
				self.transact(&mut state, &Transaction::Archive {
					entry,
					contents: String::from_utf8(bytes)?,
				})?;
			}
			reports.push(report(
				&receipt,
				grading,
				if dry_run { "would_archive" } else { "archived" },
				reason,
				Some(restoration_id),
			));
		}
		Ok(reports)
	}

	pub fn restore(&self, restoration_id: &str) -> Result<()> {
		let (_lock, mut state) = self.writer()?;
		let entry = state
			.archives
			.get(restoration_id)
			.context("restoration ID does not exist")?
			.clone();
		if state.records.contains_key(&entry.receipt.id) {
			bail!("record already exists; restoration refused");
		}
		let bytes = self
			.fs
			.read(Path::new(&entry.archive_path), MAX_STATE_BYTES)?;
		self.check_capacity(&state, None, bytes.len())?;
		self
			.transact(&mut state, &Transaction::Restore { entry, contents: String::from_utf8(bytes)? })
	}

	/// Explicit operation only; automatic organization never calls purge.
	pub fn purge(&self, restoration_id: &str) -> Result<()> {
		let (_lock, mut state) = self.writer()?;
		let entry = state
			.archives
			.get(restoration_id)
			.context("restoration ID does not exist")?
			.clone();
		self.transact(&mut state, &Transaction::Purge { entry })
	}
}

fn report(
	receipt: &Receipt,
	grade: Option<Grade>,
	action: &str,
	reason: &str,
	restoration_id: Option<String>,
) -> OrganizationEntry {
	OrganizationEntry {
		id: receipt.id.clone(),
		path: receipt.path.clone(),
		grade,
		action: action.into(),
		reason: reason.into(),
		restoration_id,
	}
}

pub fn document_candidate(document: &Document) -> Candidate {
	Candidate {
		title:      document.metadata.title.clone(),
		kind:       document.metadata.kind,
		content:    document.content.clone(),
		conditions: document.metadata.conditions.clone(),
		evidence:   document.metadata.evidence.clone(),
		depends_on: document.metadata.depends_on.clone(),
		relates_to: document.metadata.relates_to.clone(),
	}
}

pub fn render_markdown(metadata: &Metadata, body: &str) -> Result<String> {
	Ok(format!(
		"---\n{}---\n\n{}\n",
		serde_yaml_ng::to_string(metadata)?,
		crate::normalize_content(body)
	))
}

pub fn parse_markdown(scope: Scope, path: &str, text: &str) -> Result<Document> {
	let normalized = text.replace("\r\n", "\n");
	let (mut metadata, body) = if let Some(rest) = normalized.strip_prefix("---\n") {
		let Some(end) = rest
			.find("\n---\n")
			.or_else(|| rest.strip_suffix("\n---").map(str::len))
		else {
			bail!("unterminated Markdown frontmatter");
		};
		let header = &rest[..end];
		let metadata: Metadata =
			serde_yaml_ng::from_str(header).context("invalid Markdown frontmatter")?;
		(metadata, rest.get(end + 5..).unwrap_or_default().trim().to_owned())
	} else {
		(Metadata::default(), normalized.trim().to_owned())
	};
	if metadata
		.schema_version
		.is_some_and(|version| version != SCHEMA_VERSION)
	{
		bail!("unsupported brain record schema");
	}
	if metadata.id.is_empty() {
		path.clone_into(&mut metadata.id);
	}
	if metadata.id.len() > 512 || metadata.id.chars().any(char::is_control) {
		bail!("invalid record ID");
	}
	if metadata.title.is_empty() {
		metadata.title = if metadata.name.is_empty() {
			metadata.id.clone()
		} else {
			metadata.name.clone()
		};
	}
	if metadata.title.len() > 512
		|| metadata.conditions.len() > 32
		|| metadata.evidence.len() > 128
		|| metadata.depends_on.len()
			+ metadata.relates_to.len()
			+ metadata.supersedes.len()
			+ metadata.contradicts.len()
			> 64
	{
		bail!("brain frontmatter exceeds metadata limits");
	}
	if body.len() > crate::MAX_FILE_BYTES {
		bail!("brain file exceeds the indexing limit");
	}
	Ok(Document { scope, path: path.to_owned(), metadata, content: body })
}
