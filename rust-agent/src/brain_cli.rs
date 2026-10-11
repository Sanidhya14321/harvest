//! Native brain maintenance. Evidence recording is a trusted runtime API, never
//! a model tool.
use std::{
	collections::BTreeSet,
	env,
	io::{self, Write},
	path::PathBuf,
	time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use harvest_brain::{BrainIndex, BrainStore, RetrievalContext, Scope};
use serde::Serialize;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Selection {
	Project,
	User,
	Both,
}

#[derive(Debug, Args)]
pub(crate) struct BrainArgs {
	#[arg(long, default_value = ".", global = true)]
	workspace: PathBuf,
	#[arg(long, global = true)]
	agent_dir: Option<PathBuf>,
	#[arg(long, value_enum, default_value = "both", global = true)]
	scope:     Selection,
	#[command(subcommand)]
	action:    Action,
}

#[derive(Debug, Args)]
pub(crate) struct OrganizationArgs {
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	agent_dir: Option<PathBuf>,
	#[arg(long, value_enum, default_value = "both")]
	scope:     Selection,
	#[arg(long)]
	dry_run:   bool,
	#[arg(long)]
	json:      bool,
}

pub(crate) fn organize_command(args: OrganizationArgs) -> Result<u8> {
	execute(BrainArgs {
		workspace: args.workspace,
		agent_dir: args.agent_dir,
		scope:     args.scope,
		action:    Action::Organize { dry_run: args.dry_run, json: args.json },
	})
}

#[derive(Debug, Subcommand)]
enum Action {
	#[command(alias = "orgnize-skills", alias = "organize-skills")]
	Organize {
		#[arg(long)]
		dry_run: bool,
		#[arg(long)]
		json:    bool,
	},
	Retrieve {
		query: String,
		#[arg(long)]
		tag:   Vec<String>,
		#[arg(long)]
		json:  bool,
	},
	Restore {
		restoration_id: String,
	},
	/// Permanently delete a previously archived record.
	Purge {
		restoration_id: String,
	},
	/// Record an explicit lesson supplied by the user.
	Learn {
		#[arg(long)]
		title: String,
		#[arg(long)]
		file:  PathBuf,
		#[arg(long, default_value = "preference", value_parser = parse_kind)]
		kind:  harvest_brain::RecordKind,
		#[arg(long)]
		tag:   Vec<String>,
	},
	Recover,
	Status,
}

#[derive(Debug, Serialize)]
struct ScopedReport<T> {
	scope:   Scope,
	entries: T,
}

pub(crate) fn execute(args: BrainArgs) -> Result<u8> {
	let project = BrainStore::project(&args.workspace)?;
	let agent_dir = crate::config::agent_dir_from(args.agent_dir, |key| env::var_os(key));
	let user = match agent_dir {
		Some(path) if path.is_dir() => Some(BrainStore::user(&path)?),
		_ => None,
	};
	if matches!(args.scope, Selection::User) && user.is_none() {
		bail!("user agent directory does not exist; supply --agent-dir");
	}
	let mut stores = Vec::new();
	if !matches!(args.scope, Selection::User) {
		stores.push(&project);
	}
	if !matches!(args.scope, Selection::Project)
		&& let Some(user) = &user
	{
		stores.push(user);
	}
	let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
	let is_purge = matches!(&args.action, Action::Purge { .. });
	match args.action {
		Action::Organize { dry_run, json } => {
			if !dry_run {
				for store in &stores {
					store.recover()?;
				}
			}
			let mut index = BrainIndex::default();
			// Always inspect both existing scopes for dependencies, even for single-scope
			// maintenance.
			let mut all = vec![&project];
			if let Some(user) = &user {
				all.push(user);
			}
			index.refresh(&all)?;
			if !index.warnings.is_empty() {
				bail!("organization cannot prove dependency safety: {}", index.warnings.join("; "));
			}
			let required = index.required_dependencies();
			let mut reports = Vec::new();
			for store in stores {
				reports.push(ScopedReport {
					scope:   store.scope,
					entries: store.organize_with_dependencies(now, dry_run, &required)?,
				});
			}
			if json {
				output_json(&reports)?;
			} else {
				for report in reports {
					for entry in report.entries {
						writeln!(
							io::stdout(),
							"{}:{} {}: {}{}",
							report.scope.prefix(),
							entry.id,
							entry.action,
							entry.reason,
							entry
								.restoration_id
								.map_or(String::new(), |id| format!(" (restore {id})"))
						)?;
					}
				}
			}
		},
		Action::Retrieve { query, tag, json } => {
			let mut index = BrainIndex::default();
			index.refresh(&stores)?;
			let context = RetrievalContext {
				tags:      tag.into_iter().collect(),
				scopes:    stores
					.iter()
					.map(|store| store.scope)
					.collect::<BTreeSet<_>>(),
				max_chars: None,
			};
			if json {
				output_json(&index.retrieve(&query, &context))?;
			} else {
				writeln!(io::stdout(), "{}", index.context_block(&query, &context))?;
			}
		},
		Action::Restore { restoration_id } | Action::Purge { restoration_id } => {
			let mut owner = None;
			for store in stores {
				if store.state()?.archives.contains_key(&restoration_id) {
					if owner.is_some() {
						bail!("ambiguous restoration ID; select one scope");
					}
					owner = Some(store);
				}
			}
			let store = owner.context("restoration ID does not exist in the selected scopes")?;
			if is_purge {
				store.purge(&restoration_id)?;
			} else {
				store.restore(&restoration_id)?;
			}
		},
		Action::Learn { title, file, kind, tag } => {
			if stores.len() != 1 || matches!(args.scope, Selection::Both) {
				bail!("learning requires explicit --scope project or --scope user");
			}
			let store = stores[0];
			let bytes = harvest_runtime::read_bounded(
				std::fs::File::open(file)?,
				harvest_brain::MAX_CANDIDATE_BYTES,
			)?;
			let mut candidate = harvest_brain::Candidate {
				title,
				kind,
				content: String::from_utf8(bytes)?,
				conditions: tag.into_iter().collect(),
				evidence: BTreeSet::new(),
				depends_on: Vec::new(),
				relates_to: Vec::new(),
			};
			harvest_brain::validate_content(&candidate.content, store.scope)?;
			harvest_brain::validate_content(&candidate.title, store.scope)?;
			let hash = harvest_brain::record_fingerprint(&candidate)?;
			let session_id = format!(
				"cli-{}-{}",
				std::process::id(),
				SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
			);
			let id = format!("{session_id}:{hash}");
			store.record_committed(harvest_brain::EvidenceEvent {
				id: id.clone(),
				session_id,
				task_id: hash.clone(),
				project_id: harvest_brain::repository_identity(&project.fs),
				kind: harvest_brain::EvidenceKind::UserPreference,
				timestamp: now,
				conditions: candidate.conditions.clone(),
				record_hash: Some(hash),
				cross_project: store.scope == Scope::User,
			})?;
			candidate.evidence.insert(id);
			output_json(&store.propose(&candidate, now)?)?;
		},
		Action::Recover => {
			for store in stores {
				store.recover()?;
			}
		},
		Action::Status => {
			for store in stores {
				let state = store.state()?;
				output_json(&ScopedReport { scope: store.scope, entries: state })?;
			}
		},
	}
	Ok(0)
}

fn output_json(value: &impl Serialize) -> Result<()> {
	let mut output = io::stdout().lock();
	serde_json::to_writer_pretty(&mut output, value)?;
	writeln!(output)?;
	Ok(())
}

fn parse_kind(value: &str) -> Result<harvest_brain::RecordKind, String> {
	serde_json::from_value(serde_json::Value::String(value.to_owned()))
		.map_err(|error| error.to_string())
}
