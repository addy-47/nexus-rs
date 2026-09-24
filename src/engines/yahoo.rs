use scraper::{ElementRef, Html, Selector};
use url::Url;

use super::helpers::{cached_selector, element_text};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

/// Queries Yahoo search endpoint via primp TLS impersonation.
pub async fn query_yahoo(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let mut params = vec![("p", query), ("ei", "UTF-8")];
    if let Some(code) = time_filter.code() {
        params.push(("btf", code));
    }

    let response = client
        .get("https://search.yahoo.com/search")
        .query(&params)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Yahoo request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "Yahoo returned status {status}"
        )));
    }

    let html = response
        .text()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Yahoo body read failed: {e}")))?;

    parse_yahoo_html(&html)
}

/// Parses Yahoo SERP HTML document into structured hits.
pub fn parse_yahoo_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    let doc = Html::parse_document(html);
    let primary_sel = cached_selector("div.dd.algo");
    let fallback_sel = cached_selector("div.compTitle");

    let entries: Vec<_> = doc.select(&primary_sel).collect();
    let entries = if entries.is_empty() {
        doc.select(&fallback_sel).collect()
    } else {
        entries
    };

    let title_link_sel = cached_selector(".compTitle > a, h3 a");
    let title_sel = cached_selector("h3");
    let snippet_sel = cached_selector(".compText p, .compText");

    let mut hits = Vec::new();
    for entry in entries {
        let is_title_only = entry.value().classes().any(|class| class == "compTitle");
        let container = if is_title_only {
            yahoo_result_container(entry, &snippet_sel)
        } else {
            entry
        };

        let Some(link) = entry
            .select(&title_link_sel)
            .next()
            .or_else(|| container.select(&title_link_sel).next())
        else {
            continue;
        };

        let raw_href = link.value().attr("href").unwrap_or_default();
        let target_url = decode_yahoo_url(raw_href);
        if target_url.is_empty() {
            continue;
        }

        let title = link
            .value()
            .attr("aria-label")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                link.select(&title_sel)
                    .next()
                    .or_else(|| entry.select(&title_sel).next())
                    .or_else(|| container.select(&title_sel).next())
                    .map_or_else(|| element_text(link, " "), |v| element_text(v, " "))
            });

        let display_url = Url::parse(&target_url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            .unwrap_or_default();

        let snippet = container
            .select(&snippet_sel)
            .next()
            .map(|e| element_text(e, " "))
            .unwrap_or_default();

        hits.push(EngineHit::new(
            title,
            target_url,
            display_url,
            snippet,
            Engine::Yahoo,
        ));
    }

    Ok(hits)
}

/// Walks up DOM parents to locate the snippet container for title-only nodes.
fn yahoo_result_container<'a>(entry: ElementRef<'a>, snippet: &Selector) -> ElementRef<'a> {
    let mut container = entry;
    for _ in 0..5 {
        let Some(parent) = container.parent().and_then(ElementRef::wrap) else {
            break;
        };
        container = parent;
        if container.select(snippet).next().is_some() {
            break;
        }
    }
    container
}

/// Decodes Yahoo tracking redirection URLs.
fn decode_yahoo_url(value: &str) -> String {
    let Some(encoded) = value
        .split("/RU=")
        .nth(1)
        .and_then(|tail| tail.split("/RK=").next())
    else {
        return value.to_owned();
    };

    url::form_urlencoded::parse(encoded.as_bytes())
        .next()
        .map(|(decoded, _)| decoded.into_owned())
        .unwrap_or_else(|| percent_decode(encoded))
}

/// Fallback percent decoder for non-form-encoded strings.
fn percent_decode(value: &str) -> String {
    let with_prefix = format!("x={value}");
    url::form_urlencoded::parse(with_prefix.as_bytes())
        .next()
        .map(|(_, val)| val.into_owned())
        .unwrap_or_else(|| value.to_owned())
}
