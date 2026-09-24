use html_to_markdown_rs::{ConversionOptions, PreprocessingOptions, PreprocessingPreset};
use scraper::{Html, Selector};

use crate::error::NexusError;

const EXCLUDE_SELECTORS: &[&str] = &[
    "head", "script", "style", "template", "svg", "nav", "header", "footer", "aside", "form",
    "iframe", "noscript",
];

/// Extracts the document title from `<title>` or the first `<h1>`.
pub fn extract_page_title(document: &Html) -> Option<String> {
    find_first_tag_text(document, "title").or_else(|| find_first_tag_text(document, "h1"))
}

/// Finds the first non-empty line of text matching a CSS tag selector.
fn find_first_tag_text(document: &Html, tag: &str) -> Option<String> {
    let selector = Selector::parse(tag).ok()?;
    document
        .select(&selector)
        .filter_map(|e| {
            let full = e.text().collect::<String>();
            let first_line = full.lines().next()?.trim();
            if first_line.is_empty() {
                None
            } else {
                Some(first_line.to_owned())
            }
        })
        .next()
}

/// Normalizes raw HTML and converts structural content into clean Markdown.
pub fn html_to_markdown(html: &str, max_chars: usize) -> Result<String, NexusError> {
    let document = Html::parse_document(html);
    let normalized = document.html();

    let options = ConversionOptions {
        preprocessing: PreprocessingOptions {
            preset: PreprocessingPreset::Standard,
            ..Default::default()
        },
        extract_metadata: false,
        skip_images: true,
        exclude_selectors: EXCLUDE_SELECTORS.iter().map(|&s| s.to_owned()).collect(),
        ..Default::default()
    };

    let result = html_to_markdown_rs::convert(&normalized, options)
        .map_err(|e| NexusError::InvalidConfiguration(format!("HTML conversion failed: {e}")))?;

    let raw_content = result.content.unwrap_or_default();
    let trimmed = raw_content.trim();

    if trimmed.is_empty() || trimmed == "```\n\n```" {
        return Ok(String::new());
    }

    let retained: String = trimmed.chars().take(max_chars).collect();
    Ok(retained)
}
