//! OpenAI-compatible token accounting; pricing remains catalog-owned.
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
	pub input:            u64,
	pub output:           u64,
	pub cache_read:       u64,
	pub cache_write:      u64,
	pub total_tokens:     u64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct UsageReport {
	pub raw:    Box<RawValue>,
	pub tokens: Option<TokenUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
	prompt_tokens:              Option<u64>,
	completion_tokens:          Option<u64>,
	cached_tokens:              Option<u64>,
	prompt_cache_hit_tokens:    Option<u64>,
	prompt_cache_miss_tokens:   Option<u64>,
	#[serde(rename = "cachedContentTokenCount")]
	cached_content_token_count: Option<u64>,
	prompt_tokens_details:      Option<PromptDetails>,
	completion_tokens_details:  Option<CompletionDetails>,
}
#[derive(Deserialize)]
struct PromptDetails {
	cached_tokens:      Option<u64>,
	cache_write_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct CompletionDetails {
	reasoning_tokens: Option<u64>,
}

impl UsageReport {
	/// Anthropic input tokens exclude cache reads/writes. Missing or invalid
	/// counters remain unavailable; raw provider data is always retained.
	pub fn from_anthropic_raw(raw: Box<RawValue>) -> Self {
		#[derive(Deserialize)]
		struct Counters {
			#[serde(rename = "input_tokens")]
			input:       u64,
			#[serde(rename = "output_tokens")]
			output:      u64,
			#[serde(default, rename = "cache_read_input_tokens")]
			cache_read:  u64,
			#[serde(default, rename = "cache_creation_input_tokens")]
			cache_write: u64,
		}
		let tokens = serde_json::from_str::<Counters>(raw.get())
			.ok()
			.and_then(|c| {
				Some(TokenUsage {
					input:            c.input,
					output:           c.output,
					cache_read:       c.cache_read,
					cache_write:      c.cache_write,
					total_tokens:     c
						.input
						.checked_add(c.output)?
						.checked_add(c.cache_read)?
						.checked_add(c.cache_write)?,
					reasoning_tokens: None,
				})
			});
		Self { raw, tokens }
	}

	/// Preserve original bytes even when counters are partial, invalid or new.
	pub fn from_raw(raw: Box<RawValue>) -> Self {
		let tokens = serde_json::from_str::<WireUsage>(raw.get())
			.ok()
			.and_then(account);
		Self { raw, tokens }
	}
}

fn account(usage: WireUsage) -> Option<TokenUsage> {
	let prompt = usage.prompt_tokens?;
	let output = usage.completion_tokens?;
	let cached = [
		usage.cached_tokens,
		usage.prompt_cache_hit_tokens,
		usage
			.prompt_tokens_details
			.as_ref()
			.and_then(|details| details.cached_tokens),
		usage.cached_content_token_count,
	]
	.into_iter()
	.flatten()
	.find(|value| *value > 0)
	.unwrap_or(0);
	let explicit_write = usage
		.prompt_tokens_details
		.and_then(|details| details.cache_write_tokens);
	let exclusive_miss_counters = usage.prompt_cache_hit_tokens.is_some()
		&& usage.prompt_cache_miss_tokens.is_some()
		&& explicit_write.is_none()
		&& usage.prompt_cache_miss_tokens.unwrap_or(0) > 0;
	let write = if exclusive_miss_counters {
		0
	} else {
		explicit_write
			.or(usage.prompt_cache_miss_tokens)
			.unwrap_or(0)
	};
	let input = prompt.saturating_sub(cached).saturating_sub(write);
	let total = input
		.checked_add(output)?
		.checked_add(cached)?
		.checked_add(write)?;
	let reasoning = usage
		.completion_tokens_details
		.and_then(|details| details.reasoning_tokens)
		.filter(|tokens| *tokens > 0);
	Some(TokenUsage {
		input,
		output,
		cache_read: cached,
		cache_write: write,
		total_tokens: total,
		reasoning_tokens: reasoning,
	})
}

#[cfg(test)]
mod tests {
	use anyhow::Result;

	use super::*;

	#[derive(Deserialize)]
	struct OracleCase {
		name:     String,
		raw:      Box<RawValue>,
		expected: serde_json::Value,
	}

	#[test]
	fn legacy_oracle_preserves_alias_precedence_zero_overrides_and_additive_accounting() -> Result<()>
	{
		let cases: Vec<OracleCase> =
			serde_json::from_str(include_str!("../tests/fixtures/openai-usage-accounting.json"))?;
		for case in cases {
			let report = UsageReport::from_raw(case.raw);
			let tokens = report.tokens.unwrap();
			assert_eq!(serde_json::to_value(tokens)?, case.expected, "{}", case.name);
		}
		Ok(())
	}

	#[test]
	fn malformed_partial_and_overflowed_counters_remain_lossless_but_unavailable() -> Result<()> {
		for raw in [
			r#"{"total_tokens":7,"future":900719925474099312345}"#,
			r#"{"prompt_tokens":-1,"completion_tokens":2}"#,
			r#"{"prompt_tokens":18446744073709551615,"completion_tokens":1}"#,
		] {
			let report = UsageReport::from_raw(RawValue::from_string(raw.into())?);
			assert!(report.tokens.is_none());
			assert_eq!(report.raw.get(), raw);
		}
		Ok(())
	}
}
