//! HTML-to-Markdown conversion shared by the CLI and addon worker pool.
use anyhow::Result;
use html_to_markdown_rs::{
	ConversionOptions, PreprocessingOptions, PreprocessingPreset, WarningKind, convert,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct HtmlToMarkdownOptions {
	pub clean_content: bool,
	pub skip_images:   bool,
}

/// Convert HTML using the existing Tier2 parser and reject depth-truncated
/// output rather than passing incomplete document content to a model.
///
/// ```
/// use harvest_media::html::{HtmlToMarkdownOptions, html_to_markdown};
/// let text = html_to_markdown("<h1>Rust &amp; HTML</h1>", HtmlToMarkdownOptions::default())?;
/// assert_eq!(text, "# Rust & HTML\n");
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn html_to_markdown(html: &str, options: HtmlToMarkdownOptions) -> Result<String> {
	let HtmlToMarkdownOptions { clean_content, skip_images } = options;
	let conversion_opts = ConversionOptions {
		skip_images,
		preprocessing: PreprocessingOptions {
			enabled:           clean_content,
			preset:            PreprocessingPreset::Aggressive,
			remove_navigation: true,
			remove_forms:      true,
		},
		tier_strategy: html_to_markdown_rs::TierStrategy::Tier2,
		..Default::default()
	};

	let result = convert(html, Some(conversion_opts))
		.map_err(|err| anyhow::Error::msg(format!("Conversion error: {err}")))?;
	if let Some(warning) = result
		.warnings
		.iter()
		.find(|warning| warning.kind == WarningKind::DepthLimitExceeded)
	{
		return Err(anyhow::Error::msg(format!("Conversion error: {}", warning.message)));
	}
	Ok(result.content.unwrap_or_default())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn headings_links_lists_and_code_retain_markdown_structure() -> Result<()> {
		let html = "<h1>Title &amp; code</h1><p><a href=\"https://example.com\">site</a></p><ul><li>one</li></ul><pre><code>let x = 1;</code></pre>";
		let markdown = html_to_markdown(html, HtmlToMarkdownOptions::default())?;
		assert!(markdown.contains("# Title & code"));
		assert!(markdown.contains("[site](https://example.com)"));
		assert!(markdown.contains("- one"));
		assert!(markdown.contains("```\nlet x = 1;\n```"));
		Ok(())
	}

	#[test]
	fn image_omission_is_explicit_and_does_not_erase_surrounding_text() -> Result<()> {
		let html = "<p>before <img src=\"diagram.png\" alt=\"diagram\"> after</p>";
		let included = html_to_markdown(html, HtmlToMarkdownOptions::default())?;
		assert!(included.contains("![diagram](diagram.png)"));
		let omitted =
			html_to_markdown(html, HtmlToMarkdownOptions { skip_images: true, ..Default::default() })?;
		assert!(!omitted.contains("diagram"));
		assert!(omitted.contains("before"));
		assert!(omitted.contains("after"));
		Ok(())
	}

	#[test]
	fn depth_truncation_fails_instead_of_returning_incomplete_content() {
		let html = format!("{}<p>deep</p>{}", "<div>".repeat(90), "</div>".repeat(90));
		let error = html_to_markdown(&html, HtmlToMarkdownOptions {
			clean_content: true,
			..Default::default()
		})
		.unwrap_err();
		assert!(error.to_string().contains("effective depth limit of 64"));
	}
	#[test]
	fn cleaning_removes_navigation_and_footer_without_losing_main_content() -> Result<()> {
		let html = "<nav>menu</nav><main><p>body</p></main><footer>footer</footer>";
		let raw = html_to_markdown(html, HtmlToMarkdownOptions::default())?;
		assert!(raw.contains("menu"));
		assert!(raw.contains("footer"));
		let clean = html_to_markdown(html, HtmlToMarkdownOptions {
			clean_content: true,
			..Default::default()
		})?;
		assert_eq!(clean.trim(), "body");
		Ok(())
	}
}
