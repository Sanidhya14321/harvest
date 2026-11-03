//! Shared credential store: the `auth_credentials` table in `agent.db`
//! (`packages/ai/src/auth/sqlite-credential-store.ts`). Rows are read and
//! written in the TS wire format so both applications share credentials.
//! The schema and its migrations stay owned by TS; writes require a database
//! TS has already initialized. OAuth refresh is not ported: expired OAuth
//! rows are skipped.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value};

/// A deserialized credential row.
#[derive(Clone)]
pub enum Credential {
	ApiKey { key: String, login: bool },
	OAuth(Map<String, Value>),
}

impl Credential {
	fn oauth_str<'a>(&'a self, field: &str) -> Option<&'a str> {
		match self {
			Self::OAuth(map) => map.get(field).and_then(Value::as_str),
			Self::ApiKey { .. } => None,
		}
	}

	/// Unexpired OAuth access token (`Date.now() < expires`).
	fn live_access(&self) -> Option<&str> {
		let Self::OAuth(map) = self else { return None };
		let expires = map.get("expires").and_then(Value::as_f64)?;
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0.0, |d| d.as_millis() as f64);
		if now < expires { map.get("access").and_then(Value::as_str) } else { None }
	}
}

pub struct StoredCredential {
	pub id:         i64,
	pub provider:   String,
	pub credential: Credential,
}

/// Identity of one stored OAuth account (`OAuthAccountSummary`).
pub struct OAuthAccount {
	pub position:      usize,
	pub credential_id: i64,
	pub email:         Option<String>,
	pub account_id:    Option<String>,
	pub project_id:    Option<String>,
	pub enterprise:    Option<String>,
	pub org_name:      Option<String>,
	pub org_id:        Option<String>,
}

/// `getAgentDbPath`: `<agent dir>/agent.db`.
pub fn agent_db_path(agent_dir: &Path) -> PathBuf {
	agent_dir.join("agent.db")
}

pub struct AuthStorage {
	db: Option<Connection>,
}

fn deserialize(kind: &str, data: &str) -> Option<Credential> {
	let Value::Object(map) = serde_json::from_str::<Value>(data).ok()? else { return None };
	match kind {
		"api_key" => Some(Credential::ApiKey {
			key:   map.get("key")?.as_str()?.to_owned(),
			login: map.get("source").and_then(Value::as_str) == Some("login"),
		}),
		"oauth" => Some(Credential::OAuth(map)),
		_ => None,
	}
}

const SQLITE_NOW_EPOCH: &str = "CAST(strftime('%s','now') AS INTEGER)";

impl AuthStorage {
	/// Open the store under `agent_dir`. A missing database or table is an
	/// empty store.
	pub fn open(agent_dir: &Path) -> Result<Self> {
		let path = agent_db_path(agent_dir);
		if !path.is_file() {
			return Ok(Self { db: None });
		}
		let db = Connection::open_with_flags(
			&path,
			OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
		)
		.with_context(|| format!("open {}", path.display()))?;
		db.busy_timeout(Duration::from_secs(5))?;
		let exists = db
			.query_row(
				"SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'auth_credentials'",
				[],
				|_| Ok(()),
			)
			.optional()?
			.is_some();
		Ok(Self { db: exists.then_some(db) })
	}

	/// Open the store in the default agent directory.
	pub fn discover() -> Result<Self> {
		match crate::config::agent_dir_from(None, |key| std::env::var_os(key)) {
			Some(dir) => Self::open(&dir),
			None => Ok(Self { db: None }),
		}
	}

	/// `listStoredCredentials`: enabled rows, optionally for one provider, in id order.
	pub fn list(&self, provider: Option<&str>) -> Result<Vec<StoredCredential>> {
		let Some(db) = &self.db else { return Ok(Vec::new()) };
		let mut statement = db.prepare(
			"SELECT id, provider, credential_type, data FROM auth_credentials WHERE disabled_cause \
			 IS NULL AND (?1 IS NULL OR provider = ?1) ORDER BY id ASC",
		)?;
		let rows = statement.query_map(params![provider], |row| {
			Ok((
				row.get::<_, i64>(0)?,
				row.get::<_, String>(1)?,
				row.get::<_, String>(2)?,
				row.get::<_, String>(3)?,
			))
		})?;
		let mut out = Vec::new();
		for row in rows {
			let (id, provider, kind, data) = row?;
			if let Some(credential) = deserialize(&kind, &data) {
				out.push(StoredCredential { id, provider, credential });
			}
		}
		Ok(out)
	}

	/// Providers with at least one enabled credential, sorted.
	pub fn providers(&self) -> Result<Vec<String>> {
		let mut names: Vec<String> = self.list(None)?.into_iter().map(|row| row.provider).collect();
		names.sort();
		names.dedup();
		Ok(names)
	}

	/// `listOAuthAccounts`: OAuth rows in stored order.
	pub fn oauth_accounts(&self, provider: &str) -> Result<Vec<OAuthAccount>> {
		let field = |c: &Credential, name: &str| c.oauth_str(name).map(str::to_owned);
		Ok(self
			.list(Some(provider))?
			.into_iter()
			.filter(|row| matches!(row.credential, Credential::OAuth(_)))
			.enumerate()
			.map(|(position, row)| OAuthAccount {
				position,
				credential_id: row.id,
				email: field(&row.credential, "email"),
				account_id: field(&row.credential, "accountId"),
				project_id: field(&row.credential, "projectId"),
				enterprise: field(&row.credential, "enterpriseUrl"),
				org_name: field(&row.credential, "orgName"),
				org_id: field(&row.credential, "orgId"),
			})
			.collect())
	}

	/// Unexpired access token of the OAuth account at `position`.
	pub fn oauth_access_at(&self, provider: &str, position: usize) -> Result<Option<String>> {
		Ok(self
			.list(Some(provider))?
			.into_iter()
			.filter(|row| matches!(row.credential, Credential::OAuth(_)))
			.nth(position)
			.and_then(|row| row.credential.live_access().map(str::to_owned)))
	}

	/// Stored part of `getApiKey` precedence: live OAuth, then login api_key,
	/// then `env`, then a plain stored api_key.
	pub fn api_key(&self, provider: &str, env: Option<String>) -> Result<Option<String>> {
		let rows = self.list(Some(provider))?;
		if let Some(access) = rows.iter().find_map(|row| row.credential.live_access()) {
			return Ok(Some(access.to_owned()));
		}
		let key = |want_login: bool| {
			rows.iter().find_map(|row| match &row.credential {
				Credential::ApiKey { key, login } if *login == want_login => Some(key.clone()),
				_ => None,
			})
		};
		if let Some(found) = key(true) {
			return Ok(Some(found));
		}
		if env.is_some() {
			return Ok(env);
		}
		Ok(key(false))
	}

	/// Store a static API key (`serializeCredential` api_key shape), replacing
	/// the provider's existing plain api_key row.
	pub fn set_api_key(&self, provider: &str, key: &str) -> Result<()> {
		let Some(db) = &self.db else {
			bail!("credential store is not initialized; run the TypeScript application once first")
		};
		let data = serde_json::json!({ "key": key }).to_string();
		let existing = self
			.list(Some(provider))?
			.into_iter()
			.find_map(|row| match row.credential {
				Credential::ApiKey { login: false, .. } => Some(row.id),
				_ => None,
			});
		match existing {
			Some(id) => db.execute(
				&format!(
					"UPDATE auth_credentials SET credential_type = 'api_key', data = ?1, identity_key \
					 = NULL, updated_at = {SQLITE_NOW_EPOCH} WHERE id = ?2"
				),
				params![data, id],
			)?,
			None => db.execute(
				&format!(
					"INSERT INTO auth_credentials (provider, credential_type, data, identity_key, \
					 created_at, updated_at) VALUES (?1, 'api_key', ?2, NULL, {SQLITE_NOW_EPOCH}, \
					 {SQLITE_NOW_EPOCH})"
				),
				params![provider, data],
			)?,
		};
		Ok(())
	}
}

impl AuthStorage {
	fn db(&self) -> Result<&Connection> {
		match &self.db {
			Some(db) => Ok(db),
			None => bail!("credential store is not initialized; run the TypeScript application once first"),
		}
	}

	/// `deleteAuthCredentialsForProvider`: soft-disable every enabled row.
	pub fn disable_provider(&self, provider: &str, cause: &str) -> Result<()> {
		let cause = if cause.trim().is_empty() { "disabled" } else { cause.trim() };
		self.db()?.execute(
			&format!(
				"UPDATE auth_credentials SET disabled_cause = ?1, updated_at = {SQLITE_NOW_EPOCH} \
				 WHERE provider = ?2 AND disabled_cause IS NULL"
			),
			params![cause, provider],
		)?;
		Ok(())
	}

	/// Simplified `upsertAuthCredentialForProvider` for OAuth rows: api_key rows
	/// are replaced, and an OAuth row with the same email/accountId is updated.
	/// `identity_key` is left NULL so TS re-derives it from the data.
	pub fn upsert_oauth(&self, provider: &str, credential: &Map<String, Value>) -> Result<()> {
		let db = self.db()?;
		let mut data = credential.clone();
		data.remove("type");
		let data_text = Value::Object(data).to_string();
		let ident = |c: &Credential, f: &str| c.oauth_str(f).map(str::to_owned);
		let want_email = credential.get("email").and_then(Value::as_str);
		let want_account = credential.get("accountId").and_then(Value::as_str);
		let mut target = None;
		for row in self.list(Some(provider))? {
			match &row.credential {
				Credential::ApiKey { .. } => {
					db.execute(
						&format!(
							"UPDATE auth_credentials SET disabled_cause = 'replaced by oauth login', \
							 updated_at = {SQLITE_NOW_EPOCH} WHERE id = ?1"
						),
						params![row.id],
					)?;
				},
				c @ Credential::OAuth(_) => {
					let same = (want_email.is_some() && ident(c, "email").as_deref() == want_email)
						|| (want_account.is_some() && ident(c, "accountId").as_deref() == want_account)
						|| (want_email.is_none() && want_account.is_none());
					if same && target.is_none() {
						target = Some(row.id);
					}
				},
			}
		}
		match target {
			Some(id) => db.execute(
				&format!(
					"UPDATE auth_credentials SET credential_type = 'oauth', data = ?1, identity_key = \
					 NULL, updated_at = {SQLITE_NOW_EPOCH} WHERE id = ?2"
				),
				params![data_text, id],
			)?,
			None => db.execute(
				&format!(
					"INSERT INTO auth_credentials (provider, credential_type, data, identity_key, \
					 created_at, updated_at) VALUES (?1, 'oauth', ?2, NULL, {SQLITE_NOW_EPOCH}, \
					 {SQLITE_NOW_EPOCH})"
				),
				params![provider, data_text],
			)?,
		};
		Ok(())
	}
}

/// Stored API key for `provider` from the default store, ignoring store errors.
pub fn stored_api_key(provider: &str) -> Option<String> {
	AuthStorage::discover().ok()?.api_key(provider, None).ok().flatten()
}
