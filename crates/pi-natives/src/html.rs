//! HTML to Markdown conversion.

use napi::{JsString, bindgen_prelude::*};
use napi_derive::napi;

use crate::{js::into_string, task};

/// Options for HTML to Markdown conversion.
#[napi(object)]
#[derive(Debug, Default)]
pub struct HtmlToMarkdownOptions {
	/// Remove navigation elements, forms, headers, footers.
	pub clean_content: Option<bool>,
	/// Skip images during conversion.
	pub skip_images:   Option<bool>,
}

/// Convert HTML source to Markdown with optional preprocessing.
///
/// # Errors
/// Returns an error if the conversion fails or the worker task aborts.
#[napi]
pub fn html_to_markdown(
	html: JsString,
	options: Option<HtmlToMarkdownOptions>,
) -> Result<task::Promise<String>> {
	let html = into_string(html)?;
	let options = options.unwrap_or_default();
	let clean_content = options.clean_content.unwrap_or(false);
	let skip_images = options.skip_images.unwrap_or(false);

	Ok(task::blocking("html_to_markdown", (), move |_| {
		harvest_media::html::html_to_markdown(&html, harvest_media::html::HtmlToMarkdownOptions {
			clean_content,
			skip_images,
		})
		.map_err(|error| Error::from_reason(error.to_string()))
	}))
}
