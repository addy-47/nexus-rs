//! Keyless JSON verticals: authoritative APIs that join the same fanout as
//! scraped engines. Each vertical fires only when the query looks like its
//! material (whole-token phrase gates — substring matching lit "war" up on
//! "software"). General queries stay on web engines; no lane is burned on
//! irrelevant verticals. Endpoint and parsing contracts mirror DonSeTch's
//! `verticals.rs` (verified live 2026-10-15); gate predicates are our own.

use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

/// Splits a query into alphanumeric tokens, keeping '+' so "c++" survives.
fn tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '+')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// True when `signal` occurs as a contiguous run of whole tokens.
fn has_phrase(toks: &[String], signal: &str) -> bool {
    let want: Vec<String> = tokens(signal);
    !want.is_empty() && toks.windows(want.len()).any(|w| w == want.as_slice())
}

fn any_phrase(toks: &[String], query: &str, signals: &[&str]) -> bool {
    signals.iter().any(|s| has_phrase(toks, s))
        || signals.iter().any(|s| {
            // Single '?'-style markers are checked against the raw query.
            s.len() == 1 && query.contains(s)
        })
}

/// Whether a vertical engine should join the fanout for this query.
/// Web (scraped) engines always fire; verticals are gated.
pub fn should_query(engine: Engine, query: &str) -> bool {
    let q = query.to_ascii_lowercase();
    let toks = tokens(&q);
    match engine {
        Engine::Arxiv | Engine::Scholar => any_phrase(
            &toks,
            &q,
            &[
                "paper",
                "papers",
                "arxiv",
                "study",
                "studies",
                "research",
                "survey",
                "benchmark",
                "benchmarks",
                "dataset",
                "preprint",
                "doi",
                "sota",
            ],
        ),
        Engine::NewsRss => any_phrase(
            &toks,
            &q,
            &[
                "breaking",
                "today",
                "yesterday",
                "this week",
                "election",
                "elections",
                "headlines",
                "latest news",
            ],
        ),
        Engine::StackExchange => any_phrase(
            &toks,
            &q,
            &[
                "how",
                "why",
                "error",
                "exception",
                "failed",
                "cannot",
                "undefined",
                "fix",
                "debug",
                "traceback",
                "panic",
                "bug",
                "?",
            ],
        ),
        Engine::Mdn => any_phrase(
            &toks,
            &q,
            &[
                "html",
                "css",
                "javascript",
                "typescript",
                "dom",
                "fetch",
                "cors",
                "flexbox",
                "grid",
                "http",
                "cookie",
                "localstorage",
                "webpack",
                "vite",
                "react",
                "vue",
                "svelte",
                "node",
                "npm",
                "wasm",
                "webassembly",
                "browser",
            ],
        ),
        Engine::Github => any_phrase(
            &toks,
            &q,
            &[
                "error",
                "exception",
                "failed",
                "bug",
                "crash",
                "panic",
                "cannot",
                "undefined",
                "library",
                "crate",
                "framework",
                "plugin",
                "sdk",
                "cli",
                "tool",
                "extension",
                "vs code",
            ],
        ),
        Engine::HackerNews => any_phrase(
            &toks,
            &q,
            &[
                "release",
                "announce",
                "announced",
                "launch",
                "launched",
                "startup",
                "funding",
                "ycombinator",
                "ipo",
                "breaking",
                "today",
            ],
        ),
        _ => true,
    }
}

/// Dispatches a vertical engine to its API.
pub async fn query_vertical(
    client: &primp::Client,
    engine: Engine,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    match engine {
        Engine::Arxiv => query_arxiv(client, query).await,
        Engine::Scholar => query_scholar(client, query).await,
        Engine::StackExchange => query_stackexchange(client, query).await,
        Engine::Mdn => query_mdn(client, query).await,
        Engine::Github => query_github(client, query).await,
        Engine::HackerNews => query_hackernews(client, query).await,
        Engine::NewsRss => query_news_rss(client, query, time_filter).await,
        _ => Err(NexusError::InvalidConfiguration(format!(
            "{engine} is not a vertical engine"
        ))),
    }
}

fn form_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

async fn get_json(
    client: &primp::Client,
    engine: &str,
    url: &str,
    extra_headers: &[(&str, &str)],
) -> Result<serde_json::Value, NexusError> {
    let mut req = client.get(url);
    for (k, v) in extra_headers {
        req = req.header(*k, *v);
    }
    let response = req
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("{engine} request failed: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "{engine} returned status {status}"
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("{engine} body read failed: {e}")))?;
    serde_json::from_str(&body).map_err(|e| NexusError::SerpParse {
        engine: engine.to_string(),
        message: format!("{engine} JSON parse failed: {e}"),
    })
}

async fn query_arxiv(client: &primp::Client, query: &str) -> Result<Vec<EngineHit>, NexusError> {
    let url = format!(
        "https://export.arxiv.org/api/query?search_query=all:{}&max_results=5&sortBy=relevance",
        form_encode(query)
    );
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("arxiv request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "arxiv returned status {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("arxiv body read failed: {e}")))?;
    Ok(parse_arxiv(&body))
}

/// Parses an arXiv Atom feed via tag scanning (feeds are regular enough;
/// no XML dependency needed).
pub fn parse_arxiv(body: &str) -> Vec<EngineHit> {
    let mut hits = Vec::new();
    for (rank, entry) in body.split("<entry>").skip(1).enumerate() {
        if rank >= 5 {
            break;
        }
        let grab = |tag: &str| -> String {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            entry
                .split_once(&open)
                .and_then(|(_, rest)| rest.split_once(&close))
                .map(|(inner, _)| inner.trim().to_string())
                .unwrap_or_default()
        };
        let title: String = grab("title")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let url = grab("id");
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let summary: String = grab("summary")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(300)
            .collect();
        hits.push(EngineHit::new(
            title,
            url,
            "arxiv.org".to_string(),
            summary,
            Engine::Arxiv,
        ));
    }
    hits
}

async fn query_scholar(client: &primp::Client, query: &str) -> Result<Vec<EngineHit>, NexusError> {
    let url = format!(
        "https://api.semanticscholar.org/graph/v1/paper/search?query={}&limit=5&fields=title,url,abstract,year",
        form_encode(query)
    );
    let value = get_json(client, "scholar", &url, &[]).await?;
    let items = value
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(items.len());
    for (rank, item) in items.iter().enumerate() {
        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim();
        let url = item
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let year = item
            .get("year")
            .and_then(|y| y.as_i64())
            .map(|y| y.to_string())
            .unwrap_or_default();
        let abs: String = item
            .get("abstract")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .chars()
            .take(220)
            .collect();
        let _ = rank;
        hits.push(EngineHit::new(
            title,
            url,
            "semanticscholar.org".to_string(),
            format!("{abs} ({year})"),
            Engine::Scholar,
        ));
    }
    Ok(hits)
}

async fn query_stackexchange(
    client: &primp::Client,
    query: &str,
) -> Result<Vec<EngineHit>, NexusError> {
    let url = format!(
        "https://api.stackexchange.com/2.3/search/advanced?order=desc&sort=relevance&q={}&site=stackoverflow&pagesize=5",
        form_encode(query)
    );
    let value = get_json(client, "stackexchange", &url, &[]).await?;
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(items.len());
    for item in items {
        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim();
        let url = item
            .get("link")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let score = item.get("score").and_then(|s| s.as_i64()).unwrap_or(0);
        let answers = item
            .get("answer_count")
            .and_then(|a| a.as_i64())
            .unwrap_or(0);
        let answered = item
            .get("is_answered")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);
        hits.push(EngineHit::new(
            title,
            url,
            "stackoverflow.com".to_string(),
            format!(
                "Stack Overflow: score {score}, {answers} answers{}",
                if answered { ", accepted answer" } else { "" }
            ),
            Engine::StackExchange,
        ));
    }
    Ok(hits)
}

async fn query_mdn(client: &primp::Client, query: &str) -> Result<Vec<EngineHit>, NexusError> {
    let url = format!(
        "https://developer.mozilla.org/api/v1/search?q={}",
        form_encode(query)
    );
    let value = get_json(client, "mdn", &url, &[]).await?;
    let docs = value
        .get("documents")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(docs.len());
    for doc in docs {
        let title = doc
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim();
        let path = doc
            .get("mdn_url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() || path.is_empty() {
            continue;
        }
        let summary = doc
            .get("summary")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim();
        hits.push(EngineHit::new(
            title,
            format!("https://developer.mozilla.org{path}"),
            "developer.mozilla.org".to_string(),
            summary,
            Engine::Mdn,
        ));
    }
    Ok(hits)
}

async fn query_github(client: &primp::Client, query: &str) -> Result<Vec<EngineHit>, NexusError> {
    // Error-shaped queries belong on issue search; repo-shaped on repository
    // search; natural-language how-tos return spam repos and stay out
    // (should_query already gated, this picks the lane).
    let q = query.to_ascii_lowercase();
    let toks = tokens(&q);
    let errorish = [
        "error",
        "exception",
        "failed",
        "bug",
        "crash",
        "panic",
        "cannot",
        "undefined",
    ]
    .iter()
    .any(|s| has_phrase(&toks, s));
    let endpoint = if errorish { "issues" } else { "repositories" };
    let url = format!(
        "https://api.github.com/search/{endpoint}?q={}&per_page=5",
        form_encode(query)
    );
    let value = get_json(
        client,
        "github",
        &url,
        &[
            ("User-Agent", "nexus-rs"),
            ("Accept", "application/vnd.github+json"),
        ],
    )
    .await?;
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(items.len());
    for item in items {
        let html_url = item
            .get("html_url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if !html_url.starts_with("http") {
            continue;
        }
        if item.get("stargazers_count").is_some_and(|s| s.is_number()) {
            let stars = item
                .get("stargazers_count")
                .and_then(|s| s.as_i64())
                .unwrap_or(0);
            let desc = item
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let name = item.get("full_name").and_then(|n| n.as_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            hits.push(EngineHit::new(
                name,
                html_url,
                "github.com".to_string(),
                format!("{desc} (★ {stars})"),
                Engine::Github,
            ));
        } else {
            let repo = item
                .get("repository_url")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .replace("https://api.github.com/repos/", "");
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
            if title.is_empty() {
                continue;
            }
            let state = item.get("state").and_then(|s| s.as_str()).unwrap_or("");
            hits.push(EngineHit::new(
                format!("{repo}: {title}"),
                html_url,
                "github.com".to_string(),
                format!("GitHub issue ({state})"),
                Engine::Github,
            ));
        }
    }
    Ok(hits)
}

async fn query_hackernews(
    client: &primp::Client,
    query: &str,
) -> Result<Vec<EngineHit>, NexusError> {
    let url = format!(
        "https://hn.algolia.com/api/v1/search?query={}&hitsPerPage=5",
        form_encode(query)
    );
    let value = get_json(client, "hackernews", &url, &[]).await?;
    let items = value
        .get("hits")
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(items.len());
    for item in items {
        let url = item
            .get("url")
            .and_then(|u| u.as_str())
            .or_else(|| item.get("story_url").and_then(|u| u.as_str()))
            .unwrap_or("")
            .trim();
        let url = if url.is_empty() {
            match item.get("objectID").and_then(|o| o.as_str()) {
                Some(id) => format!("https://news.ycombinator.com/item?id={id}"),
                None => continue,
            }
        } else {
            url.to_string()
        };
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
        let points = item.get("points").and_then(|p| p.as_i64()).unwrap_or(0);
        let comments = item
            .get("num_comments")
            .and_then(|c| c.as_i64())
            .unwrap_or(0);
        hits.push(EngineHit::new(
            title,
            url,
            "news.ycombinator.com".to_string(),
            format!("Hacker News: {points} points, {comments} comments"),
            Engine::HackerNews,
        ));
    }
    Ok(hits)
}

async fn query_news_rss(
    client: &primp::Client,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    let mut url = format!(
        "https://news.google.com/rss/search?q={}&hl=en-US&gl=US&ceid=US:en",
        form_encode(query)
    );
    match time_filter {
        TimeFilter::Day => url.push_str(" when:1d"),
        TimeFilter::Week => url.push_str(" when:7d"),
        TimeFilter::Month => url.push_str(" when:30d"),
        TimeFilter::Year => url.push_str(" when:365d"),
        TimeFilter::Any => {}
    }
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("news rss request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(NexusError::ScraperTransport(format!(
            "news rss returned status {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| NexusError::ScraperTransport(format!("news rss body read failed: {e}")))?;
    Ok(parse_news_rss(&body))
}

/// Parses a Google News RSS feed via tag scanning (items are regular enough;
/// no XML dependency needed).
pub fn parse_news_rss(body: &str) -> Vec<EngineHit> {
    let mut hits = Vec::new();
    for item in body.split("<item>").skip(1).take(8) {
        let grab = |tag: &str| -> String {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            item.split_once(&open)
                .and_then(|(_, rest)| rest.split_once(&close))
                .map(|(inner, _)| {
                    inner
                        .trim_start_matches("<![CDATA[")
                        .trim_end_matches("]]>")
                        .trim()
                        .to_string()
                })
                .unwrap_or_default()
        };
        let title = grab("title");
        let url = grab("link");
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        // Google News titles end in " - Publisher"; the publisher plus the
        // date says more than a bare date as a snippet.
        let publisher = title
            .rsplit_once(" - ")
            .map(|(_, p)| p.trim().to_string())
            .unwrap_or_default();
        let date = grab("pubDate");
        let snippet = if publisher.is_empty() {
            date
        } else if date.is_empty() {
            publisher
        } else {
            format!("{publisher} · {date}")
        };
        hits.push(EngineHit::new(
            title,
            url,
            String::new(),
            snippet,
            Engine::NewsRss,
        ));
    }
    hits
}
