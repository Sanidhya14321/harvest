//! Bank path resolution (`core/banks.ts` `getBankDbPath`).

use std::path::{Path, PathBuf};

use crate::{MnemopiError, Result, config::DEFAULT_DB_FILENAME};

pub fn validate_bank_name(name: &str) -> Result<()> {
	if name.is_empty() {
		return Err(MnemopiError::Invalid("Bank name cannot be empty".into()));
	}
	if name == "default" {
		return Ok(());
	}
	if name.chars().count() > 64 {
		return Err(MnemopiError::Invalid(format!("Bank name '{name}' exceeds 64 characters")));
	}
	if !name
		.chars()
		.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
	{
		return Err(MnemopiError::Invalid(format!("Bank name '{name}' contains invalid characters")));
	}
	Ok(())
}

/// `default` (or empty) maps to `<data_dir>/mnemopi.db`; others to
/// `<data_dir>/banks/<name>/mnemopi.db`.
pub fn bank_db_path(data_dir: &Path, name: &str) -> Result<PathBuf> {
	if name.is_empty() || name == "default" {
		return Ok(data_dir.join(DEFAULT_DB_FILENAME));
	}
	validate_bank_name(name)?;
	Ok(data_dir.join("banks").join(name).join(DEFAULT_DB_FILENAME))
}
