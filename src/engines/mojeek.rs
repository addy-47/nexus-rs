use scraper::Html;

use super::helpers::{cached_selector, element_text, read_serp_html};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

/// Queries Mojeek search endpoint via primp TLS impersonation.
pub async fn query_mojeek(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let mut request = client.get("https://www.mojeek.com/search").query(&[("q", query)]);
    match time_filter {
        TimeFilter::Day => request = request.query(&[("t", "1")]),
        TimeFilter::Week => request = request.query(&[("t", "7")]),
        TimeFilter::Month => request = request.query(&[("t", "30")]),
        TimeFilter::Year => request = request.query(&[("t", "365")]),
        TimeFilter::Any => {}
    }
    let response = request
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Mojeek request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "Mojeek returned status {status}"
        )));
    }

    let html = read_serp_html(response, "Mojeek").await?;

    parse_mojeek_html(&html)
}

/// Parses Mojeek SERP HTML document into structured hits.
pub fn parse_mojeek_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    let doc = Html::parse_document(html);

    if is_mojeek_challenge(&doc) {
        return Err(NexusError::SerpParse {
            engine: "mojeek".to_string(),
            message: "Bot challenge encountered on Mojeek".to_string(),
        });
    }

    let item_sel = cached_selector("ul.results-standard > li");
    let link_sel = cached_selector("a.ob");
    let title_sel = cached_selector("h2 a");
    let snippet_sel = cached_selector("p.s");

    let mut hits = Vec::new();
    for item in doc.select(&item_sel) {
        let Some(link) = item.select(&link_sel).next() else {
            continue;
        };

        let target_url = link
            .value()
            .attr("href")
            .unwrap_or_default()
            .trim()
            .to_owned();
        if target_url.is_empty() {
            continue;
        }

        let title = item
            .select(&title_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();

        let snippet = item
            .select(&snippet_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();

        let display_url = url::Url::parse(&target_url)
            .ok()
            .and_then(|p| p.host_str().map(str::to_owned))
            .unwrap_or_default();

        hits.push(EngineHit::new(
            title,
            target_url,
            display_url,
            snippet,
            Engine::Mojeek,
        ));
    }

    Ok(hits)
}

/// Detects if Mojeek SERP returned a CAPTCHA or JS challenge.
fn is_mojeek_challenge(doc: &Html) -> bool {
    let title_sel = cached_selector("head > title");
    let captcha_title = doc.select(&title_sel).any(|title| {
        title
            .text()
            .collect::<String>()
            .trim()
            .eq_ignore_ascii_case("captcha")
    });

    let challenge_sel = cached_selector(".captcha-wrap > p");
    let challenge_msg = doc.select(&challenge_sel).any(|msg| {
        msg.text()
            .collect::<String>()
            .to_lowercase()
            .contains("javascript is required")
    });

    captcha_title && challenge_msg
}
