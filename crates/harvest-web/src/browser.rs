use harvest_protocol::display::{
	DisplayBlock, DisplayContent, DisplayEntry, DisplayExecution, ExecutionLanguage,
	ReplayProjection,
};
use leptos::{mount::mount_to, prelude::*};
use wasm_bindgen::{JsCast, prelude::*};

use crate::{image_source, searchable_text};

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.ok_or_else(|| JsValue::from_str("Harvest replay requires a document"))?;
	let root = document
		.get_element_by_id("harvest-replay")
		.ok_or_else(|| JsValue::from_str("Harvest replay root is missing"))?;
	let snapshot = document
		.get_element_by_id("harvest-replay-data")
		.and_then(|element| element.text_content())
		.ok_or_else(|| JsValue::from_str("Harvest replay snapshot is missing"))?;
	let projection: ReplayProjection = if let Ok(projection) = serde_json::from_str(&snapshot) {
		projection
	} else {
		root.set_text_content(Some("This session snapshot could not be decoded."));
		root.set_attribute("role", "alert")?;
		return Err(JsValue::from_str("Invalid Harvest replay snapshot"));
	};
	document.set_title(
		projection
			.title
			.as_deref()
			.unwrap_or("Harvest session replay"),
	);
	mount_to(root.dyn_into::<web_sys::HtmlElement>()?, move || {
		view! { <ReplayApp projection/> }
	})
	.forget();
	Ok(())
}

#[component]
fn ReplayApp(projection: ReplayProjection) -> impl IntoView {
	let (query, set_query) = signal(String::new());
	let title = projection
		.title
		.unwrap_or_else(|| "Session replay".to_owned());
	let entries = projection
		.entries
		.into_iter()
		.filter(DisplayEntry::is_visible);
	let views = entries
		.map(|entry| {
			let searchable = searchable_text(&entry);
			let element_id = format!("entry-{}", entry.id);
			view! {
				<article class="entry" id=element_id
					hidden=move || !searchable.contains(&query.get())>
					<EntryView entry/>
				</article>
			}
		})
		.collect_view();
	view! {
		<main>
			<header>
				<p class="eyebrow">"HARVEST · READ-ONLY REPLAY"</p>
				<h1>{title}</h1>
				<p class="metadata">"Session " {projection.session_id} " · Branch " {projection.leaf_id}</p>
				{projection.cwd.map(|cwd| view! { <p class="metadata">{cwd}</p> })}
				<label for="replay-search">"Search this branch"</label>
				<input id="replay-search" type="search" autocomplete="off"
					on:input=move |event| set_query.set(event_target_value(&event).to_lowercase())/>
			</header>
			<section aria-label="Session history">{views}</section>
		</main>
	}
}

#[component]
fn EntryView(entry: DisplayEntry) -> AnyView {
	let timestamp = entry
		.timestamp
		.map(|timestamp| view! { <time>{timestamp}</time> });
	let body = match entry.content {
		DisplayContent::Message { message } => {
			let heading = match (&message.tool_name, &message.tool_call_id) {
				(Some(name), _) => format!("Tool result · {name}"),
				(None, Some(_)) => "Tool result".to_owned(),
				_ => message.role,
			};
			view! {
				<h2>{heading}</h2>
				{message.tool_call_id.map(|id| view! { <p class="metadata">"Call " {id}</p> })}
				{message.execution.map(|execution| view! { <ExecutionView execution/> })}
				{message.blocks.into_iter().map(|block| view! { <BlockView block/> }).collect_view()}
				{message.error_message.map(|error| view! { <pre class="error">{error}</pre> })}
				{message.is_error.then(|| view! { <p class="error">"Tool reported an error"</p> })}
			}
			.into_any()
		},
		DisplayContent::CustomMessage { custom_type, blocks } => view! {
			<h2>{custom_type}</h2>
			{blocks.into_iter().map(|block| view! { <BlockView block/> }).collect_view()}
		}
		.into_any(),
		DisplayContent::Compaction {
			summary, short_summary, tokens_before, tokens_after, ..
		} => view! {
			<h2>"Context compaction"</h2>
			{short_summary.map(|short| view! { <p>{short}</p> })}
			<pre>{summary}</pre>
			{tokens_before.zip(tokens_after).map(|(before, after)| view! {
				<p class="metadata">{format!("{before} → {after} tokens")}</p>
			})}
		}
		.into_any(),
		DisplayContent::BranchSummary { summary, .. } => {
			view! { <h2>"Branch summary"</h2><pre>{summary}</pre> }.into_any()
		},
		DisplayContent::ModelChange { model, role } => view! {
			<h2>"Model changed"</h2><p>{model}</p>
			{role.map(|role| view! { <p class="metadata">{role}</p> })}
		}
		.into_any(),
		DisplayContent::ThinkingLevelChange { thinking_level } => view! {
			<h2>"Thinking level changed"</h2>
			<p>{thinking_level.unwrap_or_else(|| "Default".to_owned())}</p>
		}
		.into_any(),
		DisplayContent::Hidden | DisplayContent::Unknown { .. } => ().into_any(),
	};
	view! { {timestamp}{body} }.into_any()
}

#[component]
fn BlockView(block: DisplayBlock) -> AnyView {
	match block {
		DisplayBlock::Text { text } => view! { <pre>{text}</pre> }.into_any(),
		DisplayBlock::Thinking { thinking } => view! {
			<details><summary>"Reasoning"</summary><pre>{thinking}</pre></details>
		}
		.into_any(),
		DisplayBlock::Image { data, mime_type } => match image_source(&mime_type, &data) {
			Some(source) => view! {
				<figure><img src=source alt="Image attached to the session" loading="lazy"/></figure>
			}
			.into_any(),
			None => {
				view! { <p class="metadata">"Image omitted: unsupported or invalid image data"</p> }
					.into_any()
			},
		},
		DisplayBlock::ToolCall { id, name, arguments, intent } => view! {
			<details class="tool-call" open>
				<summary>{"Tool call · "}{name}</summary>
				<p class="metadata">"Call " {id}</p>
				{intent.map(|intent| view! { <p>{intent}</p> })}
				{arguments.map(|arguments| view! { <pre>{arguments.get().to_owned()}</pre> })}
			</details>
		}
		.into_any(),
		DisplayBlock::RedactedThinking => {
			view! { <p class="metadata">"Reasoning is redacted"</p> }.into_any()
		},
		DisplayBlock::Unknown { .. } => ().into_any(),
	}
}

#[component]
fn ExecutionView(execution: DisplayExecution) -> impl IntoView {
	let language = match execution.language {
		ExecutionLanguage::Shell => "Shell",
		ExecutionLanguage::Python => "Python",
		ExecutionLanguage::Javascript => "Historical JavaScript",
	};
	view! {
		<details class="execution" open>
			<summary>{language} " execution"</summary>
			<pre>{execution.code}</pre><pre>{execution.output}</pre>
			{execution.exit_code.map(|code| view! { <p class="metadata">{format!("Exit code {code}")}</p> })}
			{execution.cancelled.then(|| view! { <p class="metadata">"Cancelled"</p> })}
			{execution.truncated.then(|| view! { <p class="metadata">"Output was truncated"</p> })}
		</details>
	}
}
