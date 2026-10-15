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
use nexus::engines::duckduckgo::parse_duckduckgo_lite_html;
use nexus::engines::helpers::canonicalize_url;
use nexus::engines::mojeek::parse_mojeek_html;
use nexus::engines::searxng::parse_searxng_json;
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

#[tokio::test]
async fn test_brave_serp_parsing() {
    use nexus::engines::brave::parse_brave_html;

    let brave_html = r#"
        <div class="snippet">
            <a class="snippet-title" href="https://example.com/rust">Rust Programming</a>
            <p class="snippet-description">Rust is a systems programming language.</p>
        </div>
    "#;
    let hits = parse_brave_html(brave_html).expect("Brave parsing should succeed");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].engine, Engine::Brave);
    assert_eq!(hits[0].title, "Rust Programming");
    assert_eq!(hits[0].url, "https://example.com/rust");
    assert!(hits[0].snippet.contains("systems programming language"));
}

#[tokio::test]
async fn test_brave_challenge_detection() {
    use nexus::engines::brave::parse_brave_html;

    let challenge_html = r#"<html><body><div class="cf-browser-verification"></div></body></html>"#;
    assert!(parse_brave_html(challenge_html).is_err());
}

#[tokio::test]
async fn test_wikipedia_json_parsing() {
    use nexus::engines::wikipedia::parse_wikipedia_json;

    let wiki_json = br#"
    {
        "pages": [
            {
                "key": "Rust_(programming_language)",
                "title": "Rust (programming language)",
                "excerpt": "Rust is a multi-paradigm, general-purpose programming language.",
                "description": "General-purpose programming language"
            }
        ]
    }
    "#;
    let hits = parse_wikipedia_json(wiki_json).expect("Wikipedia parsing should succeed");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].engine, Engine::Wikipedia);
    assert_eq!(hits[0].title, "Rust (programming language)");
    assert_eq!(
        hits[0].url,
        "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
    assert!(hits[0].snippet.contains("multi-paradigm"));
}

#[tokio::test]
async fn test_duckduckgo_lite_table_parsing_and_uddg_decoding() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let lite_html = r#"
            <table>
                <tr>
                    <td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage1&rut=abc123">First Result Title</a></td>
                    <td class="result-snippet">First snippet text.</td>
                </tr>
                <tr>
                    <td><a class="result-link" href="https://example.org/page2">Second Result Title</a></td>
                    <td class="result-snippet">Second snippet text.</td>
                </tr>
            </table>
        "#;

        let hits = parse_duckduckgo_lite_html(lite_html).expect("Lite parsing should succeed");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].engine, Engine::Duckduckgo);
        assert_eq!(hits[1].engine, Engine::Duckduckgo);
        assert_eq!(hits[0].title, "First Result Title");
        assert_eq!(hits[0].url, "https://example.com/page1");
        assert!(hits[0].snippet.contains("First snippet text"));
        assert_eq!(hits[1].title, "Second Result Title");
        assert_eq!(hits[1].url, "https://example.org/page2");
        assert!(hits[1].snippet.contains("Second snippet text"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_duckduckgo_lite_challenge_detection() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let anomaly_html = r#"<html><body><p>anomaly detected</p></body></html>"#;
        assert!(
            parse_duckduckgo_lite_html(anomaly_html).is_err(),
            "Must report error on anomaly marker"
        );

        let captcha_html = r#"<html><body><p>captcha required</p></body></html>"#;
        assert!(
            parse_duckduckgo_lite_html(captcha_html).is_err(),
            "Must report error on captcha marker"
        );

        let form_html =
            r#"<html><body><form id="challenge-form" action="/lite/"></form></body></html>"#;
        assert!(
            parse_duckduckgo_lite_html(form_html).is_err(),
            "Must report error on challenge-form marker"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_duckduckgo_lite_skips_relative_and_non_http_hrefs() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let lite_html = r#"
            <table>
                <tr>
                    <td><a class="result-link" href="/lite/?q=rust">Relative Link</a></td>
                    <td class="result-snippet">Relative snippet.</td>
                </tr>
                <tr>
                    <td><a class="result-link" href="javascript:void(0)">JS Link</a></td>
                    <td class="result-snippet">JS snippet.</td>
                </tr>
                <tr>
                    <td><a class="result-link" href="">Empty Link</a></td>
                    <td class="result-snippet">Empty snippet.</td>
                </tr>
                <tr>
                    <td><a class="result-link" href="https://example.com/valid">Valid Title</a></td>
                    <td class="result-snippet">Valid snippet.</td>
                </tr>
            </table>
        "#;

        let hits = parse_duckduckgo_lite_html(lite_html).expect("Lite parsing should succeed");
        assert_eq!(hits.len(), 1, "Only the absolute http href must survive");
        assert_eq!(hits[0].title, "Valid Title");
        assert_eq!(hits[0].url, "https://example.com/valid");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_searxng_json_parsing() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let body = r#"{
            "results": [
                {"title": "Rust Programming Language", "url": "https://www.rust-lang.org/", "content": "A language empowering everyone to build reliable software."},
                {"title": "crates.io", "url": "http://crates.io/", "content": "The Rust package registry."}
            ]
        }"#;

        let hits = parse_searxng_json(body).expect("SearXNG parsing should succeed");
        assert_eq!(hits.len(), 2);
        for hit in &hits {
            assert_eq!(hit.engine, Engine::Searxng);
        }
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert!(hits[0].snippet.contains("reliable software"));
        assert_eq!(hits[1].title, "crates.io");
        assert_eq!(hits[1].url, "http://crates.io/");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_searxng_json_skips_non_http_urls_and_empty_titles() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let body = r#"{
            "results": [
                {"title": "Valid Hit", "url": "https://example.com/valid", "content": "Kept."},
                {"title": "FTP Hit", "url": "ftp://example.com/file", "content": "Dropped: non-http."},
                {"title": "Relative Hit", "url": "/search?q=rust", "content": "Dropped: non-http."},
                {"title": "", "url": "https://example.com/empty-title", "content": "Dropped: empty title."},
                {"title": "   ", "url": "https://example.com/blank-title", "content": "Dropped: blank title."},
                {"url": "https://example.com/missing-title", "content": "Dropped: missing title."},
                {"title": "Missing URL", "content": "Dropped: missing url."}
            ]
        }"#;

        let hits = parse_searxng_json(body).expect("SearXNG parsing should succeed");
        assert_eq!(hits.len(), 1, "Only the valid http hit with a title must survive");
        assert_eq!(hits[0].title, "Valid Hit");
        assert_eq!(hits[0].url, "https://example.com/valid");
        assert_eq!(hits[0].engine, Engine::Searxng);
    })
    .await
    .expect("test timed out");
}
