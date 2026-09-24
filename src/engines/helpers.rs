use std::cell::RefCell;
use std::collections::HashMap;

use scraper::{ElementRef, Selector};
use url::Url;

thread_local! {
    static SELECTOR_CACHE: RefCell<HashMap<&'static str, Selector>> = RefCell::new(HashMap::new());
}

/// Retrieves or parses a cached static CSS selector.
pub fn cached_selector(pattern: &'static str) -> Selector {
    SELECTOR_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(pattern)
            .or_insert_with(|| Selector::parse(pattern).expect("Static CSS selector must be valid"))
            .clone()
    })
}

/// Extracts combined trimmed text from a DOM element, separated by a delimiter.
pub fn element_text(element: ElementRef<'_>, delimiter: &str) -> String {
    element
        .text()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(delimiter)
}

/// Normalizes a URL for canonical deduplication across search engines.
pub fn canonicalize_url(raw_url: &str) -> String {
    let Ok(mut parsed) = Url::parse(raw_url) else {
        return raw_url.trim().to_lowercase();
    };

    parsed.set_fragment(None);

    let query_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| {
            let key = k.to_ascii_lowercase();
            !key.starts_with("utm_")
                && key != "ref"
                && key != "fbclid"
                && key != "gclid"
                && key != "mc_eid"
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if query_pairs.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(query_pairs);
    }

    let mut path = parsed.path().to_owned();
    if path.len() > 1 && path.ends_with('/') {
        path.pop();
        parsed.set_path(&path);
    }

    parsed.into()
}

pub const MAX_SERP_BYTES: usize = 2 * 1024 * 1024; // 2 MB SERP ceiling

/// Reads SERP response body via streaming and verifies that body does not exceed MAX_SERP_BYTES.
pub async fn read_serp_html(
    response: primp::Response,
    engine_name: &'static str,
) -> Result<String, crate::error::NexusError> {
    use futures_util::StreamExt;

    if let Some(cl) = response.content_length()
        && cl as usize > MAX_SERP_BYTES
    {
        return Err(crate::error::NexusError::ScraperTransport(format!(
            "{engine_name} SERP response content-length {cl} exceeds maximum limit of {MAX_SERP_BYTES} bytes"
        )));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| {
            crate::error::NexusError::ScraperTransport(format!(
                "{engine_name} SERP body chunk read failed: {e}"
            ))
        })?;
        if buffer.len() + chunk.len() > MAX_SERP_BYTES {
            return Err(crate::error::NexusError::ScraperTransport(format!(
                "{engine_name} SERP response body exceeds maximum limit of {MAX_SERP_BYTES} bytes"
            )));
        }
        buffer.extend_from_slice(&chunk);
    }

    String::from_utf8(buffer).map_err(|e| {
        crate::error::NexusError::ScraperTransport(format!(
            "{engine_name} SERP body invalid UTF-8: {e}"
        ))
    })
}
