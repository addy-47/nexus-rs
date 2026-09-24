use scraper::{ElementRef, Html};
use url::Url;

use super::helpers::{cached_selector, element_text, read_serp_html};
use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

/// Observed working Nokia mobile User-Agent profile for no-JS WML endpoint.
pub const NOKIA_USER_AGENT: &str =
    "Nokia6230/2.0 (03.15) Profile/MIDP-2.0 Configuration/CLDC-1.1";

/// Queries Google's keyless mobile no-JS endpoint.
pub async fn query_google_wml(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let mut request = client
        .get("https://www.google.com/wml/search")
        .header("User-Agent", NOKIA_USER_AGENT)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .query(&[
            ("q", query),
            ("sca_esv", "1"),
            ("hl", "en"),
            ("ie", "utf-8"),
            ("oe", "utf-8"),
        ]);

    match time_filter {
        TimeFilter::Day => request = request.query(&[("tbs", "qdr:d")]),
        TimeFilter::Week => request = request.query(&[("tbs", "qdr:w")]),
        TimeFilter::Month => request = request.query(&[("tbs", "qdr:m")]),
        TimeFilter::Year => request = request.query(&[("tbs", "qdr:y")]),
        TimeFilter::Any => {}
    }

    let response = request
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Google WML request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "Google WML returned status {status}"
        )));
    }

    let html = read_serp_html(response, "GoogleWml").await?;
    parse_google_wml_html(&html)
}

/// Parses Google mobile WML SERP HTML document into structured hits.
pub fn parse_google_wml_html(html: &str) -> Result<Vec<EngineHit>, NexusError> {
    if is_google_challenge(html) {
        return Err(NexusError::SerpParse {
            engine: "google_wml".to_string(),
            message: "Bot challenge or CAPTCHA encountered on Google WML".to_string(),
        });
    }

    let doc = Html::parse_document(html);
    let mut hits = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    let block_sel = cached_selector("div.zMzFAb");
    let link_sel = cached_selector("a.fuLhoc[href]");
    let title_sel = cached_selector("span.CVA68e");
    let snippet_sel = cached_selector("div.taTFJ span.FrIlee");

    for block in doc.select(&block_sel) {
        let Some(a) = block.select(&link_sel).next() else {
            continue;
        };
        let Some(t) = a.select(&title_sel).next() else {
            continue;
        };
        let body = block
            .select(&snippet_sel)
            .map(|s| element_text(s, " "))
            .collect::<Vec<_>>()
            .join(" ");

        push_hit(&mut hits, &mut seen_urls, a, element_text(t, " "), body);
    }

    // Structural fallback: div followed by snippet table if primary selectors shifted
    if hits.is_empty() {
        let div_sel = cached_selector("div");
        let span_sel = cached_selector("span");
        let table_sel = cached_selector("table");

        for block in doc.select(&div_sel) {
            let children: Vec<_> = block.children().filter_map(ElementRef::wrap).collect();
            if children.len() < 2
                || children[0].value().name() != "div"
                || children[1].value().name() != "div"
                || children[1].select(&table_sel).next().is_none()
            {
                continue;
            }
            let Some(a) = children[0]
                .children()
                .filter_map(ElementRef::wrap)
                .find(|el| el.value().name() == "a" && el.value().attr("href").is_some())
            else {
                continue;
            };
            let Some(t) = a.select(&span_sel).next() else {
                continue;
            };
            push_hit(
                &mut hits,
                &mut seen_urls,
                a,
                element_text(t, " "),
                element_text(children[1], " "),
            );
        }
    }

    Ok(hits)
}

/// Helper to extract, sanitize destination target URL, and record an EngineHit.
fn push_hit(
    hits: &mut Vec<EngineHit>,
    seen: &mut std::collections::HashSet<String>,
    anchor: ElementRef<'_>,
    title: String,
    snippet: String,
) {
    let Some(href) = anchor.value().attr("href") else {
        return;
    };
    let Some(target_url) = extract_target_url(href) else {
        return;
    };

    if title.trim().is_empty() || !seen.insert(target_url.clone()) {
        return;
    }

    let display_url = Url::parse(&target_url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_else(|| target_url.clone());

    hits.push(EngineHit::new(
        title,
        target_url,
        display_url,
        snippet,
        Engine::GoogleWml,
    ));
}

/// Unescapes wrapped `/url?q=...` or resolves relative destination links.
pub fn extract_target_url(href: &str) -> Option<String> {
    if !(href.starts_with("https://") || href.starts_with("http://") || href.starts_with("/url?")) {
        return None;
    }
    let base = Url::parse("https://www.google.com").ok()?;
    let wrapped = base.join(href).ok()?;

    let target = if matches!(wrapped.host_str(), Some("google.com" | "www.google.com"))
        && wrapped.path() == "/url"
    {
        let (_, value) = wrapped
            .query_pairs()
            .find(|(key, _)| key == "q" || key == "url")?;
        Url::parse(&value).ok()?
    } else {
        wrapped
    };

    if !matches!(target.scheme(), "http" | "https")
        || target.host_str().is_none()
        || !target.username().is_empty()
        || target.password().is_some()
    {
        return None;
    }

    if matches!(target.host_str(), Some("google.com" | "www.google.com"))
        && matches!(
            target.path(),
            "/search" | "/wml/search" | "/url" | "/preferences" | "/sorry" | "/sorry/"
        )
    {
        return None;
    }

    Some(target.into())
}

/// Detects Google anti-bot challenges and CAPTCHAs in response body.
fn is_google_challenge(html: &str) -> bool {
    (html.len() < 4000 && html.contains("/sorry/"))
        || html.contains("id=\"captcha-form\"")
        || html.contains("id='captcha-form'")
        || html.contains("consent.google.")
        || html.contains("Your browser isn't supported any more")
}
