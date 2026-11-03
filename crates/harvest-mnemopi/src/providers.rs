//! Host-implemented model access. Mnemopi never talks to the network itself.

/// Text completion used for fact extraction and LLM-assisted consolidation.
pub trait Llm: Send + Sync {
	/// Complete `prompt`. Implementations MUST cap the response body they read
	/// from the network; Mnemopi additionally truncates to
	/// [`MAX_LLM_RESPONSE_CHARS`] before parsing.
	fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String, String>;
}

/// Dense embedding provider.
pub trait Embedder: Send + Sync {
	/// Model identifier stamped onto every `memory_embeddings` row.
	fn model(&self) -> String;
	/// Embed a batch of texts. Must return one vector per input.
	fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

/// Upper bound applied to any LLM output before it is parsed.
pub const MAX_LLM_RESPONSE_CHARS: usize = 64 * 1024;
