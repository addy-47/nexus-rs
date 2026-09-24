use scraper::Html;

use crate::model::RawPage;

pub mod cleaner;

pub const DEFAULT_MAX_PAGE_CHARS: usize = 30_000;

/// Extracts clean Markdown content and page title from raw HTML.
pub fn extract_document(url: &str, html: &str, max_chars: usize) -> RawPage {
    let doc = Html::parse_document(html);
    let title = cleaner::extract_page_title(&doc).unwrap_or_else(|| url.to_owned());
    let markdown = cleaner::html_to_markdown(html, max_chars).unwrap_or_default();

    RawPage {
        url: url.to_owned(),
        title,
        markdown,
    }
}
