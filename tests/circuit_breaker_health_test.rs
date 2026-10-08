//! ============================================================================
//! circuit_breaker_health_test.rs — Hermetic unit & state test for EngineHealthTracker
//! ============================================================================
//! Category     : Unit / State Machine Test
//! Component    : nexus::engines::health
//! Prerequisites: None (hermetic in-memory state)
//! Execution    : cargo test --test circuit_breaker_health_test
//! ============================================================================

use std::time::Duration;

use nexus::Engine;
use nexus::engines::health::{EngineHealthTracker, QUARANTINE_DURATION};

#[test]
fn test_initial_engine_state_is_healthy() {
    let tracker = EngineHealthTracker::new();
    assert!(!tracker.is_quarantined(Engine::Duckduckgo));
    assert!(!tracker.is_quarantined(Engine::Brave));
    assert!(!tracker.is_quarantined(Engine::Wikipedia));
}

#[test]
fn test_consecutive_failures_trigger_quarantine() {
    let mut tracker = EngineHealthTracker::new();
    let engine = Engine::Duckduckgo;

    // First failure (soft): not quarantined
    tracker.record_failure(engine, false);
    assert!(!tracker.is_quarantined(engine));

    // Second failure (soft): not quarantined
    tracker.record_failure(engine, false);
    assert!(!tracker.is_quarantined(engine));

    // Third failure (soft): triggers quarantine threshold (3)
    tracker.record_failure(engine, false);
    assert!(
        tracker.is_quarantined(engine),
        "Must be quarantined after 3 soft failures"
    );
}

#[test]
fn test_hard_block_immediately_quarantines() {
    let mut tracker = EngineHealthTracker::new();
    let engine = Engine::Mojeek;

    // Hard block (e.g. CAPTCHA, 403 Forbidden, bot challenge)
    tracker.record_failure(engine, true);
    assert!(
        tracker.is_quarantined(engine),
        "Hard block must trigger quarantine immediately"
    );
}

#[test]
fn test_success_resets_consecutive_failures_and_updates_ewma() {
    let mut tracker = EngineHealthTracker::new();
    let engine = Engine::Bing;

    // Initial default latency is 500.0ms
    assert_eq!(tracker.get_ewma_latency(engine), 500.0);

    // Two soft failures
    tracker.record_failure(engine, false);
    tracker.record_failure(engine, false);
    assert!(!tracker.is_quarantined(engine));

    // Success with 200ms latency clears consecutive failure count
    tracker.record_success(engine, 200);
    assert!(!tracker.is_quarantined(engine));

    // EWMA update: 0.8 * 500.0 + 0.2 * 200.0 = 400.0 + 40.0 = 440.0
    let ewma = tracker.get_ewma_latency(engine);
    assert!(
        (ewma - 440.0).abs() < 1e-4,
        "EWMA latency must blend smoothly, got {ewma}"
    );

    // Two more failures should NOT quarantine because counter was reset
    tracker.record_failure(engine, false);
    tracker.record_failure(engine, false);
    assert!(
        !tracker.is_quarantined(engine),
        "Failure count must have been reset by earlier success"
    );
}

#[test]
fn test_quarantine_duration_constant() {
    assert_eq!(
        QUARANTINE_DURATION,
        Duration::from_secs(600),
        "Quarantine TTL must be 10 minutes"
    );
}

/// B5 Panic Isolation killer:
/// Asserts that a panicking engine does NOT detonate the entire fanout wave.
/// The healthy engine's hits must survive, and the panicking engine must be recorded
/// as unsuccessful with panic details in metrics, incrementing its failure count in health.
/// If AssertUnwindSafe(...).catch_unwind() is removed from engines/mod.rs, this test PANICS and fails (RED).
#[tokio::test]
async fn test_fanout_engine_panic_isolation_and_metric_reporting() {
    use nexus::engines::EngineFanout;
    use nexus::model::{FanoutPolicy, TimeFilter};
    use std::sync::{Arc, RwLock};

    // Panic hook is env-gated in production dispatch (S7); enable so Mojeek
    // actually panics instead of hitting the real network. Serial execution only.
    // SAFETY: serial test execution; no other thread observes env here.
    unsafe { std::env::set_var("NEXUS_TEST_HOOKS", "1") };

    let health = Arc::new(RwLock::new(EngineHealthTracker::new()));
    let fanout = EngineFanout::with_health(
        vec![Engine::Duckduckgo, Engine::Mojeek],
        FanoutPolicy {
            min_reporting_engines: 2,
            min_distinct_domains: 2,
            min_candidate_hits: 10,
            max_fanout_deadline_ms: 1000,
        },
        Arc::clone(&health),
    );

    // Query triggers simulated panic in Mojeek while Duckduckgo succeeds with a hit
    let outcome = fanout
        .query_all("__test_panic_engine__", TimeFilter::Any)
        .await
        .expect("query_all must NOT unwind when an engine panics (B5 catch_unwind isolation)");

    // Healthy engine hits must survive
    assert!(
        !outcome.hits.is_empty(),
        "Healthy engine hits must survive another engine's panic"
    );
    assert_eq!(outcome.hits[0].engine, Engine::Duckduckgo);

    // Panicking engine must be recorded in metrics as unsuccessful with panic message
    let mojeek_metric = outcome
        .metrics
        .iter()
        .find(|m| m.engine == Engine::Mojeek)
        .expect("Mojeek metric must be present");
    assert!(
        !mojeek_metric.success,
        "Panicking engine must be marked unsuccessful in metrics"
    );
    let err_msg = mojeek_metric.error.as_deref().unwrap_or("");
    assert!(
        err_msg.contains("panicked"),
        "Metric must report panic failure, got: {err_msg}"
    );

    // Health tracker must have recorded failure for the panicking engine
    let h = health.read().unwrap();
    assert_eq!(
        h.consecutive_failures(Engine::Mojeek),
        1,
        "Failure must be recorded in health tracker"
    );
}
