//! ============================================================================
//! extraction_test.rs — High-impact test suite for DOM normalization & Markdown extraction
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::extraction
//! Prerequisites: None (hermetic document parser)
//! Execution    : cargo test --test extraction_test
//! Metrics      : Boilerplate removal rate, title accuracy
//! ============================================================================

use std::time::Duration;

use nexus::extraction::extract_document;

#[tokio::test]
async fn test_extraction_strips_scripts_and_navigation_boilerplates() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let raw_html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Rust Memory Safety Guide</title>
                <script>alert("malicious script execution!");</script>
                <style>body { background: red; }</style>
            </head>
            <body>
                <header>
                    <nav><a href="/home">Home</a><a href="/login">Login</a></nav>
                </header>
                <aside>
                    <div>Sidebar advertising banner</div>
                </aside>
                <main>
                    <h1>Core Ownership Principles</h1>
                    <p>Rust enforces memory safety without garbage collection through compile-time ownership rules.</p>
                    <p>References must always remain valid and data races are prevented at compile time.</p>
                </main>
                <form action="/newsletter"><input type="text"/></form>
                <footer><p>© 2026 Example Corp</p></footer>
            </body>
            </html>
        "#;

        let page = extract_document("https://example.com/rust", raw_html, 10_000);

        assert_eq!(page.title, "Rust Memory Safety Guide");
        assert!(
            page.markdown.contains("Core Ownership Principles")
                || page.markdown.contains("ownership rules"),
            "Markdown body must contain main content"
        );

        // Security & cleanup invariants
        assert!(!page.markdown.contains("alert("), "Scripts must be stripped");
        assert!(!page.markdown.contains("malicious"), "Script body must not leak");
        assert!(!page.markdown.contains("background: red"), "Styles must be stripped");
        assert!(!page.markdown.contains("Sidebar advertising"), "Sidebar must be stripped");
        assert!(!page.markdown.contains("Home"), "Navigation links must be stripped");
        assert!(!page.markdown.contains("© 2026"), "Footer must be stripped");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_extraction_handles_malformed_html_and_extracts_h1_fallback() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let malformed_html = r#"
            <div>
                <h1>Architecture Overview
                <p>Paragraph without closing tags
                <p>Second paragraph with unclosed <b>bold text
            </div>
        "#;

        let page = extract_document("https://example.com/arch", malformed_html, 5_000);

        assert_eq!(page.title, "Architecture Overview");
        assert!(page.markdown.contains("Paragraph without closing tags"));
        assert!(page.markdown.contains("Second paragraph"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_extraction_empty_html_falls_back_to_url() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let empty_html = "   ";
        let page = extract_document("https://example.com/fallback", empty_html, 5_000);

        assert_eq!(page.title, "https://example.com/fallback");
        assert!(page.markdown.is_empty());
    })
    .await
    .expect("test timed out");
}
