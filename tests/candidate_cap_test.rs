//! candidate_cap_test.rs — Hermetic tests for the `order_candidates`
//! per-apex cap contract (pass 3 uncapped fill removed).
//! Category     : Integration Test
//! Component    : nexus::extraction::quality::order_candidates
//! Prerequisites: None (hermetic — no network, no models)
//! Execution    : CARGO_BUILD_JOBS=1 cargo test --test candidate_cap_test

use nexus::extraction::quality::{extract_apex_domain, order_candidates};
use nexus::model::{Engine, EngineHit};

fn hit(url: &str) -> EngineHit {
    EngineHit::new("T", url, url, "S", Engine::Duckduckgo)
}

fn apex_counts(urls: &[String]) -> std::collections::HashMap<String, usize> {
    let mut m = std::collections::HashMap::new();
    for u in urls {
        *m.entry(extract_apex_domain(u)).or_insert(0usize) += 1;
    }
    m
}

#[test]
fn t_cap_satisfiable_diversity_fills_to_max_with_two_per_apex() {
    // 4 same-apex + 4 distinct-domain hits, max 6 → exactly 6, example.com 2×.
    // Empty query preserves input order (no scoring re-rank).
    let hits = vec![
        hit("https://example.com/a1"),
        hit("https://example.com/a2"),
        hit("https://example.com/a3"),
        hit("https://example.com/a4"),
        hit("https://alpha.org/p"),
        hit("https://beta.net/p"),
        hit("https://gamma.io/p"),
        hit("https://delta.dev/p"),
    ];
    let out = order_candidates("", hits, 6);
    assert_eq!(
        out.len(),
        6,
        "diverse pool must fill all 6 slots, got {out:?}"
    );
    let counts = apex_counts(&out);
    assert_eq!(
        counts.get("example.com").copied().unwrap_or(0),
        2,
        "example.com must appear exactly 2×, got {out:?}"
    );
    for (apex, n) in &counts {
        assert!(*n <= 2, "apex {apex:?} exceeds cap with {n}× in {out:?}");
    }
}

#[test]
fn t_cap_domain_poor_pool_returns_fewer_than_max() {
    // 8 same-apex + 4 distinct, max 8 → strict 2-per-apex cap wins over fill:
    // 2 (example.com) + 4 (distincts × 1) = 6, not 8.
    let mut hits = Vec::new();
    for i in 0..8 {
        hits.push(hit(&format!("https://example.com/p{i}")));
    }
    for d in ["alpha.org", "beta.net", "gamma.io", "delta.dev"] {
        hits.push(hit(&format!("https://{d}/p")));
    }
    let out = order_candidates("", hits, 8);
    assert_eq!(
        out.len(),
        6,
        "domain-poor pool must stop at 6 under the cap, got {out:?}"
    );
    let counts = apex_counts(&out);
    assert_eq!(
        counts.get("example.com").copied().unwrap_or(0),
        2,
        "example.com capped at 2×, got {out:?}"
    );
    for (apex, n) in &counts {
        assert!(*n <= 2, "apex {apex:?} exceeds cap with {n}× in {out:?}");
    }
}

#[test]
fn t_cap_tiny_pool_returns_all_in_order() {
    let h1 = hit("https://example.com/a");
    let h2 = hit("https://alpha.org/b");
    let h3 = hit("https://beta.net/c");
    let out = order_candidates("", vec![h1.clone(), h2.clone(), h3.clone()], 8);
    assert_eq!(out, vec![h1.url, h2.url, h3.url]);
}
