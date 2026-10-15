use scraper::Html;
use url::Url;

use super::helpers::{cached_selector, element_text, read_serp_html};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DDG_LITE_URL: &str = "https://lite.duckduckgo.com/lite/";

/// Queries DuckDuckGo with the Lite endpoint as primary and HTML as fallback.
///
/// The HTML endpoint serves CAPTCHA challenges to datacenter/proxy IPs
/// (DonSeTch, jcode-Rust, and production Vox logs agree). Lite is a lighter
/// table endpoint with the same redirect-wrapped links. HTML POST (the way the
/// real page submits) remains as fallback when Lite errors or yields nothing.
pub async fn query_duckduckgo(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    match query_duckduckgo_lite(client, query).await {
        Ok(hits) if !hits.is_empty() => Ok(hits),
        Ok(_) => {
            log::info!(
                "[Nexus::Engines] DuckDuckGo Lite returned zero hits; falling back to HTML endpoint"
            );
            query_duckduckgo_html(client, query, time_filter).await
        }
        Err(e) => {
            log::info!(
                "[Nexus::Engines] DuckDuckGo Lite failed ({e}); falling back to HTML endpoint"
            );
            query_duckduckgo_html(client, query, time_filter).await
        }
    }
}

/// Queries the DuckDuckGo Lite endpoint via primp TLS impersonation.
async fn query_duckduckgo_lite(
    client: &primp::Client,
    query: &str,
) -> Result<Vec<EngineHit>, NexusError> {
    let response = client
        .get(DDG_LITE_URL)
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|e| {
            NexusError::ScraperTransport(format!("DuckDuckGo Lite request failed: {e}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "DuckDuckGo Lite returned status {status}"
        )));
    }

    let html = read_serp_html(response, "DuckDuckGo Lite").await?;

    parse_duckduckgo_lite_html(&html)
}

/// Queries DuckDuckGo HTML search endpoint via primp TLS impersonation.
async fn query_duckduckgo_html(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let mut form_params = vec![("q", query)];
    if let Some(code) = time_filter.code() {
        form_params.push(("df", code));
    }

    let body = {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.extend_pairs(form_params);
        serializer.finish()
    };

    let response = client
        .post(DDG_HTML_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("DuckDuckGo request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "DuckDuckGo returned status {status}"
        )));
    }

    let html = read_serp_html(response, "DuckDuckGo").await?;

    parse_duckduckgo_html(&html)
}

/// Parses DuckDuckGo Lite SERP HTML document into structured hits.
///
/// Lite renders a result table: `a.result-link` anchors carry title and URL,
/// `td.result-snippet` cells carry snippets in the same order.
pub fn parse_duckduckgo_lite_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    if is_challenge_page(html) {
        return Err(NexusError::SerpParse {
            engine: "duckduckgo".to_string(),
            message: "Bot challenge encountered on DuckDuckGo Lite".to_string(),
        });
    }

    let doc = Html::parse_document(html);
    let link_sel = cached_selector("a.result-link");
    let snippet_sel = cached_selector("td.result-snippet");
    let snippet_vec: Vec<String> = doc
        .select(&snippet_sel)
        .map(|e| element_text(e, " "))
        .collect();

    let mut hits = Vec::new();
    for (rank, link) in doc.select(&link_sel).enumerate() {
        let raw_href = link.value().attr("href").unwrap_or_default();
        let target_url = decode_duckduckgo_url(raw_href);
        if !target_url.starts_with("http") {
            continue;
        }

        let title = element_text(link, " ");
        let snippet = snippet_vec.get(rank).cloned().unwrap_or_default();

        hits.push(EngineHit::new(
            title,
            target_url,
            String::new(),
            snippet,
            Engine::Duckduckgo,
        ));
    }

    Ok(hits)
}

/// Parses DuckDuckGo SERP HTML document into structured hits.
pub fn parse_duckduckgo_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    let doc = Html::parse_document(html);

    let challenge_sel =
        cached_selector("form#challenge-form, form[action*='anomaly.js'], .anomaly-modal");
    if doc.select(&challenge_sel).next().is_some() {
        return Err(NexusError::SerpParse {
            engine: "duckduckgo".to_string(),
            message: "Bot challenge encountered on DuckDuckGo".to_string(),
        });
    }

    let item_sel = cached_selector("div.result.results_links.results_links_deep.web-result");
    let title_sel = cached_selector("h2.result__title a.result__a");
    let display_sel = cached_selector("a.result__url");
    let snippet_sel = cached_selector("a.result__snippet");

    let mut hits = Vec::new();
    for entry in doc.select(&item_sel) {
        let Some(title_link) = entry.select(&title_sel).next() else {
            continue;
        };

        let raw_href = title_link.value().attr("href").unwrap_or_default();
        let target_url = decode_duckduckgo_url(raw_href);
        if target_url.is_empty() {
            continue;
        }

        let title = element_text(title_link, " ");
        let display_url = entry
            .select(&display_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();
        let snippet = entry
            .select(&snippet_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();

        hits.push(EngineHit::new(
            title,
            target_url,
            display_url,
            snippet,
            Engine::Duckduckgo,
        ));
    }

    Ok(hits)
}

/// Cheap pre-parse challenge check for the Lite endpoint.
fn is_challenge_page(html: &str) -> bool {
    let window: String = html.chars().take(4000).collect();
    let lower = window.to_ascii_lowercase();
    lower.contains("anomaly") || lower.contains("captcha") || lower.contains("challenge-form")
}

/// Decodes redirection wrapper from DuckDuckGo search links.
fn decode_duckduckgo_url(raw_href: &str) -> String {
    let trimmed = raw_href.trim();
    if !trimmed.contains("uddg=") {
        return trimmed.to_owned();
    }

    let url_str = if trimmed.starts_with("//") {
        format!("https:{trimmed}")
    } else {
        trimmed.to_owned()
    };

    if let Ok(parsed) = Url::parse(&url_str) {
        for (k, v) in parsed.query_pairs() {
            if k == "uddg" {
                return v.into_owned();
            }
        }
    }

    trimmed.to_owned()
}
