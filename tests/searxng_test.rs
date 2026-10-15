//! ============================================================================
//! searxng_test.rs — Hermetic test for SearXNG fanout skip behavior
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::engines::searxng fanout skip
//! Prerequisites: None (hermetic test-hook query, no network)
//! Execution    : cargo test --test searxng_test
//! ============================================================================

use nexus::Engine;
use nexus::engines::EngineFanout;
use nexus::model::{FanoutPolicy, TimeFilter};

/// SearXNG without a configured instance URL must be skipped at fanout time:
/// zero hits, and the SearXNG metric records `success=false` with error
/// `"unconfigured"`. The `__test_hermetic_` prefix short-circuits dispatch to
/// empty (no network) via the `NEXUS_TEST_HOOKS` branch in
/// `dispatch_engine_query`; the assertion targets the metrics entry since
/// hermetic queries return empty for ALL engines.
#[tokio::test]
async fn test_searxng_fanout_skip_without_instance_url_reports_unconfigured() {
    // Hermetic hook is env-gated in production dispatch; enable so no real
    // network is attempted. Serial execution only.
    // SAFETY: serial test execution; no other thread observes env here.
    unsafe { std::env::set_var("NEXUS_TEST_HOOKS", "1") };

    let fanout = EngineFanout::new(vec![Engine::Searxng], FanoutPolicy::default());

    let outcome = fanout
        .query_all("__test_hermetic_searxng_unconfigured__", TimeFilter::Any)
        .await
        .expect("query_all must return an outcome carrying the unconfigured SearXNG metric");

    assert!(
        outcome.hits.is_empty(),
        "Unconfigured SearXNG must yield zero hits, got {}",
        outcome.hits.len()
    );

    let metric = outcome
        .metrics
        .iter()
        .find(|m| m.engine == Engine::Searxng)
        .expect("SearXNG metric must be present");
    assert!(
        !metric.success,
        "Unconfigured SearXNG must be marked unsuccessful in metrics"
    );
    assert_eq!(
        metric.error.as_deref(),
        Some("unconfigured"),
        "SearXNG metric must report the unconfigured skip"
    );
}
