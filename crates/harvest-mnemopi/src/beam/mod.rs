//! BEAM memory: working + episodic tiers over one SQLite connection.

pub mod consolidate;
pub mod recall;
pub mod store;

use std::{
	path::{Path, PathBuf},
	sync::Arc,
};

use rusqlite::Connection;

use crate::{
	Result, config, db,
	providers::{Embedder, Llm},
	schema,
};

#[derive(Debug, Clone)]
pub struct BeamConfig {
	pub working_memory_limit:     i64,
	pub working_memory_ttl_hours: i64,
	pub recency_halflife_hours:   f64,
	pub vec_weight:               f64,
	pub fts_weight:               f64,
	pub importance_weight:        f64,
	pub max_episode_chars:        i64,
	pub proactive_linking:        bool,
}

impl Default for BeamConfig {
	fn default() -> Self {
		Self {
			working_memory_limit:     config::working_memory_max_items(),
			working_memory_ttl_hours: config::working_memory_ttl_hours(),
			recency_halflife_hours:   config::recency_halflife_hours(),
			vec_weight:               0.5,
			fts_weight:               0.3,
			importance_weight:        0.2,
			max_episode_chars:        config::max_episode_chars(),
			proactive_linking:        false,
		}
	}
}

#[derive(Default, Clone)]
pub struct BeamOptions {
	pub session_id:           Option<String>,
	/// `None` → in-memory database.
	pub db_path:              Option<PathBuf>,
	pub author_id:            Option<String>,
	pub author_type:          Option<String>,
	pub channel_id:           Option<String>,
	pub config:               Option<BeamConfig>,
	pub embedder:             Option<Arc<dyn Embedder>>,
	pub llm:                  Option<Arc<dyn Llm>>,
	/// False keeps a diagnostics/open consumer from wiping old embedding rows.
	pub reconcile_embeddings: Option<bool>,
}

pub struct BeamMemory {
	pub db:          Connection,
	pub db_path:     Option<PathBuf>,
	pub session_id:  String,
	pub author_id:   Option<String>,
	pub author_type: Option<String>,
	pub channel_id:  String,
	pub config:      BeamConfig,
	pub embedder:    Option<Arc<dyn Embedder>>,
	pub llm:         Option<Arc<dyn Llm>>,
}

impl BeamMemory {
	pub fn open(options: BeamOptions) -> Result<Self> {
		let session_id = options.session_id.unwrap_or_else(|| "default".into());
		crate::limits::check_text(&session_id, 1024, "session id")?;
		let config = options.config.unwrap_or_default();
		if !(0..=1_000_000).contains(&config.working_memory_limit)
			|| !(0..=8_760_000).contains(&config.working_memory_ttl_hours)
			|| !(1..=crate::limits::MAX_CONTENT_BYTES as i64).contains(&config.max_episode_chars)
			|| !config.recency_halflife_hours.is_finite()
			|| config.recency_halflife_hours <= 0.0
			|| [config.vec_weight, config.fts_weight, config.importance_weight]
				.iter()
				.any(|v| !v.is_finite() || *v < 0.0)
		{
			return Err(crate::MnemopiError::Invalid(
				"invalid memory bounds or scoring configuration".into(),
			));
		}
		let conn = db::open_database(options.db_path.as_deref())?;
		db::transaction(&conn, || schema::init_all(&conn))?;
		let beam = Self {
			db: conn,
			db_path: options.db_path,
			channel_id: options.channel_id.unwrap_or_else(|| session_id.clone()),
			session_id,
			author_id: options.author_id,
			author_type: options.author_type,
			config,
			embedder: options.embedder,
			llm: options.llm,
		};
		if options.reconcile_embeddings != Some(false) {
			store::reconcile_embedding_model(&beam)?;
		}
		Ok(beam)
	}

	pub fn open_path(session_id: &str, path: &Path) -> Result<Self> {
		Self::open(BeamOptions {
			session_id: Some(session_id.into()),
			db_path: Some(path.to_path_buf()),
			..Default::default()
		})
	}

	/// Clone of this handle's identity scoped to another session (shares nothing
	/// but config/providers).
	pub(crate) fn with_session<T>(
		&mut self,
		session: &str,
		f: impl FnOnce(&Self) -> Result<T>,
	) -> Result<T> {
		let prev_session = std::mem::replace(&mut self.session_id, session.to_string());
		let prev_channel = std::mem::replace(&mut self.channel_id, session.to_string());
		let out = f(self);
		self.session_id = prev_session;
		self.channel_id = prev_channel;
		out
	}

	pub fn source_session(&self) -> &str {
		if self.session_id.is_empty() {
			"default"
		} else {
			&self.session_id
		}
	}
}
