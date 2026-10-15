//! failure_class_test.rs — FailureClass taxonomy wire + hermetic NoHits path.
//! Category     : Telemetry contract / hermetic integration
//! Execution    : CARGO_BUILD_JOBS=1 cargo test --test failure_class_test

use nexus::pipeline::{is_thin_evidence, NexusSearch};
use nexus::{Engine, FailureClass, NexusSearchMetrics, NexusSearchOptions};

#[test]
fn test_failure_class_serde_wire_values() {
    let cases = [
        (FailureClass::NoHits, "\"no_hits\""),
        (FailureClass::FetchStarved, "\"fetch_starved\""),
        (FailureClass::AnswerAbsent, "\"answer_absent\""),
        (FailureClass::LowScoreFloor, "\"low_score_floor\""),
        (FailureClass::BudgetExhausted, "\"budget_exhausted\""),
    ];
    for (variant, wire) in cases {
        let ser = serde_json::to_string(&variant).expect("serialize FailureClass");
        assert_eq!(ser, wire, "wire value for {variant:?}");
        let back: FailureClass = serde_json::from_str(wire).expect("deserialize FailureClass");
        assert_eq!(back, variant, "round-trip for {wire}");
    }
}

#[test]
fn test_metrics_default_failure_class_is_none() {
    let m = NexusSearchMetrics::default();
    assert_eq!(m.failure_class, None);
}

#[test]
fn test_is_thin_evidence_truth_table() {
    let cases = [
        ((1, 1), true),
        ((2, 1), false),
        ((1, 2), false),
        ((0, 0), false),
        ((5, 3), false),
    ];
    for ((passages, pages), expected) in cases {
        assert_eq!(
            is_thin_evidence(passages, pages),
            expected,
            "is_thin_evidence({passages}, {pages})"
        );
    }
}

#[test]
fn test_thin_evidence_serde_wire_value() {
    let ser =
        serde_json::to_string(&FailureClass::ThinEvidence).expect("serialize ThinEvidence");
    assert_eq!(ser, "\"thin_evidence\"");
    let back: FailureClass =
        serde_json::from_str("\"thin_evidence\"").expect("deserialize ThinEvidence");
    assert_eq!(back, FailureClass::ThinEvidence);
}

#[tokio::test]
async fn test_hermetic_nohits_probe_yields_no_hits_class() {
    // SAFETY: serial test execution (--test-threads=1); no other thread observes env here.
    unsafe { std::env::set_var("NEXUS_TEST_HOOKS", "1") };
    let client = NexusSearch::builder()
        .with_engines(vec![Engine::Duckduckgo])
        .build()
        .expect("builder failed");
    let res = client
        .search(
            "__test_hermetic_nohits_probe",
            &NexusSearchOptions::default(),
        )
        .await
        .expect("search failed");
    assert!(
        res.scored_passages.is_empty(),
        "hermetic hook must yield no passages"
    );
    assert_eq!(res.metrics.failure_class, Some(FailureClass::NoHits));
}
