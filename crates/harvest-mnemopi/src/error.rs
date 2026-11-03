use thiserror::Error;

#[derive(Debug, Error)]
pub enum MnemopiError {
	#[error("sqlite: {0}")]
	Sqlite(#[from] rusqlite::Error),
	#[error("io: {0}")]
	Io(#[from] std::io::Error),
	#[error("json: {0}")]
	Json(#[from] serde_json::Error),
	#[error("invalid argument: {0}")]
	Invalid(String),
	#[error("provider: {0}")]
	Provider(String),
}

pub type Result<T> = std::result::Result<T, MnemopiError>;
