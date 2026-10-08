//! ============================================================================
//! cleaner_ranking_gates_test.rs — Integration tests for speech-sanitized
//! extraction, title scanning, and Sparse ranking normalization (P0-6/P0-8,
//! char-boundary P1).
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::extraction::cleaner, nexus::ranking
//! Prerequisites: None (hermetic — Sparse mode only, no embedder, no network)
//! Execution    : cargo nextest run --release --test-threads=1 --no-fail-fast
//! ============================================================================
//!
//! Authored by the testing subagent per plan §11.4 (D8). The private
//! `sanitize_for_speech` (link-strip, fence-drop, table-skeleton drop,
//! decode-once) is exercised exclusively through the public `html_to_markdown`.

use nexus::extraction::cleaner::{extract_page_title, html_to_markdown};
use nexus::model::{RankingMode, RankingPolicy, ScoredPassage};
use nexus::ranking::rank_passages;

// ---------------------------------------------------------------------------
// html_to_markdown (observable sanitize_for_speech behavior)
// ---------------------------------------------------------------------------

#[test]
fn t_cleaner_prose_html_survives() {
    let html = "<html><head><title>Guide</title></head><body><main>\
        <h1>Core Ownership Principles</h1>\
        <p>Rust enforces memory safety without garbage collection.</p>\
        </main></body></html>";
    let out = html_to_markdown(html, 10_000).unwrap();
    assert!(!out.is_empty());
    assert!(
        out.contains("Ownership") || out.contains("memory safety"),
        "main content must survive; got: {out:?}"
    );
}

#[test]
fn t_cleaner_link_markup_stripped_url_dropped() {
    // `[label](url)` -> `label`: label kept, raw URL and markdown link
    // syntax gone.
    let html = "<html><body><p>See \
        <a href=\"https://example.com/x\">label</a> now</p></body></html>";
    let out = html_to_markdown(html, 10_000).unwrap();
    assert!(
        out.contains("label"),
        "link label must survive; got: {out:?}"
    );
    assert!(
        !out.contains("example.com/x"),
        "raw link URL must be stripped; got: {out:?}"
    );
    assert!(
        !out.contains("]("),
        "markdown link syntax must be stripped; got: {out:?}"
    );
}

#[test]
fn t_cleaner_fenced_code_leaves_no_fence_markers() {
    let html = "<html><body><article><p>Prose before.</p>\
        <pre><code>fn main() { println!(\"hi\"); }</code></pre>\
        <p>Prose after.</p></article></body></html>";
    let out = html_to_markdown(html, 10_000).unwrap();
    assert!(out.contains("Prose before"), "got: {out:?}");
    assert!(out.contains("Prose after"), "got: {out:?}");
    assert!(
        !out.contains("```"),
        "fence markers must be dropped; got: {out:?}"
    );
}

fn is_skeleton_line(line: &str) -> bool {
    // Mirrors the production table-skeleton predicate; used only to assert
    // the postcondition "no skeleton residue survives".
    line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) && line.contains('|')
}

#[test]
fn t_cleaner_table_keeps_cells_drops_skeleton() {
    let html = "<html><body>\
        <table><tr><th>Name</th><th>Age</th></tr>\
        <tr><td>Ada</td><td>36</td></tr></table>\
        <p>After table.</p></body></html>";
    let out = html_to_markdown(html, 10_000).unwrap();
    assert!(
        out.contains("Ada"),
        "cell content must survive; got: {out:?}"
    );
    assert!(out.contains("After table"), "got: {out:?}");
    assert!(
        !out.lines().any(is_skeleton_line),
        "pipe/dash skeleton rows must be dropped; got: {out:?}"
    );
}

#[test]
fn t_cleaner_entities_decoded_exactly_once() {
    let html = "<html><body><p>Fish &amp; Chips and it&apos;s &quot;quoted&quot;</p></body></html>";
    let out = html_to_markdown(html, 10_000).unwrap();
    assert!(
        out.contains("Fish & Chips") || out.contains("Fish &"),
        "got: {out:?}"
    );
    assert!(out.contains("it's"), "got: {out:?}");
    for raw in ["&amp;", "&apos;", "&quot;"] {
        assert!(
            !out.contains(raw),
            "entity {raw} must be decoded; got: {out:?}"
        );
    }
}

#[test]
fn t_cleaner_heading_and_emphasis_marks_stripped() {
    let html = "<html><body><h1>Big Title</h1>\
        <p>Some <strong>bold</strong> text here</p></body></html>";
    let out = html_to_markdown(html, 10_000).unwrap();
    assert!(out.contains("Big Title"), "got: {out:?}");
    assert!(out.contains("bold"), "got: {out:?}");
    assert!(!out.contains("**"), "emphasis marks stripped; got: {out:?}");
    assert!(
        !out.lines().any(|l| l.starts_with('#')),
        "heading marks stripped; got: {out:?}"
    );
}

#[test]
fn t_cleaner_max_chars_truncates() {
    let para = "lorem ".repeat(500);
    let html = format!("<html><body><p>{para}</p></body></html>");
    let out = html_to_markdown(&html, 50).unwrap();
    assert!(!out.is_empty());
    assert!(
        out.chars().count() <= 50,
        "output must respect max_chars; got {} chars",
        out.chars().count()
    );
}

#[test]
fn t_cleaner_max_chars_zero_yields_empty() {
    let html = "<html><body><p>Some prose content here.</p></body></html>";
    let out = html_to_markdown(html, 0).unwrap();
    assert!(out.is_empty());
}

#[test]
fn t_cleaner_empty_html_yields_empty() {
    let out = html_to_markdown("", 1000).unwrap();
    assert!(out.is_empty());
}

// ---------------------------------------------------------------------------
// extract_page_title (incl. 64KB char-boundary P1)
// ---------------------------------------------------------------------------

#[test]
fn t_title_simple_tag() {
    let html = "<html><head><title>Hello World</title></head><body><p>x</p></body></html>";
    assert_eq!(extract_page_title(html), Some("Hello World".to_string()));
}

#[test]
fn t_title_h1_fallback() {
    let html = "<html><body><h1>Section Head</h1><p>body text</p></body></html>";
    assert_eq!(extract_page_title(html), Some("Section Head".to_string()));
}

#[test]
fn t_title_entity_decoded() {
    let html = "<html><head><title>Fish &amp; Chips</title></head></html>";
    assert_eq!(extract_page_title(html), Some("Fish & Chips".to_string()));
}

#[test]
fn t_title_empty_inputs_none() {
    assert_eq!(extract_page_title(""), None);
    assert_eq!(extract_page_title("   "), None);
}

#[test]
fn t_title_survives_multibyte_char_straddling_64k() {
    // Live P1: a PDF body panicked `&html[..65536]` when byte 65536 landed
    // inside a multi-byte char. Prefix is exactly 65535 bytes so U+0375
    // (2 bytes) spans bytes 65535..65537; the old slice panicked here.
    let prefix = "<title>Early Title</title>";
    assert_eq!(prefix.len(), 26);
    let mut html = String::with_capacity(72_000);
    html.push_str(prefix);
    html.push_str(&"a".repeat(65535 - prefix.len()));
    html.push('\u{0375}'); // ͵ — straddles byte 65536
    html.push_str(&"b".repeat(5000));
    assert!(html.len() > 70_000);
    assert_eq!(extract_page_title(&html), Some("Early Title".to_string()));
}

#[test]
fn t_title_beyond_64k_window_does_not_panic() {
    // Title lives past the fast-scan window: must fall back to DOM parsing
    // without panicking on the straddling char.
    let mut html = "a".repeat(65535);
    html.push('\u{0375}');
    html.push_str(&"b".repeat(1000));
    html.push_str("<title>Late Title</title>");
    assert_eq!(extract_page_title(&html), Some("Late Title".to_string()));
}

#[test]
fn t_title_giant_ascii_input_without_tags_is_none() {
    assert_eq!(extract_page_title(&"a".repeat(70_000)), None);
}

// ---------------------------------------------------------------------------
// rank_passages in Sparse mode (min-max normalization + min_score floor)
// ---------------------------------------------------------------------------

fn passage(text: &str, url: &str, title: &str, idx: usize, score: f32) -> ScoredPassage {
    ScoredPassage {
        text: text.to_string(),
        source_url: url.to_string(),
        source_title: title.to_string(),
        passage_index: idx,
        score,
        sparse_score: None,
        dense_score: None,
    }
}

fn graded_passages_inverted_scores() -> Vec<ScoredPassage> {
    // NOTE: input scores are deliberately INVERTED vs relevance (low relevance
    // carries the highest RRF-ish score). Sparse mode must recompute via BM25,
    // so these inputs cannot survive — this proves recompute+normalize rather
    // than score preservation.
    vec![
        passage(
            "cooking pasta recipes italian cuisine simmer sauce",
            "https://ex.com/low",
            "Low",
            0,
            0.033,
        ),
        passage(
            "rust programming language systems",
            "https://ex.com/mid",
            "Mid",
            1,
            0.031,
        ),
        passage(
            "rust compiler borrow checker memory safety ownership rules rust compiler borrow checker",
            "https://ex.com/top",
            "Top",
            2,
            0.030,
        ),
    ]
}

const RANK_QUERY: &str = "rust compiler borrow checker memory safety";

#[tokio::test]
async fn t_sparse_normalizes_top_to_one_preserves_order() {
    let mut v = graded_passages_inverted_scores();
    rank_passages(
        RANK_QUERY,
        &mut v,
        RankingMode::Sparse,
        None,
        &RankingPolicy {
            min_score: 0.0,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(v.len(), 3, "min_score 0.0 keeps everything");
    let titles: Vec<&str> = v.iter().map(|p| p.source_title.as_str()).collect();
    assert_eq!(titles, vec!["Top", "Mid", "Low"]);
    assert_eq!(v[0].score, 1.0, "top must normalize to exactly 1.0");
    for p in &v {
        assert!(
            (0.0..=1.0).contains(&p.score),
            "scores confined to [0,1]; got {}",
            p.score
        );
    }
    assert!(v[0].score > v[1].score);
    assert!(v[2].score < 1e-6, "irrelevant passage near zero");
}

#[tokio::test]
async fn t_sparse_min_score_floor_keeps_only_top() {
    let policy = RankingPolicy {
        min_score: 0.9,
        ..Default::default()
    };
    let mut v = graded_passages_inverted_scores();
    rank_passages(RANK_QUERY, &mut v, RankingMode::Sparse, None, &policy, None)
        .await
        .unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].source_url, "https://ex.com/top");
    assert_eq!(v[0].score, 1.0);
}

#[tokio::test]
async fn t_sparse_floor_above_one_empties() {
    let policy = RankingPolicy {
        min_score: 1.5,
        ..Default::default()
    };
    let mut v = graded_passages_inverted_scores();
    rank_passages(RANK_QUERY, &mut v, RankingMode::Sparse, None, &policy, None)
        .await
        .unwrap();
    assert!(v.is_empty(), "no normalized score can reach 1.5");
}

#[tokio::test]
async fn t_sparse_uniform_scores_all_become_one() {
    let mut v = vec![
        passage(
            "rust compiler memory safety alpha",
            "https://a.ex/",
            "A",
            0,
            0.0,
        ),
        passage(
            "rust compiler memory safety beta",
            "https://b.ex/",
            "B",
            1,
            0.0,
        ),
        passage(
            "rust compiler memory safety gamma",
            "https://c.ex/",
            "C",
            2,
            0.0,
        ),
    ];
    rank_passages(
        RANK_QUERY,
        &mut v,
        RankingMode::Sparse,
        None,
        &RankingPolicy {
            min_score: 0.0,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(v.len(), 3);
    for p in &v {
        assert_eq!(p.score, 1.0, "uniform span collapses to 1.0");
    }
}

#[tokio::test]
async fn t_sparse_single_passage_becomes_one() {
    let mut v = vec![passage("rust compiler", "https://a.ex/", "A", 0, 0.5)];
    rank_passages(
        RANK_QUERY,
        &mut v,
        RankingMode::Sparse,
        None,
        &RankingPolicy::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].score, 1.0);
}

#[tokio::test]
async fn t_sparse_empty_input_ok() {
    let mut v: Vec<ScoredPassage> = vec![];
    rank_passages(
        RANK_QUERY,
        &mut v,
        RankingMode::Sparse,
        None,
        &RankingPolicy::default(),
        None,
    )
    .await
    .unwrap();
    assert!(v.is_empty());
}

#[test]
fn t_ranking_default_min_score_is_twelve_percent() {
    assert_eq!(RankingPolicy::default().min_score, 0.12);
}
