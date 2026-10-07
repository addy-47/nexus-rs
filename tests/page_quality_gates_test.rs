//! ============================================================================
//! page_quality_gates_test.rs — Integration tests for page-quality gates &
//! candidate ordering (G3-baseline hardening: P0-3/P0-4, wall_02, ent_01/03,
//! cmp_03, fanout D7).
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::extraction::quality, nexus::engines::EngineFanout
//! Prerequisites: None (hermetic — no network, no models)
//! Execution    : cargo nextest run --release --test-threads=1 --no-fail-fast
//! ============================================================================
//!
//! Authored by the testing subagent per plan §11.4 (D8). Private helpers
//! (`entity_anchors`, `domain_intent_domains`) are exercised exclusively
//! through the public `order_candidates`.

use nexus::engines::EngineFanout;
use nexus::extraction::quality::{
    is_challenge_or_error_page, is_code_like, is_language_mismatched, order_candidates,
};
use nexus::model::{Engine, EngineHit};

// ---------------------------------------------------------------------------
// Fixture builders (live G3 shapes)
// ---------------------------------------------------------------------------

/// `cmp_03` shape: minified Google Closure-style JS with ~5000-char lines.
fn cmp03_payload() -> String {
    let unit = "Wb(){return Rb()?Ib.platform===\"Windows\":Kb(\"Windows\")};";
    // 55 chars * 91 = 5005 chars per line; 3 lines ≈ 15KB.
    let line = unit.repeat(91);
    format!("{line}\n{line}\n{line}")
}

/// `ent_01` shape: Adobe Target / ContextHub personalization JS with
/// `campaignPath`, `ContextHub.SegmentEngine`, and `\u002D` escapes (~30KB).
fn ent01_payload() -> String {
    let unit = "if(a==b){campaign_path=ContextHub.SegmentEngine.get($id);opts={\"campaignPath\":\"x\\u002Dtest\"};}";
    let line = unit.repeat(100);
    format!("{line}\n{line}\n{line}")
}

/// Prose control: 5-sentence Federal Reserve press-release paragraph.
/// Must be negative for BOTH quality gates.
fn fed_prose() -> String {
    [
        "The Federal Reserve today announced its latest decision on interest rates after a two day meeting in Washington.",
        "Officials said inflation has eased over the past year while job growth remains steady across most regions.",
        "The committee voted to leave the target range unchanged and noted that future moves will depend on incoming data.",
        "Markets reacted calmly as investors had widely expected the outcome before the announcement.",
        "A press conference is scheduled for tomorrow morning to discuss the outlook in more detail.",
    ]
    .join("\n")
}

fn hit(url: &str, title: &str, snippet: &str) -> EngineHit {
    EngineHit::new(title, url, url, snippet, Engine::Duckduckgo)
}

// ---------------------------------------------------------------------------
// is_challenge_or_error_page
// ---------------------------------------------------------------------------

#[test]
fn t_challenge_ent03_block_page_detected() {
    // Exact live shape: block title + automated-process body.
    let title = "Your request has been blocked. This could be";
    let body = "Your current User-Agent string appears to be from an automated process. \
        If this is incorrect, please click this link to try again. \
        Turn on cookies in your browser.";
    assert!(is_challenge_or_error_page(title, body));
}

#[test]
fn t_challenge_title_only_detected() {
    assert!(is_challenge_or_error_page(
        "404 Not Found",
        "A helpful guide to neighborhood parks and weekend trails."
    ));
    assert!(is_challenge_or_error_page(
        "Attention Required! | Cloudflare",
        "Market data and quarterly earnings summary."
    ));
}

#[test]
fn t_challenge_body_only_detected() {
    assert!(is_challenge_or_error_page(
        "Quarterly Report",
        "Please verify you are a human to continue reading."
    ));
    assert!(is_challenge_or_error_page(
        "Search results",
        "Access Denied: you do not have permission to view this page."
    ));
}

#[test]
fn t_challenge_matching_is_case_insensitive() {
    for marker in ["captcha", "CLOUDFLARE", "ReCaPtChA", "ACCESS DENIED"] {
        let body = format!("Something something {marker} something");
        assert!(
            is_challenge_or_error_page("Normal title", &body),
            "marker {marker:?} must match case-insensitively"
        );
    }
}

#[test]
fn t_challenge_prose_control_negative() {
    assert!(!is_challenge_or_error_page(
        "Federal Reserve press release",
        &fed_prose()
    ));
}

#[test]
fn t_challenge_empty_inputs_negative() {
    assert!(!is_challenge_or_error_page("", ""));
    assert!(!is_challenge_or_error_page("   ", "   "));
}

#[test]
fn t_challenge_marker_beyond_body_window_ignored() {
    // Body is inspected only in the first 2000 chars; a marker at ~2500
    // must NOT fire (locks the bounded-window behavior).
    let body = format!("{}captcha at the very end", "a".repeat(2500));
    assert!(!is_challenge_or_error_page("Clean title", &body));
}

#[test]
fn t_challenge_marker_within_body_window_detected() {
    let body = format!("{} captcha failure", "a".repeat(1900));
    assert!(is_challenge_or_error_page("Clean title", &body));
}

// ---------------------------------------------------------------------------
// is_code_like
// ---------------------------------------------------------------------------

#[test]
fn t_codelike_cmp03_minified_closure_is_code() {
    assert!(is_code_like(&cmp03_payload()));
}

#[test]
fn t_codelike_ent01_adobe_target_is_code() {
    let payload = ent01_payload();
    assert!(
        payload.contains("campaignPath")
            && payload.contains("ContextHub.SegmentEngine")
            && payload.contains("\\u002D"),
        "fixture must preserve the live ent_01 shape markers"
    );
    assert!(is_code_like(&payload));
}

#[test]
fn t_codelike_prose_control_negative() {
    assert!(!is_code_like(&fed_prose()));
}

#[test]
fn t_codelike_empty_and_whitespace_negative() {
    assert!(!is_code_like(""));
    assert!(!is_code_like("   \n\t  "));
}

#[test]
fn t_codelike_short_snippet_not_flagged() {
    // 2-of-3-signals rule: a lone snippet may trip punctuation density but
    // must not nuke a docs page.
    assert!(!is_code_like("let x = 5;"));
    assert!(!is_code_like("fn main() { }"));
}

#[test]
fn t_codelike_unicode_prose_negative() {
    let ru = "Центральный банк сегодня объявил о своем решении по процентной ставке.\n\
        Чиновники отметили снижение инфляции за последний год.\n\
        Рынки отреагировали спокойно на это заявление.";
    assert!(!is_code_like(ru));
}

// ---------------------------------------------------------------------------
// order_candidates (also covers private entity_anchors / domain_intent_domains)
// ---------------------------------------------------------------------------

#[test]
fn t_order_wall02_reddit_intent_beats_redditinc() {
    // Query says "reddit discussion"; redditinc.com must NOT satisfy it.
    let q = "reddit discussion best local speech to speech models";
    let corp = hit(
        "https://redditinc.com/blog/company-news",
        "Reddit Inc company blog",
        "corporate newsroom and investor updates",
    );
    let comm = hit(
        "https://reddit.com/r/LocalLLaMA/comments/xyz",
        "r/LocalLLaMA thread",
        "community discussion of local speech models",
    );
    // Reversed input order: community hit must still come first.
    let out = order_candidates(q, vec![corp.clone(), comm.clone()], 5);
    assert_eq!(out, vec![comm.url, corp.url]);
}

#[test]
fn t_order_entity_anchor_outranks_hotel_homepage() {
    // G3 ent_01: token `all` matched a hotel brand because nothing required `MiniLM`.
    let q = "all-MiniLM-L6-v2 embedding dimensions and max sequence length";
    let hotel = hit(
        "https://example-hotels.com/",
        "Grand Harbor Hotel",
        "book rooms and suites near the harbor",
    );
    let model = hit(
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2",
        "all-MiniLM-L6-v2",
        "embedding dimensions 384 and max sequence length 256",
    );
    let out = order_candidates(q, vec![hotel, model.clone()], 5);
    assert_eq!(out[0], model.url);
}

#[test]
fn t_order_quoted_span_anchor_wins() {
    let q = r#"where is the "fog computing" summit held"#;
    let expo = hit(
        "https://expo.example.com/",
        "Cloud Expo",
        "cloud expo in Berlin this spring",
    );
    let fog = hit(
        "https://fog.example.com/summit",
        "Summit page",
        "the fog computing summit is in Berlin",
    );
    let out = order_candidates(q, vec![expo.clone(), fog.clone()], 5);
    assert_eq!(out, vec![fog.url, expo.url]);
}

#[test]
fn t_order_never_starves_without_matches() {
    // Anchor matches nothing: all hits score 0, original order preserved,
    // truncated to max — fetch slots are spent, never emptied.
    let q = r#""v2026.9.6-polyc-egress" release notes"#;
    let h1 = hit("https://a.example.com/", "A", "alpha page");
    let h2 = hit("https://b.example.com/", "B", "beta page");
    let h3 = hit("https://c.example.com/", "C", "gamma page");
    let out = order_candidates(q, vec![h1.clone(), h2.clone(), h3], 2);
    assert_eq!(out, vec![h1.url, h2.url]);
}

#[test]
fn t_order_max_zero_and_empty_hits() {
    let h = hit("https://a.example.com/", "A", "alpha");
    assert!(order_candidates("reddit discussion", vec![h], 0).is_empty());
    let empty: Vec<EngineHit> = vec![];
    assert!(order_candidates("anything at all", empty, 5).is_empty());
}

#[test]
fn t_order_max_truncates_to_top() {
    let q = "reddit discussion best local speech to speech models";
    let corp = hit("https://redditinc.com/blog", "Corp", "newsroom");
    let comm = hit("https://reddit.com/r/x/comments/y", "Thread", "discussion");
    let extra = hit("https://other.example.com/", "Other", "unrelated page");
    let out = order_candidates(q, vec![corp, comm.clone(), extra], 1);
    assert_eq!(out, vec![comm.url]);
}

#[test]
fn t_order_unicode_query_with_intent() {
    let q = "日本語 reddit discussion モデル";
    let corp = hit("https://redditinc.com/blog", "Corp", "newsroom");
    let comm = hit("https://reddit.com/r/x/comments/y", "Thread", "discussion");
    let out = order_candidates(q, vec![corp.clone(), comm.clone()], 5);
    assert_eq!(out, vec![comm.url, corp.url]);
}

#[test]
fn t_order_empty_query_preserves_input_order() {
    let h1 = hit("https://a.example.com/", "A", "alpha");
    let h2 = hit("https://b.example.com/", "B", "beta");
    let out = order_candidates("", vec![h1.clone(), h2.clone()], 10);
    assert_eq!(out, vec![h1.url, h2.url]);
}

// ---------------------------------------------------------------------------
// is_language_mismatched
// ---------------------------------------------------------------------------

#[test]
fn t_lang_cmp03_ru_pair() {
    // Exact live shape: Russian help URL + ASCII query => mismatched;
    // same URL + Russian query => not mismatched.
    let url = "https://support.google.com/youtube/answer/55756?hl=ru";
    assert!(is_language_mismatched(url, "how to upload a video"));
    assert!(!is_language_mismatched(url, "как загрузить видео"));
}

#[test]
fn t_lang_true_variants() {
    for (url, query) in [
        ("https://example.com/?hl=uk", "news today"),
        ("https://example.com/?HL=RU", "news today"),
        ("https://example.com/a?hl=zh-CN", "paper pdf"),
        ("https://example.com/?lr=lang_ja&x=1", "camera review"),
        ("https://example.com/?a=1&hl=ko", "lentil soup recipe"),
        ("https://example.com/?hl=ar", "weather forecast"),
    ] {
        assert!(
            is_language_mismatched(url, query),
            "expected mismatch for {url:?} with ASCII query"
        );
    }
}

#[test]
fn t_lang_false_variants() {
    for (url, query) in [
        // Latin-script hl markers are not mismatch signals.
        ("https://example.com/?hl=en", "news today"),
        ("https://example.com/?hl=fr", "news today"),
        // Bare locale path segments are not explicit markers (conservative).
        ("https://example.com/ru/support", "help page"),
        ("https://example.com/", "plain ascii query"),
        // Non-ASCII queries never fire, even with a marker present.
        ("https://example.com/?hl=ru", "как дела"),
        ("https://example.com/?hl=ar", "🎉 party live blog"),
    ] {
        assert!(
            !is_language_mismatched(url, query),
            "expected no mismatch for {url:?} / {query:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// EngineFanout::default (D7 lock)
// ---------------------------------------------------------------------------

#[test]
fn t_fanout_default_is_exactly_live_engines() {
    let engines = EngineFanout::default().engines().to_vec();
    assert_eq!(
        engines,
        vec![
            Engine::Duckduckgo,
            Engine::Bing,
            Engine::Yahoo,
            Engine::Brave,
            Engine::Wikipedia,
        ],
        "default fanout must be exactly the 5 live engines, in order"
    );
}
