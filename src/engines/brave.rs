use scraper::Html;
use url::Url;

use super::helpers::{cached_selector, element_text, read_serp_html};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

const BRAVE_URL: &str = "https://search.brave.com/search";

/// Queries Brave search endpoint via primp TLS impersonation.
pub async fn query_brave(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let mut params = vec![("q", query)];
    match time_filter {
        TimeFilter::Day => params.push(("tf", "pd")),
        TimeFilter::Week => params.push(("tf", "pw")),
        TimeFilter::Month => params.push(("tf", "pm")),
        TimeFilter::Year => params.push(("tf", "py")),
        TimeFilter::Any => {}
    }

    let response = client
        .get(BRAVE_URL)
        .query(&params)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Brave request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "Brave returned status {status}"
        )));
    }

    let html = read_serp_html(response, "Brave").await?;
    parse_brave_html(&html)
}

/// Parses Brave SERP HTML document into structured hits.
pub fn parse_brave_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    let doc = Html::parse_document(html);

    let challenge_sel = cached_selector(
        "form#challenge-form, .cf-browser-verification, div.captcha, #cf-challenge-running",
    );
    if doc.select(&challenge_sel).next().is_some() {
        return Err(NexusError::SerpParse {
            engine: "brave".to_string(),
            message: "Bot challenge encountered on Brave".to_string(),
        });
    }

    let item_sel =
        cached_selector("div.snippet, div[data-type='web'], div[data-type='search'], div.result");
    let link_sel = cached_selector("a.result-header, a.snippet-title, h2 a, a[href]");
    let title_sel = cached_selector(".snippet-title, h2, a.result-header");
    let display_sel = cached_selector(".snippet-url, cite, .result-header cite");
    let snippet_sel = cached_selector(
        ".snippet-description, .snippet-content, p.snippet-description, div.snippet-description",
    );

    let mut hits = Vec::new();
    for entry in doc.select(&item_sel) {
        let Some(link) = entry.select(&link_sel).next() else {
            continue;
        };

        let raw_href = link.value().attr("href").unwrap_or_default().trim();
        if raw_href.is_empty()
            || raw_href.starts_with('/')
            || raw_href.starts_with('#')
            || raw_href.starts_with("javascript:")
        {
            continue;
        }

        let target_url = if let Ok(parsed) = Url::parse(raw_href) {
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                continue;
            }
            parsed.to_string()
        } else {
            continue;
        };

        let title = entry
            .select(&title_sel)
            .next()
            .map(|e| element_text(e, " "))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| element_text(link, " "));

        if title.is_empty() {
            continue;
        }

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
            Engine::Brave,
        ));
    }

    Ok(hits)
}
