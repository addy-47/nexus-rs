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
    // N0/P1: byte-indexing a `&str` panics when 65536 lands inside a multi-byte
    // char (observed live: a PDF served as a search result). Floor to the
    // boundary — behavior is otherwise identical.
    let search_slice = if html.len() > 65536 {
        &html[..html.floor_char_boundary(65536)]
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

    // P0-6: the observation is vocalized, so strip Markdown that is unspeakable
    // and decode entities exactly once. `[text](url)` -> `text`; fenced code
    // blocks and flattened table skeletons are dropped (code passages are
    // rejected wholesale by `quality::is_code_like`, this handles residue).
    let sanitized = sanitize_for_speech(trimmed);
    let retained: String = sanitized.chars().take(max_chars).collect();
    Ok(retained)
}

/// Strips unspeakable Markdown residue and decodes entities exactly once.
///
/// The converter output may carry `[label](url)` links, `#`/`##` heading marks,
/// `**bold**`, fenced code blocks, and double-encoded entities (`&amp;apos;`).
/// All are noise for a spoken answer; entities are decoded once so `&apos;`
/// becomes `'` here and is then correctly re-escaped at the XML render layer.
fn sanitize_for_speech(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_fence = false;
    for line in input.lines() {
        let t = line.trim();
        // Drop fenced code blocks entirely (including the fences).
        if t.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Drop flattened table skeletons: rows that are only pipes/dashes/colons.
        let skeleton = t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) && t.contains('|');
        if skeleton {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    // `[label](url)` -> `label`. Iterates to handle adjacent links.
    let mut s = out;
    while let Some(open) = s.find('[') {
        let Some(mid) = s[open..].find("](") else {
            break;
        };
        let mid = open + mid;
        let Some(close) = s[mid + 2..].find(')') else {
            break;
        };
        let close = mid + 2 + close;
        let label = s[open + 1..mid].to_string();
        s.replace_range(open..=close, &label);
    }
    // Strip `#` heading marks and `**`/`__` emphasis, then decode entities once.
    let mut cleaned = String::with_capacity(s.len());
    for line in s.lines() {
        let t = line.trim_start_matches(['#', ' ']);
        let t = t.replace("**", "").replace("__", "");
        cleaned.push_str(&t);
        cleaned.push('\n');
    }
    decode_html_entities(cleaned.trim())
        .replace("&amp;apos;", "'")
        .replace("&amp;quot;", "\"")
        .replace("&amp;gt;", ">")
        .replace("&amp;lt;", "<")
        .replace("&amp;amp;", "&")
}
