//! ============================================================================
//! engine_parsing_test.rs — High-impact test suite for SERP parsing & deduplication
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::engines
//! Prerequisites: None (hermetic mock HTML parser)
//! Execution    : cargo test --test engine_parsing_test
//! Metrics      : SERP hit extraction accuracy, challenge detection, deduplication
//! ============================================================================

use std::time::Duration;

use nexus::Engine;
use nexus::engines::bing::parse_bing_html;
use nexus::engines::duckduckgo::parse_duckduckgo_html;
use nexus::engines::helpers::canonicalize_url;
use nexus::engines::mojeek::parse_mojeek_html;
use nexus::engines::yahoo::parse_yahoo_html;

#[tokio::test]
async fn test_duckduckgo_serp_parsing_and_uddg_decoding() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let ddg_html = r#"
            <div class="result results_links results_links_deep web-result">
                <h2 class="result__title">
                    <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F&rut=...">The Rust Programming Language</a>
                </h2>
                <a class="result__url" href="...">doc.rust-lang.org/book/</a>
                <a class="result__snippet">The Rust Programming Language is the official book on Rust.</a>
            </div>
        "#;

        let hits = parse_duckduckgo_html(ddg_html).expect("Parsing should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].engine, Engine::Duckduckgo);
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/book/");
        assert!(hits[0].snippet.contains("official book on Rust"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_duckduckgo_challenge_detection() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let challenge_html = r#"
            <html><body>
                <form id="challenge-form" action="/anomaly.js"></form>
            </body></html>
        "#;

        let result = parse_duckduckgo_html(challenge_html);
        assert!(result.is_err(), "Must report error on bot challenge");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_bing_serp_parsing() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let bing_html = r#"
            <ul>
                <li class="b_algo">
                    <h2><a href="https://www.rust-lang.org">Rust Programming Language</a></h2>
                    <div class="b_attribution"><cite>www.rust-lang.org</cite></div>
                    <div class="b_caption"><p>A language empowering everyone to build reliable software.</p></div>
                </li>
            </ul>
        "#;

        let hits = parse_bing_html(bing_html).expect("Bing parsing should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].engine, Engine::Bing);
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org");
        assert!(hits[0].snippet.contains("reliable software"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_yahoo_serp_parsing_and_ru_decoding() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let yahoo_html = r#"
            <div class="dd algo">
                <div class="compTitle">
                    <h3><a href="https://r.search.yahoo.com/_ylt=.../RU=https%3a%2f%2fcrates.io%2f/RK=2/...">crates.io: The Rust Package Registry</a></h3>
                </div>
                <div class="compText"><p>The Rust community's crate registry.</p></div>
            </div>
        "#;

        let hits = parse_yahoo_html(yahoo_html).expect("Yahoo parsing should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].engine, Engine::Yahoo);
        assert_eq!(hits[0].title, "crates.io: The Rust Package Registry");
        assert_eq!(hits[0].url, "https://crates.io/");
        assert!(hits[0].snippet.contains("crate registry"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_mojeek_serp_parsing() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mojeek_html = r#"
            <ul class="results-standard">
                <li>
                    <h2><a class="ob" href="https://rustup.rs">Install Rust with rustup</a></h2>
                    <p class="s">The primary way to install and manage Rust toolchains.</p>
                </li>
            </ul>
        "#;

        let hits = parse_mojeek_html(mojeek_html).expect("Mojeek parsing should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].engine, Engine::Mojeek);
        assert_eq!(hits[0].title, "Install Rust with rustup");
        assert_eq!(hits[0].url, "https://rustup.rs");
        assert!(
            hits[0]
                .snippet
                .contains("install and manage Rust toolchains")
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_canonicalize_url_strips_tracking_params_and_fragments() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let raw1 = "https://example.com/article/?utm_source=twitter&utm_medium=social#section2";
        let raw2 = "https://example.com/article?ref=homepage&fbclid=123456";

        let canonical1 = canonicalize_url(raw1);
        let canonical2 = canonicalize_url(raw2);

        assert_eq!(canonical1, "https://example.com/article");
        assert_eq!(canonical2, "https://example.com/article");
        assert_eq!(
            canonical1, canonical2,
            "Both URLs must normalize to identical canonical form"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_norm_url_key_collapses_variants_and_tracking() {
    use nexus::engines::helpers::norm_url_key;

    assert_eq!(
        norm_url_key("https://www.docs.rs/ratatui/index.html"),
        norm_url_key("http://docs.rs/ratatui/")
    );

    let tracked = norm_url_key(
        "https://example.com/search?topic=rust&utm_source=newsletter&page=2&gclid=abc",
    );
    let clean = norm_url_key("https://example.com/search?topic=rust&page=2");
    let different = norm_url_key("https://example.com/search?topic=rust&page=3");

    assert_eq!(tracked, clean);
    assert_ne!(clean, different);
}

#[tokio::test]
async fn test_google_wml_parsing_and_url_unescaping() {
    use nexus::engines::google_wml::parse_google_wml_html;

    let wml_html = r#"<?xml version="1.0" encoding="UTF-8"?>
    <div class="zMzFAb">
        <a class="fuLhoc" href="/url?q=https%3A%2F%2Fexample.org%2F%3Fa%3D1%26b%3D2&amp;sa=U">
            <span class="CVA68e">Rust &amp; TLS</span>
        </a>
        <div class="taTFJ"><span class="FrIlee">Native <b>HTTP</b> transport</span></div>
    </div>
    <div class="zMzFAb">
        <a class="fuLhoc" href="https://example.org/?a=1&amp;b=2">
            <span class="CVA68e">Duplicate Hit</span>
        </a>
    </div>
    <div class="zMzFAb">
        <a class="fuLhoc" href="https://example.org/distinct">
            <span class="CVA68e">Distinct Page</span>
        </a>
        <div class="taTFJ"><span class="FrIlee">Another snippet</span></div>
    </div>"#;

    let hits = parse_google_wml_html(wml_html).expect("Google WML parsing should succeed");
    assert_eq!(hits.len(), 2, "Duplicate URL must be suppressed");
    assert_eq!(hits[0].engine, Engine::GoogleWml);
    assert_eq!(hits[0].title, "Rust & TLS");
    assert_eq!(hits[0].url, "https://example.org/?a=1&b=2");
    assert!(hits[0].snippet.contains("Native HTTP transport"));

    assert_eq!(hits[1].title, "Distinct Page");
    assert_eq!(hits[1].url, "https://example.org/distinct");
}

#[tokio::test]
async fn test_google_wml_challenge_detection() {
    use nexus::engines::google_wml::parse_google_wml_html;

    let captcha_html = r#"<html><body><form id="captcha-form"></form></body></html>"#;
    assert!(parse_google_wml_html(captcha_html).is_err());

    let sorry_html = r#"<html><body>Please solve this /sorry/challenge</body></html>"#;
    assert!(parse_google_wml_html(sorry_html).is_err());
}
