//! Adapt the host's existing model to the engine's synchronous LLM callback.
//! The callback can run only inside a caller-owned cancellable operation.
use std::{
	cell::RefCell,
	marker::PhantomData,
	path::{Path, PathBuf},
	rc::Rc,
	sync::Arc,
};

use harvest_runtime::cancel::CancelToken;

use crate::{chat, provider::Model};

thread_local! {
	static OPERATION: RefCell<Option<CancelToken>> = const { RefCell::new(None) };
}

/// Reentrant caller-thread context avoids sharing one request's cancellation
/// with a concurrent session using the same native backend.
pub(crate) struct Operation {
	previous:      Option<CancelToken>,
	_thread_bound: PhantomData<Rc<()>>,
}

impl Operation {
	pub(crate) fn enter(cancellation: Option<CancelToken>) -> Self {
		Self {
			previous:      OPERATION.with(|slot| slot.replace(cancellation)),
			_thread_bound: PhantomData,
		}
	}
}

impl Drop for Operation {
	fn drop(&mut self) {
		OPERATION.with(|slot| {
			slot.replace(self.previous.take());
		});
	}
}

pub struct ModelLlm {
	model:           Arc<dyn Model + Send + Sync>,
	root:            PathBuf,
	agent_directory: Option<PathBuf>,
}

impl ModelLlm {
	pub fn new(model: Arc<dyn Model + Send + Sync>, root: &Path, directory: Option<&Path>) -> Self {
		Self { model, root: root.to_owned(), agent_directory: directory.map(Path::to_owned) }
	}
}

impl harvest_mnemopi::providers::Llm for ModelLlm {
	fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
		let cancellation = OPERATION
			.with(|slot| slot.borrow().clone())
			.ok_or("memory provider has no active cancellable operation")?;
		cancellation
			.heartbeat()
			.map_err(|_| "memory provider operation cancelled")?;
		if prompt.len() > 256 * 1024 || max_tokens == 0 {
			return Err("memory provider request exceeds configured bounds".into());
		}
		let agent_dir =
			crate::config::agent_dir_from(self.agent_directory.clone(), |key| std::env::var_os(key))
				.unwrap_or_else(|| self.root.clone());
		let redactor = crate::commands::secrets::SecretObfuscator::build(&self.root, &agent_dir);
		let redacted = redactor.obfuscate(prompt);
		if redacted.len() > 256 * 1024 {
			return Err("redacted memory request exceeds byte limit".into());
		}
		let messages =
			[chat::system(include_str!("../prompts/memory-provider.md")), chat::user(redacted)];
		let limit = (max_tokens.clamp(1, 4096) as usize)
			.saturating_mul(16)
			.min(64 * 1024);
		let mut received = 0_usize;
		let response = self
			.model
			.stream_response_with_output_limit(
				&messages,
				&[],
				max_tokens.clamp(1, 4096),
				&cancellation,
				&mut |text| {
					cancellation.heartbeat()?;
					received = received
						.checked_add(text.len())
						.ok_or_else(|| anyhow::anyhow!("memory response size overflow"))?;
					if received > limit {
						anyhow::bail!("memory provider response exceeds byte limit");
					}
					Ok(())
				},
			)
			.map_err(|_| "memory provider completion failed")?;
		cancellation
			.heartbeat()
			.map_err(|_| "memory provider operation cancelled")?;
		if response.message.role != "assistant"
			|| response
				.message
				.tool_calls
				.as_ref()
				.is_some_and(|calls| !calls.is_empty())
		{
			return Err("memory provider returned an unsupported response".into());
		}
		let text = response.message.content.unwrap_or_default();
		if text.len() > limit {
			return Err("memory provider response exceeds byte limit".into());
		}
		Ok(redactor.obfuscate(&text))
	}
}
