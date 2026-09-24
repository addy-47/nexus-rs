use html_to_markdown_rs::{ConversionOptions, PreprocessingOptions, PreprocessingPreset};
use scraper::{Html, Selector};

use crate::error::NexusError;

const EXCLUDE_SELECTORS: &[&str] = &[
    "head", "script", "style", "template", "svg", "nav", "header", "footer", "aside", "form",
    "iframe", "noscript",
];

/// Extracts the document title from `<title>` or the first `<h1>` using fast substring scanning,
/// falling back to DOM parsing only if malformed.
pub fn extract_page_title(html: &str) -> Option<String> {
    if let Some(title) = fast_scan_tag(html, "title") {
        return Some(title);
    }
    if let Some(h1) = fast_scan_tag(html, "h1") {
        return Some(h1);
    }

    // Fallback parser for complex/malformed DOMs
    let document = Html::parse_document(html);
    find_first_tag_text(&document, "title").or_else(|| find_first_tag_text(&document, "h1"))
}

/// Fast substring scan for opening and closing tag text without parsing full HTML DOM.
fn fast_scan_tag(html: &str, tag: &str) -> Option<String> {
    let search_slice = if html.len() > 65536 {
        &html[..65536]
    } else {
        html
    };

    let open_tag = format!("<{tag}");
    let close_tag = format!("</{tag}>");

    let lower = search_slice.to_ascii_lowercase();
    let start_idx = lower.find(&open_tag)?;
    let tag_open_end = search_slice[start_idx..].find('>')? + start_idx + 1;

    let content_end = if let Some(close_idx) = lower[tag_open_end..].find(&close_tag) {
        tag_open_end + close_idx
    } else {
        // Fallback for unclosed tag: take up to next opening '<' or newline
        let rel_next_tag = search_slice[tag_open_end..]
            .find('<')
            .unwrap_or(search_slice.len() - tag_open_end);
        let rel_newline = search_slice[tag_open_end..]
            .find('\n')
            .unwrap_or(search_slice.len() - tag_open_end);
        tag_open_end + rel_next_tag.min(rel_newline)
    };

    let raw = &search_slice[tag_open_end..content_end];
    let decoded = decode_html_entities(raw.trim());
    let first_line = decoded.lines().next()?.trim();
    if first_line.is_empty() {
        None
    } else {
        Some(first_line.to_string())
    }
}

/// Decodes common XML/HTML character entities in extracted titles.
fn decode_html_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&ndash;", "–")
        .replace("&mdash;", "—")
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

    let result = html_to_markdown_rs::convert(html, options)
        .map_err(|e| NexusError::InvalidConfiguration(format!("HTML conversion failed: {e}")))?;

    let raw_content = result.content.unwrap_or_default();
    let trimmed = raw_content.trim();

    if trimmed.is_empty() || trimmed == "```\n\n```" {
        return Ok(String::new());
    }

    let retained: String = trimmed.chars().take(max_chars).collect();
    Ok(retained)
}
