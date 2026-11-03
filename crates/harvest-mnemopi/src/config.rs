//! Environment-driven configuration (subset of `config.ts` used by the engine).

use std::path::PathBuf;

pub const DEFAULT_DB_FILENAME: &str = "mnemopi.db";

fn env_opt(name: &str) -> Option<String> {
	std::env::var(name)
		.ok()
		.map(|v| v.trim().to_string())
		.filter(|v| !v.is_empty())
}

fn env_int(name: &str, default: i64) -> i64 {
	env_opt(name)
		.and_then(|v| v.parse().ok())
		.unwrap_or(default)
}

fn env_float(name: &str, default: f64) -> f64 {
	env_opt(name)
		.and_then(|v| v.parse().ok())
		.filter(|v: &f64| v.is_finite())
		.unwrap_or(default)
}

pub fn home_dir() -> PathBuf {
	std::env::var_os("HOME")
		.filter(|h| !h.is_empty())
		.map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_data_dir() -> PathBuf {
	home_dir().join(".hermes").join("mnemopi").join("data")
}

pub fn data_dir() -> PathBuf {
	env_opt("MNEMOPI_DATA_DIR")
		.map(PathBuf::from)
		.unwrap_or_else(default_data_dir)
}

pub fn db_path() -> PathBuf {
	data_dir().join(DEFAULT_DB_FILENAME)
}

pub fn working_memory_max_items() -> i64 {
	env_int("MNEMOPI_WM_MAX_ITEMS", 10000)
}
pub fn working_memory_ttl_hours() -> i64 {
	env_int("MNEMOPI_WM_TTL_HOURS", 24)
}
pub fn max_episode_chars() -> i64 {
	env_int("MNEMOPI_MAX_EPISODE_CHARS", 100_000).max(1)
}
pub fn sleep_batch_size() -> i64 {
	env_int("MNEMOPI_SLEEP_BATCH", 5000)
}
pub fn scratchpad_max_items() -> i64 {
	env_int("MNEMOPI_SP_MAX", 1000)
}
pub fn recency_halflife_hours() -> f64 {
	env_float("MNEMOPI_RECENCY_HALFLIFE", 168.0)
}
pub fn temporal_halflife_hours() -> f64 {
	env_float("MNEMOPI_TEMPORAL_HALFLIFE", 24.0)
}
pub fn tier2_days() -> i64 {
	env_int("MNEMOPI_TIER2_DAYS", 30)
}
pub fn tier3_days() -> i64 {
	env_int("MNEMOPI_TIER3_DAYS", 180)
}
pub fn degrade_batch_size() -> i64 {
	env_int("MNEMOPI_DEGRADE_BATCH", 100)
}
pub fn tier3_max_chars() -> usize {
	env_int("MNEMOPI_TIER3_MAX_CHARS", 300).max(0) as usize
}
pub fn embedding_max_input_chars() -> usize {
	env_int("MNEMOPI_EMBEDDING_MAX_INPUT_CHARS", 8192).max(0) as usize
}
pub fn embeddings_disabled() -> bool {
	env_opt("MNEMOPI_NO_EMBEDDINGS").is_some()
}
pub fn proactive_linking_override() -> Option<bool> {
	std::env::var("MNEMOPI_PROACTIVE_LINKING")
		.ok()
		.map(|v| v == "1")
}
pub fn mcp_bank() -> Option<String> {
	env_opt("MNEMOPI_MCP_BANK")
}
pub fn session_id_env() -> Option<String> {
	env_opt("MNEMOPI_SESSION_ID")
}
pub fn shared_surface_db() -> PathBuf {
	env_opt("MNEMOPI_SHARED_SURFACE_DB")
		.map(PathBuf::from)
		.unwrap_or_else(|| data_dir().join("shared").join(DEFAULT_DB_FILENAME))
}
pub fn author_id_env() -> Option<String> {
	env_opt("MNEMOPI_AUTHOR_ID")
}
pub fn author_type_env() -> Option<String> {
	env_opt("MNEMOPI_AUTHOR_TYPE")
}
pub fn channel_id_env() -> Option<String> {
	env_opt("MNEMOPI_CHANNEL_ID")
}
pub fn llm_max_tokens() -> u32 {
	env_int("MNEMOPI_LLM_MAX_TOKENS", 512).clamp(1, 32_768) as u32
}
