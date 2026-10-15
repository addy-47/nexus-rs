use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

/// Queries a self-hosted SearXNG instance via its JSON API.
///
/// SearXNG aggregates 70+ engines server-side, so one configured instance is
/// a whole fallback tier (jcode-Rust and Hermes both treat it this way).
/// No instance is configured by default; the fanout skips this engine when
/// no URL is provided. Only `http(s)` instance URLs are accepted.
pub async fn query_searxng(
    client: &primp::Client,
    instance_url: &str,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let base = instance_url.trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(NexusError::InvalidConfiguration(
            "SearXNG instance URL must use http or https".to_string(),
        ));
    }
    let endpoint = format!("{base}/search");

    let mut params = vec![
        ("q", query.to_string()),
        ("format", "json".to_string()),
        ("language", "en-US".to_string()),
    ];
    if let Some(range) = time_range(time_filter) {
        params.push(("time_range", range.to_string()));
    }

    let response = client
        .get(&endpoint)
        .query(&params)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("SearXNG request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "SearXNG returned status {status}"
        )));
    }

    let body = response
        .text()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("SearXNG body read failed: {e}")))?;

    parse_searxng_json(&body)
}

/// Maps the recency filter to SearXNG's `time_range` parameter.
fn time_range(filter: TimeFilter) -> Option<&'static str> {
    match filter {
        TimeFilter::Any => None,
        TimeFilter::Day => Some("day"),
        TimeFilter::Week => Some("week"),
        TimeFilter::Month => Some("month"),
        TimeFilter::Year => Some("year"),
    }
}

/// Parses a SearXNG JSON response into structured hits.
pub fn parse_searxng_json(body: &str) -> Result<Vec<EngineHit>, NexusError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| NexusError::SerpParse {
            engine: "searxng".to_string(),
            message: format!("SearXNG JSON parse failed: {e}"),
        })?;

    let results = value
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(results.len());
    for item in results {
        let url = item
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if !url.starts_with("http") {
            continue;
        }
        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() {
            continue;
        }
        let snippet = item
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();

        hits.push(EngineHit::new(
            title,
            url,
            String::new(),
            snippet,
            Engine::Searxng,
        ));
    }

    Ok(hits)
}
