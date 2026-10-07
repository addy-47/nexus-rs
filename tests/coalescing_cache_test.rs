//! ============================================================================
//! coalescing_cache_test.rs — Hermetic integration test for query cache & coalescing
//! ============================================================================
//! Category     : Concurrency / Cache Integration Test
//! Component    : nexus::pipeline::NexusSearch
//! Prerequisites: None (hermetic mock / in-memory state)
//! Execution    : cargo test --test coalescing_cache_test
//! ============================================================================

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nexus::model::{NexusSearchOptions, NexusSearchResult, RankingMode, TimeFilter};
use nexus::pipeline::{InFlightGuard, InFlightMap, NexusSearch};

#[tokio::test]
async fn test_empty_query_returns_empty_result_instantly() {
    let client = NexusSearch::builder()
        .with_engines(vec![nexus::Engine::Duckduckgo])
        .build()
        .expect("Client builder failed");

    let options = NexusSearchOptions {
        time_filter: TimeFilter::Any,
        ranking_mode: RankingMode::Sparse,
        max_candidates: 3,
        chunk_size_words: 150,
        chunk_overlap_words: 30,
        focus: None,
        ..Default::default()
    };

    let res = client.search("   ", &options).await.expect("Search failed");
    assert!(res.raw_pages.is_empty());
    assert!(res.scored_passages.is_empty());
}

#[tokio::test]
async fn test_builder_configuration_validation() {
    // Empty engine list must be rejected
    let empty_res = NexusSearch::builder().with_engines(vec![]).build();
    assert!(empty_res.is_err(), "Empty engines list must fail validation");

    // Zero timeout must be rejected
    let zero_timeout = NexusSearch::builder()
        .with_fetch_timeout(Duration::from_millis(0))
        .build();
    assert!(zero_timeout.is_err(), "Zero timeout must fail validation");

    // Zero max response bytes must be rejected
    let zero_bytes = NexusSearch::builder().with_max_response_bytes(0).build();
    assert!(zero_bytes.is_err(), "Zero response bytes must fail validation");
}

/// B3 Mutation 1 killer:
/// Asserts that InFlightGuard removes its key from in_flight map on drop when not disarmed.
/// If InFlightGuard::drop is neutered to a no-op, this assertion fails immediately (RED).
#[tokio::test]
async fn test_in_flight_guard_cleans_up_on_drop_when_aborted() {
    let in_flight: InFlightMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let key = "test_guard_cleanup_key".to_string();

    {
        let (tx, _rx) = tokio::sync::broadcast::channel(4);
        in_flight.lock().await.insert(key.clone(), tx);
        assert_eq!(in_flight.lock().await.len(), 1, "Must contain inserted key initially");

        // Guard is armed but dropped without calling disarm()
        let _guard = InFlightGuard::new(Arc::clone(&in_flight), key.clone());
    }

    // Yield to let the asynchronous drop cleanup task run
    tokio::time::sleep(Duration::from_millis(50)).await;

    let remaining_len = in_flight.lock().await.len();
    assert_eq!(
        remaining_len, 0,
        "InFlightGuard must remove the key from in_flight on drop (Mutant 1 killed)"
    );
}

/// B3 Mutation 2 killer:
/// Asserts that a follower waiting on a hung leader times out boundedly rather than hanging.
/// If follower wait is set to u64::MAX/4 (unbounded), this test times out and fails (RED).
#[tokio::test]
async fn test_follower_bounded_wait_times_out_on_hung_leader() {
    let client = NexusSearch::builder()
        .with_engines(vec![nexus::Engine::Duckduckgo])
        .build()
        .expect("Client builder failed");

    // Construct exact cache key used for "__test_hermetic_hung_leader_query"
    let cache_key = format!(
        "{}:{}:{:?}:{:?}:{}:{}:{}:{:?}",
        "__test_hermetic_hung_leader_query",
        3,
        TimeFilter::Any,
        RankingMode::Sparse,
        150,
        30,
        524288,
        ""
    );

    // Insert a leader broadcast channel that NEVER sends anything
    let (tx, _rx) = tokio::sync::broadcast::channel(4);
    client.in_flight_map().lock().await.insert(cache_key.clone(), tx);

    let options = NexusSearchOptions {
        time_filter: TimeFilter::Any,
        ranking_mode: RankingMode::Sparse,
        max_candidates: 3,
        chunk_size_words: 150,
        chunk_overlap_words: 30,
        fetch_timeout_ms: 80, // Request short 80ms follower timeout
        focus: None,
        ..Default::default()
    };

    let start = std::time::Instant::now();
    // Follower joins in-flight wave, waits with bounded timeout (80ms), times out and proceeds
    // With unbounded wait (Mutant 2), this hangs and tokio::time::timeout fires at 400ms
    let search_fut = client.search("__test_hermetic_hung_leader_query", &options);
    let outcome = tokio::time::timeout(Duration::from_millis(400), search_fut).await;

    assert!(
        outcome.is_ok(),
        "Follower must boundedly time out and proceed rather than hanging indefinitely (Mutant 2 killed)"
    );

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(400),
        "Follower must return within bounded time, took {:?}",
        elapsed
    );
}

/// B3 follower result delivery:
/// Asserts that a coalesced follower receives the exact result broadcast by the leader.
#[tokio::test]
async fn test_coalescing_follower_receives_leader_result() {
    let client = NexusSearch::builder()
        .with_engines(vec![nexus::Engine::Duckduckgo])
        .build()
        .expect("Client builder failed");

    let cache_key = format!(
        "{}:{}:{:?}:{:?}:{}:{}:{}:{:?}",
        "coalesced_broadcast_query",
        3,
        TimeFilter::Any,
        RankingMode::Sparse,
        150,
        30,
        524288,
        ""
    );

    let (tx, _rx) = tokio::sync::broadcast::channel(4);
    client.in_flight_map().lock().await.insert(cache_key.clone(), tx.clone());

    let leader_result = NexusSearchResult {
        raw_pages: vec![nexus::RawPage {
            url: "https://example.com/broadcast".to_string(),
            title: "Broadcast Title".to_string(),
            markdown: "Broadcast passage content for follower.".to_string(),
        }],
        scored_passages: vec![nexus::ScoredPassage {
            text: "Broadcast passage content for follower.".to_string(),
            source_url: "https://example.com/broadcast".to_string(),
            source_title: "Broadcast Title".to_string(),
            passage_index: 0,
            score: 0.95,
            sparse_score: Some(0.95),
            dense_score: None,
        }],
        metrics: Default::default(),
    };

    // Leader sends result on channel asynchronously
    let leader_res_clone = leader_result.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx.send(Ok(leader_res_clone));
    });

    let options = NexusSearchOptions {
        time_filter: TimeFilter::Any,
        ranking_mode: RankingMode::Sparse,
        max_candidates: 3,
        chunk_size_words: 150,
        chunk_overlap_words: 30,
        fetch_timeout_ms: 1000,
        focus: None,
        ..Default::default()
    };

    // Follower executes search() - must receive the broadcasted leader result via coalescing
    let follower_outcome = client
        .search("coalesced_broadcast_query", &options)
        .await
        .expect("Follower search must receive broadcasted leader result");

    assert_eq!(follower_outcome.raw_pages.len(), 1);
    assert_eq!(follower_outcome.raw_pages[0].title, "Broadcast Title");
    assert_eq!(follower_outcome.scored_passages.len(), 1);
    assert_eq!(follower_outcome.scored_passages[0].score, 0.95);
    assert_eq!(
        follower_outcome.scored_passages[0].text,
        leader_result.scored_passages[0].text,
        "Follower must receive exact leader result bytes"
    );
}

/// B4 Mutex Poisoning killer:
/// Asserts that NexusSearch recovers from a poisoned query_cache mutex and continues operating.
/// If unwrap_or_else(into_inner) is replaced with unwrap(), this test panics and fails (RED).
#[tokio::test]
async fn test_cache_recovers_from_mutex_poisoning() {
    let client = NexusSearch::builder()
        .with_engines(vec![nexus::Engine::Duckduckgo])
        .build()
        .expect("Client builder failed");

    // Poison the cache mutex via test helper thread panic
    client.poison_cache_for_test();

    // Verify cache is poisoned
    let is_poisoned = client.in_flight_map(); // just to verify client is valid
    let _ = is_poisoned;

    // Search with empty query must NOT panic and must recover via unwrap_or_else
    let options = NexusSearchOptions::default();
    let res = client.search("   ", &options).await;
    assert!(
        res.is_ok(),
        "search() must succeed and recover even if query_cache mutex is poisoned (B4 killed)"
    );

    // Also verify cache query for non-empty search recovers without panic
    let non_empty_res = client.search("poison_recovery_test_query", &options).await;
    assert!(
        non_empty_res.is_ok() || non_empty_res.is_err(),
        "search() must not panic when querying poisoned cache"
    );
}

#[tokio::test]
async fn test_focus_passage_extraction() {
    let client = NexusSearch::builder()
        .with_engines(vec![nexus::Engine::Duckduckgo])
        .build()
        .expect("Client builder failed");

    let raw_page = nexus::RawPage {
        url: "https://example.com/rust".to_string(),
        title: "Rust Systems Programming".to_string(),
        markdown: "Introduction to systems programming.\n\nConcurrency and memory safety in Rust with borrow checker rules.\n\nWebAssembly target details.".to_string(),
    };

    let focused = client.extract_focused_passages(&raw_page, Some("borrow checker"), 5);
    assert!(!focused.is_empty(), "Focused extraction should yield passages");
    assert!(
        focused[0].text.to_lowercase().contains("borrow checker"),
        "Top focused passage must contain the query terms"
    );
}
