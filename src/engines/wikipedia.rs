use serde::Deserialize;

use crate::error::NexusError;
use crate::model::{Engine, EngineHit};

const WIKI_API_URL: &str = "https://en.wikipedia.org/w/rest.php/v1/search/page";

#[derive(Debug, Deserialize)]
struct WikiResponse {
    #[serde(default)]
    pages: Vec<WikiPage>,
}

#[derive(Debug, Deserialize)]
struct WikiPage {
    #[serde(default)]
    key: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    excerpt: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Queries Wikipedia REST search endpoint via primp client.
pub async fn query_wikipedia(
    client: &primp::Client,
    query: &str,
) -> Result<Vec<EngineHit>, NexusError> {
    let response = client
        .get(WIKI_API_URL)
        .query(&[("q", query), ("limit", "5")])
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Wikipedia request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "Wikipedia returned status {status}"
        )));
    }

    let json_bytes = response
        .bytes()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("Failed to read Wikipedia body: {e}")))?;

    parse_wikipedia_json(&json_bytes)
}

/// Parses Wikipedia REST search JSON payload into structured engine hits.
pub fn parse_wikipedia_json(json_bytes: &[u8]) -> Result<Vec<EngineHit>, NexusError> {
    let wiki_resp: WikiResponse = serde_json::from_slice(json_bytes).map_err(|e| {
        NexusError::SerpParse {
            engine: "wikipedia".to_string(),
            message: format!("Failed to parse Wikipedia JSON response: {e}"),
        }
    })?;

    let mut hits = Vec::new();
    for page in wiki_resp.pages {
        if page.key.trim().is_empty() {
            continue;
        }

        let target_url = format!("https://en.wikipedia.org/wiki/{}", page.key);
        let display_url = format!("en.wikipedia.org/wiki/{}", page.key);
        let title = if page.title.trim().is_empty() {
            page.key.replace('_', " ")
        } else {
            page.title
        };

        let snippet = page
            .excerpt
            .map(|e| strip_html_tags(&e))
            .filter(|s| !s.trim().is_empty())
            .or(page.description)
            .unwrap_or_default();

        hits.push(EngineHit::new(
            title,
            target_url,
            display_url,
            snippet,
            Engine::Wikipedia,
        ));
    }

    Ok(hits)
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        let sample = "<span>Rust</span> is a <b>programming language</b>.";
        assert_eq!(strip_html_tags(sample), "Rust is a programming language.");
    }
}
