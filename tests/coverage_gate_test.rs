//! ============================================================================
//! coverage_gate_test.rs — Integration tests for the query-term coverage gate
//! (`passage_set_coverage` + `uncovered_query_tokens` + drain predicate in
//! `pipeline.rs`).
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::extraction::quality::{passage_set_coverage,
//!                uncovered_query_tokens},
//!                nexus::model::RankingPolicy::coverage_floor
//! Prerequisites: None (hermetic — no network, no models)
//! Execution    : cargo test --release --test coverage_gate_test
//! ============================================================================
//!
//! Context: G3 `long_02` delivered a 0.618 fused score on Rust-language
//! content for an "HTML-to-markdown libraries" query — embedding drift to the
//! head term ("rust") that the score floor could not see. The coverage gate
//! drains the delivered set when `uncovered_query_tokens(query, passages)`
//! names 2+ distinctive tokens (and `coverage_floor > 0.0`). A single miss is
//! tolerated as synonym variance. `passage_set_coverage` remains the reported
//! fraction value; the drain decision rests on the uncovered list.

use nexus::extraction::quality::{passage_set_coverage, uncovered_query_tokens};
use nexus::model::{RankingPolicy, ScoredPassage};

fn passage(title: &str, text: &str) -> ScoredPassage {
    ScoredPassage {
        text: text.to_string(),
        source_url: "https://example.com/rust-lang-overview".to_string(),
        source_title: title.to_string(),
        passage_index: 0,
        score: 0.618,
        sparse_score: None,
        dense_score: None,
    }
}

/// Mirrors the drain predicate in `pipeline.rs::search_uncached` so the gate
/// decision itself is pinned without needing network access:
/// `uncovered.len() >= 2 && floor > 0.0` (empty sets never drain because
/// `uncovered_query_tokens` is empty for empty passage lists).
fn gate_trips(query: &str, passages: &[ScoredPassage], floor: f32) -> bool {
    !passages.is_empty()
        && uncovered_query_tokens(query, passages).len() >= 2
        && floor > 0.0
}

// ---------------------------------------------------------------------------
// 1. long_02-shaped case: drift to the head term must trip the gate via the
//    uncovered list (3 missing distinctive tokens).
// ---------------------------------------------------------------------------

#[test]
fn long02_drift_coverage_below_floor() {
    let query = "Rust HTML to markdown libraries";
    // Substantive tokens per `substantive_query_tokens`: rust, html,
    // markdown, libraries ("to" is too short; nothing else is a stopword).
    let passages = vec![
        passage(
            "The Rust Programming Language",
            "Rust is a systems programming language focused on safety and speed. \
             Ownership and borrowing let developers write fearless concurrent code. \
             The compiler enforces memory safety without a garbage collector.",
        ),
        passage(
            "Rust Language Overview",
            "The Rust community values performance and reliability. Cargo builds \
             projects while rustc compiles crates with strong static guarantees.",
        ),
    ];
    let coverage = passage_set_coverage(query, &passages);
    // Only "rust" of the 4 substantive tokens appears: 1/4 = 0.25.
    assert!(
        (coverage - 0.25).abs() < f32::EPSILON,
        "expected 0.25, got {coverage}"
    );
    assert!(
        coverage < 0.5,
        "drift content must stay below the 0.5 floor, got {coverage}"
    );
    let uncovered = uncovered_query_tokens(query, &passages);
    assert_eq!(
        uncovered,
        vec![
            "html".to_string(),
            "markdown".to_string(),
            "libraries".to_string()
        ],
        "long_02 drift must leave 3 distinctive tokens uncovered"
    );
    assert!(
        gate_trips(query, &passages, RankingPolicy::default().coverage_floor),
        "gate must trip on long_02-shaped drift (2+ uncovered tokens)"
    );
}

// ---------------------------------------------------------------------------
// 1b. Multi-token miss boundary: exactly 2 uncovered tokens still trips.
// ---------------------------------------------------------------------------

#[test]
fn two_uncovered_tokens_still_trips() {
    let query = "Rust HTML to markdown libraries";
    let passages = vec![passage(
        "Rust HTML guide",
        "Rust crates render HTML pages with speed and safety. \
         The HTML parser handles tags while Rust ownership keeps memory safe.",
    )];
    let uncovered = uncovered_query_tokens(query, &passages);
    assert_eq!(uncovered.len(), 2, "expected exactly 2 uncovered, got {uncovered:?}");
    assert_eq!(
        uncovered,
        vec!["markdown".to_string(), "libraries".to_string()],
        "the two misses must be markdown + libraries"
    );
    // Reported fraction is 2/4 = 0.5 — the drain decision comes from the
    // uncovered count, not the fraction.
    let coverage = passage_set_coverage(query, &passages);
    assert!(
        (coverage - 0.5).abs() < f32::EPSILON,
        "expected 0.5, got {coverage}"
    );
    assert!(
        gate_trips(query, &passages, RankingPolicy::default().coverage_floor),
        "gate must trip on exactly 2 uncovered tokens"
    );
}

// ---------------------------------------------------------------------------
// 2. Single-synonym-miss: all-but-one distinctive tokens present → gate does
//    NOT trip (tolerated as synonym variance).
// ---------------------------------------------------------------------------

#[test]
fn single_synonym_miss_does_not_trip() {
    let query = "tokio async runtime";
    // Substantive tokens: tokio, async, runtime. Passages carry tokio +
    // async but never the word "runtime" (uses "scheduler" instead).
    let passages = vec![ScoredPassage {
        text: "Tokio drives async tasks with a work-stealing scheduler for \
               concurrent programs. The tokio scheduler parks idle workers."
            .to_string(),
        source_url: "https://example.com/tokio-guide".to_string(),
        source_title: "Tokio async scheduler guide".to_string(),
        passage_index: 0,
        score: 0.9,
        sparse_score: Some(1.2),
        dense_score: None,
    }];
    let uncovered = uncovered_query_tokens(query, &passages);
    assert_eq!(
        uncovered,
        vec!["runtime".to_string()],
        "single synonym miss must leave exactly [runtime] uncovered"
    );
    let coverage = passage_set_coverage(query, &passages);
    assert!(
        (coverage - 2.0 / 3.0).abs() < 1e-6,
        "expected 2/3, got {coverage}"
    );
    assert!(
        !gate_trips(query, &passages, RankingPolicy::default().coverage_floor),
        "gate must NOT trip on a single uncovered token (synonym variance)"
    );
}

// ---------------------------------------------------------------------------
// 2b. Disabled floor (0.0) never drains, even with a multi-token miss.
// ---------------------------------------------------------------------------

#[test]
fn disabled_floor_never_trips() {
    let query = "Rust HTML to markdown libraries";
    let passages = vec![passage(
        "The Rust Programming Language",
        "Rust is a systems programming language focused on safety and speed.",
    )];
    let uncovered = uncovered_query_tokens(query, &passages);
    assert!(
        uncovered.len() >= 2,
        "fixture must have 2+ uncovered, got {uncovered:?}"
    );
    assert!(
        !gate_trips(query, &passages, 0.0),
        "gate must not trip when coverage_floor is 0.0 (disabled)"
    );
}

// ---------------------------------------------------------------------------
// 3. Genuine case: every substantive term present → coverage == 1.0.
// ---------------------------------------------------------------------------

#[test]
fn genuine_full_match_coverage_is_one() {
    let query = "tokio async runtime";
    let passages = vec![ScoredPassage {
        text: "Tokio is an async runtime for Rust. The tokio scheduler drives \
               async tasks to completion with work-stealing runtime workers."
            .to_string(),
        source_url: "https://example.com/tokio-guide".to_string(),
        source_title: "Tokio async runtime guide".to_string(),
        passage_index: 0,
        score: 0.9,
        sparse_score: Some(1.2),
        dense_score: None,
    }];
    let coverage = passage_set_coverage(query, &passages);
    assert!(
        (coverage - 1.0).abs() < f32::EPSILON,
        "expected 1.0, got {coverage}"
    );
    assert!(
        uncovered_query_tokens(query, &passages).is_empty(),
        "genuine match must leave no uncovered tokens"
    );
    assert!(
        !gate_trips(query, &passages, RankingPolicy::default().coverage_floor),
        "gate must not trip on a genuine full match"
    );
}

// ---------------------------------------------------------------------------
// 4. Abstract query → coverage == 1.0 (gate never trips).
// ---------------------------------------------------------------------------

#[test]
fn abstract_query_coverage_is_one() {
    // "what"/"is"/"about" are stopwords; the passage carries "this" so the
    // value is 1.0 whether or not "this" is treated as substantive.
    let query = "what is this about";
    let passages = vec![passage(
        "Untitled notes",
        "This page collects assorted thoughts on many things.",
    )];
    let coverage = passage_set_coverage(query, &passages);
    assert!(
        (coverage - 1.0).abs() < f32::EPSILON,
        "expected 1.0, got {coverage}"
    );
    assert!(
        uncovered_query_tokens(query, &passages).is_empty(),
        "abstract query must leave no uncovered tokens"
    );
    assert!(
        !gate_trips(query, &passages, RankingPolicy::default().coverage_floor),
        "gate must not trip on an abstract query"
    );
}

// ---------------------------------------------------------------------------
// 5. Empty passage list → 1.0 (vacuous; nothing to drain).
// ---------------------------------------------------------------------------

#[test]
fn empty_passages_coverage_is_one() {
    let query = "Rust HTML to markdown libraries";
    let coverage = passage_set_coverage(query, &[]);
    assert!(
        (coverage - 1.0).abs() < f32::EPSILON,
        "expected 1.0, got {coverage}"
    );
    assert!(
        uncovered_query_tokens(query, &[]).is_empty(),
        "empty passage list must yield no uncovered tokens"
    );
    assert!(
        !gate_trips(query, &[], RankingPolicy::default().coverage_floor),
        "gate must not trip on an empty passage list"
    );
}

// ---------------------------------------------------------------------------
// 6. Policy default: coverage_floor == 0.5.
// ---------------------------------------------------------------------------

#[test]
fn ranking_policy_default_coverage_floor() {
    assert!(
        (RankingPolicy::default().coverage_floor - 0.5).abs() < f32::EPSILON,
        "expected default coverage_floor 0.5, got {}",
        RankingPolicy::default().coverage_floor
    );
}
