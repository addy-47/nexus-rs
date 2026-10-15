//! ============================================================================
//! verticals_test.rs — Gate predicates + XML-lite parsers for keyless verticals
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::engines::verticals
//! Prerequisites: None (hermetic; no network — synthetic feeds only)
//! Execution    : CARGO_BUILD_JOBS=1 cargo test --test verticals_test
//! Metrics      : Gate true/false per query shape, parser field contracts
//! ============================================================================
//!
//! Public seams under test: `should_query`, `parse_arxiv`, `parse_news_rss`.
//! The JSON verticals (scholar/stackexchange/mdn/github/hackernews) parse
//! inline inside private `query_*` fns behind live HTTP, so they are covered
//! here only through their gate predicates, not their parsers.

use nexus::Engine;
use nexus::engines::verticals::{parse_arxiv, parse_news_rss, should_query};

/// All seven gated vertical engines.
fn all_verticals() -> [Engine; 7] {
    [
        Engine::Arxiv,
        Engine::Scholar,
        Engine::StackExchange,
        Engine::Mdn,
        Engine::Github,
        Engine::HackerNews,
        Engine::NewsRss,
    ]
}

fn assert_fires(query: &str, engines: &[Engine]) {
    for e in engines {
        assert!(should_query(*e, query), "{e:?} should fire for {query:?}");
    }
}

fn assert_silent(query: &str, engines: &[Engine]) {
    for e in engines {
        assert!(
            !should_query(*e, query),
            "{e:?} should NOT fire for {query:?}"
        );
    }
}

fn others_than(firing: &[Engine]) -> Vec<Engine> {
    all_verticals()
        .into_iter()
        .filter(|e| !firing.contains(e))
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Gate routing per query shape
// ---------------------------------------------------------------------------

#[test]
fn academic_query_fires_arxiv_and_scholar_only() {
    let q = "survey paper on diffusion models benchmark dataset";
    assert_fires(q, &[Engine::Arxiv, Engine::Scholar]);
    assert_silent(
        q,
        &[
            Engine::Mdn,
            Engine::NewsRss,
            Engine::StackExchange,
            Engine::Github,
            Engine::HackerNews,
        ],
    );
}

#[test]
fn gates_are_case_insensitive() {
    assert!(should_query(
        Engine::Arxiv,
        "PAPER on quantum error correction"
    ));
    assert!(should_query(Engine::Mdn, "FETCH API CORS"));
}

#[test]
fn howto_query_fires_stackexchange_not_mdn() {
    let q = "how do I fix python traceback";
    assert_fires(q, &[Engine::StackExchange]);
    // "fix" is a StackExchange-only signal; Github's list has no "fix".
    assert_silent(
        q,
        &[Engine::Mdn, Engine::Github, Engine::Arxiv, Engine::NewsRss],
    );
}

#[test]
fn web_platform_query_fires_mdn() {
    let q = "fetch API cors";
    assert_fires(q, &[Engine::Mdn]);
    assert_silent(q, &others_than(&[Engine::Mdn]));
}

#[test]
fn panic_query_fires_github_and_stackexchange() {
    let q = "tokio panic";
    assert_fires(q, &[Engine::Github, Engine::StackExchange]);
    assert_silent(q, &others_than(&[Engine::Github, Engine::StackExchange]));
}

#[test]
fn startup_query_fires_hackernews_but_not_newsrss() {
    // NOTE (spec mismatch in task brief): the brief claims
    // "startup funding announced" fires HackerNews+NewsRss, but the NewsRss
    // gate list (breaking/today/yesterday/this week/election/headlines/
    // latest news) contains none of startup/funding/announced, so NewsRss
    // is correctly silent here. The companion test below shows the query
    // shape that fires both.
    let q = "startup funding announced";
    assert_fires(q, &[Engine::HackerNews]);
    assert!(!should_query(Engine::NewsRss, q));
}

#[test]
fn startup_query_with_today_fires_hackernews_and_newsrss() {
    let q = "startup funding announced today";
    assert_fires(q, &[Engine::HackerNews, Engine::NewsRss]);
}

#[test]
fn breaking_election_query_fires_newsrss() {
    let q = "breaking election results today";
    assert_fires(q, &[Engine::NewsRss]);
    // "breaking"/"today" are also HackerNews signals, so HN fires too —
    // entailed by the gate lists, not a defect.
    assert_fires(q, &[Engine::HackerNews]);
    assert_silent(
        q,
        &[
            Engine::Arxiv,
            Engine::Scholar,
            Engine::StackExchange,
            Engine::Mdn,
            Engine::Github,
        ],
    );
}

#[test]
fn plain_query_fires_no_vertical() {
    let q = "capital city of Australia";
    assert_silent(q, &all_verticals());
}

#[test]
fn web_engines_always_fire_regardless_of_query() {
    for q in [
        "capital city of Australia",
        "survey paper on diffusion models benchmark dataset",
        "",
    ] {
        assert!(should_query(Engine::Duckduckgo, q));
        assert!(should_query(Engine::Bing, q));
    }
}

// ---------------------------------------------------------------------------
// 2. Substring-trap regression (whole-token phrase gates)
// ---------------------------------------------------------------------------

#[test]
fn software_war_fires_nothing() {
    // Historical bug: substring matching lit "war" up inside "software".
    // No gate lists "war" now, and matching is whole-token either way.
    assert_silent("software war", &all_verticals());
}

#[test]
fn wallpaper_alone_does_not_fire_arxiv() {
    // "wallpaper" merely contains "paper"; whole-token match must not fire.
    assert_silent("show me the wallpaper", &all_verticals());
}

#[test]
fn whole_token_paper_correctly_fires_arxiv() {
    // Contrast with the trap above: standalone "paper" IS a signal token.
    let q = "wallpaper paper";
    assert_fires(q, &[Engine::Arxiv, Engine::Scholar]);
    assert_silent(q, &others_than(&[Engine::Arxiv, Engine::Scholar]));
}

#[test]
fn show_does_not_fire_how_gate() {
    // "show" contains "how" as a substring; whole-token gate stays silent.
    assert!(!should_query(Engine::StackExchange, "show me this photo"));
}

#[test]
fn doing_does_not_fire_doi_gate() {
    // "doing" contains "doi" as a substring; Arxiv must stay silent.
    assert!(!should_query(Engine::Arxiv, "doing homework"));
    assert!(!should_query(Engine::Scholar, "doing homework"));
}

#[test]
fn prefix_does_not_fire_fix_gate() {
    // "prefix" contains "fix" as a substring; StackExchange stays silent.
    assert!(!should_query(
        Engine::StackExchange,
        "prefix notation explained"
    ));
}

#[test]
fn newspaper_does_not_fire_paper_gate_but_headlines_fire_news() {
    let q = "newspaper headlines today";
    assert!(!should_query(Engine::Arxiv, q));
    assert!(!should_query(Engine::Scholar, q));
    assert!(should_query(Engine::NewsRss, q));
}

#[test]
fn question_mark_fires_stackexchange_via_raw_query_branch() {
    // Single-char "?" signals are checked against the raw query string.
    assert!(should_query(
        Engine::StackExchange,
        "capital city of australia?"
    ));
}

#[test]
fn empty_query_fires_nothing_vertical() {
    assert_silent("", &all_verticals());
}

// ---------------------------------------------------------------------------
// 3. parse_arxiv on synthetic Atom feed
// ---------------------------------------------------------------------------

fn arxiv_feed() -> String {
    let long_summary = "word ".repeat(100); // 500 chars pre-cap
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<entry>
<id>http://arxiv.org/abs/1234.5678v1</id>
<title>
  Deep
  Learning   for
  Search
</title>
<summary>{long_summary}</summary>
</entry>
<entry>
<id>https://arxiv.org/abs/8765.4321v2</id>
<title>Second Paper Title</title>
<summary>A short abstract.</summary>
</entry>
<entry>
<id>/relative/path/not-a-url</id>
<title>Skipped Entry</title>
<summary>Non-http links are dropped.</summary>
</entry>
</feed>"#
    )
}

#[test]
fn parse_arxiv_two_entries_normalized_capped_filtered() {
    let hits = parse_arxiv(&arxiv_feed());
    assert_eq!(hits.len(), 2, "relative-URL entry must be dropped");

    // Entry 1: title whitespace-normalized, abs URL kept, summary capped.
    assert_eq!(hits[0].title, "Deep Learning for Search");
    assert_eq!(hits[0].url, "http://arxiv.org/abs/1234.5678v1");
    assert_eq!(hits[0].display_url, "arxiv.org");
    assert_eq!(hits[0].engine, Engine::Arxiv);
    // 499-char normalized input capped to 300 by the parser; EngineHit::new
    // trims the cut edge, so accept 299 (cut landed on a space) or 300.
    let snip_len = hits[0].snippet.chars().count();
    assert!(snip_len <= 300, "summary must be capped, got {snip_len}");
    assert!(
        snip_len > 250,
        "summary must not be over-truncated, got {snip_len}"
    );
    assert!(
        hits[0]
            .snippet
            .chars()
            .all(|c| !c.is_whitespace() || c == ' ')
    );

    // Entry 2: short summary passes through intact.
    assert_eq!(hits[1].title, "Second Paper Title");
    assert_eq!(hits[1].url, "https://arxiv.org/abs/8765.4321v2");
    assert_eq!(hits[1].snippet, "A short abstract.");
    assert_eq!(hits[1].engine, Engine::Arxiv);
}

#[test]
fn parse_arxiv_empty_body_yields_no_hits() {
    assert!(parse_arxiv("").is_empty());
    assert!(parse_arxiv("<feed></feed>").is_empty());
}

// ---------------------------------------------------------------------------
// 4. parse_news_rss on synthetic RSS
// ---------------------------------------------------------------------------

fn news_feed() -> String {
    r#"<rss version="2.0"><channel>
<item>
<title><![CDATA[Markets rally - Reuters]]></title>
<link>https://example.com/markets</link>
<pubDate>Wed, 15 Oct 2026 12:00:00 GMT</pubDate>
</item>
<item>
<title>JustAHeadline</title>
<link>https://example.com/bare</link>
<pubDate>Thu, 16 Oct 2026 08:00:00 GMT</pubDate>
</item>
<item>
<title>Bad link - Sketchy</title>
<link>ftp://example.com/bad</link>
<pubDate>Thu, 16 Oct 2026 09:00:00 GMT</pubDate>
</item>
</channel></rss>"#
        .to_string()
}

#[test]
fn parse_news_rss_publisher_date_snippet_cdata_and_drop() {
    let hits = parse_news_rss(&news_feed());
    assert_eq!(hits.len(), 2, "non-http link item must be dropped");

    // Item 1: CDATA title kept whole; snippet is "publisher · date".
    assert_eq!(hits[0].title, "Markets rally - Reuters");
    assert_eq!(hits[0].url, "https://example.com/markets");
    assert_eq!(hits[0].snippet, "Reuters · Wed, 15 Oct 2026 12:00:00 GMT");
    assert_eq!(hits[0].engine, Engine::NewsRss);
    assert!(hits[0].display_url.is_empty());

    // Item 2: no " - " separator, so snippet falls back to the bare date.
    assert_eq!(hits[1].title, "JustAHeadline");
    assert_eq!(hits[1].url, "https://example.com/bare");
    assert_eq!(hits[1].snippet, "Thu, 16 Oct 2026 08:00:00 GMT");
}

#[test]
fn parse_news_rss_empty_body_yields_no_hits() {
    assert!(parse_news_rss("").is_empty());
    assert!(parse_news_rss("<rss></rss>").is_empty());
}
