use scraper::Html;
use url::Url;

use super::helpers::{cached_selector, element_text, read_serp_html};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

const DDG_URL: &str = "https://html.duckduckgo.com/html/";

/// Queries DuckDuckGo HTML search endpoint via primp TLS impersonation.
pub async fn query_duckduckgo(
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
        .post(DDG_URL)
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
