use std::collections::HashMap;

use base64::Engine as _;
use scraper::Html;
use url::Url;

use super::helpers::{cached_selector, element_text, read_serp_html};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit};

/// Queries Bing search endpoint via primp TLS impersonation.
pub async fn query_bing(client: &primp::Client, query: &str) -> Result<Vec<EngineHit>, NexusError> {
    let target_url = {
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        encoded.append_pair("q", query);
        let query_string = encoded.finish().replace('+', "%20");
        format!("https://www.bing.com/search?{query_string}")
    };

    let response = client
        .get(&target_url)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Bing request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "Bing returned status {status}"
        )));
    }

    let html = read_serp_html(response, "Bing").await?;

    parse_bing_html(&html)
}

/// Parses Bing SERP HTML document into structured hits.
pub fn parse_bing_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    let doc = Html::parse_document(html);
    let item_sel = cached_selector("li.b_algo");
    let title_sel = cached_selector("h2 a");
    let snippet_sel = cached_selector(".b_caption p");
    let display_sel = cached_selector(".b_attribution cite, cite");

    let mut hits = Vec::new();
    for entry in doc.select(&item_sel) {
        let Some(title_link) = entry.select(&title_sel).next() else {
            continue;
        };

        let raw_href = title_link.value().attr("href").unwrap_or_default();
        let target_url = decode_bing_url(raw_href);
        if target_url.is_empty() {
            continue;
        }

        let title = element_text(title_link, " ");
        let snippet = entry
            .select(&snippet_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();
        let display_url = entry
            .select(&display_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();

        hits.push(EngineHit::new(
            title,
            target_url,
            display_url,
            snippet,
            Engine::Bing,
        ));
    }

    Ok(hits)
}

/// Decodes Bing tracking and redirection URLs.
fn decode_bing_url(value: &str) -> String {
    if !value.contains("/ck/a") && !value.contains("/cr?") {
        return value.to_owned();
    }

    let Ok(url) = Url::parse(value) else {
        return value.to_owned();
    };

    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    if let Some(target) = params.get("rurl") {
        return target.to_owned();
    }

    let Some(encoded) = params.get("u") else {
        return value.to_owned();
    };

    let encoded_str = encoded.strip_prefix("a1").unwrap_or(encoded);
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_str)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded_str))
        .ok()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .unwrap_or_else(|| value.to_owned())
}
