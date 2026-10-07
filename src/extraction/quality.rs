//! Page-level quality gates: reject sources that can never yield answer-bearing
//! evidence, before they consume chunking, ranking, and context budget.
//!
//! P0-3/P0-4 (G3 baseline): the batch delivered 22,540 chars of minified Closure
//! JavaScript as 5 passages (`cmp_03`), 41,930 chars of Adobe Target JS as 2
//! passages (`ent_01`), and a "Your request has been blocked" page scored at
//! 0.033 (`ent_03`). All three passed every existing gate because no gate
//! inspects content shape.

/// Markers that identify bot-challenge, block, and error-shell pages.
///
/// Matched case-insensitively against the extracted title first (cheap), then
/// against a bounded prefix of the body. A page matching any marker is rejected
/// as a source regardless of its rank score.
const CHALLENGE_MARKERS: &[&str] = &[
    "your request has been blocked",
    "request has been blocked",
    "appears to be from an automated process",
    "automated process",
    "if this is incorrect, please click this link",
    "turn on cookies in your browser",
    "enable cookies in your browser",
    "please verify you are a human",
    "verify you are human",
    "captcha",
    "recaptcha",
    "cloudflare",
    "attention required",
    "access denied",
    "access to this page has been denied",
    "403 forbidden",
    "404 not found",
    "500 internal server error",
    "service unavailable",
    "sign in to continue",
    "log in to continue",
    "javascript is required",
    "please enable javascript",
];

/// Returns true when the page is a bot-challenge, block, or error shell.
///
/// Checks the title (a block page announces itself there: `ent_03`'s title was
/// "Your request has been blocked") and the first 2,000 chars of the body.
/// Bounded and allocation-free beyond lowercasing the inspected windows.
pub fn is_challenge_or_error_page(title: &str, markdown: &str) -> bool {
    let title_lower = title.to_ascii_lowercase();
    if CHALLENGE_MARKERS.iter().any(|m| title_lower.contains(m)) {
        return true;
    }
    let body_window: String = markdown.chars().take(2000).collect();
    let body_lower = body_window.to_ascii_lowercase();
    CHALLENGE_MARKERS.iter().any(|m| body_lower.contains(m))
}

/// Returns true when the text is dominated by source code rather than prose.
///
/// Heuristic over three independent signals; at least two must fire so that a
/// legitimate code sample embedded in a docs page does not nuke the page:
/// 1. **Punctuation density**: code is dense in `{ } ; = ( )` where prose is not.
/// 2. **camelCase/snake density**: identifiers like `ContextHubJQ` or
///    `campaign_path` are near-absent from prose.
/// 3. **Minified shape**: very long lines (no wrapping) with high symbol ratio —
///    the exact shape of the `cmp_03` Closure payload and the `ent_01` campaign JSON.
pub fn is_code_like(text: &str) -> bool {
    let sample: String = text.chars().take(8000).collect();
    if sample.trim().is_empty() {
        return false;
    }

    let total = sample.chars().count() as f32;

    // Signal 1: code punctuation density.
    let punct = sample
        .chars()
        .filter(|c| matches!(c, '{' | '}' | ';' | '=' | '(' | ')' | '[' | ']' | '$' | '_'))
        .count() as f32
        / total;

    // Signal 2: identifier density (camelCase transitions + snake_case + dots in chains).
    let mut camel_transitions = 0u32;
    let mut prev_lower = false;
    let mut snake_or_dotted = 0u32;
    for c in sample.chars() {
        if c.is_ascii_lowercase() {
            prev_lower = true;
        } else {
            if c.is_ascii_uppercase() && prev_lower {
                camel_transitions += 1;
            }
            prev_lower = false;
        }
        if c == '_' || c == '.' {
            snake_or_dotted += 1;
        }
    }
    let words = sample.split_whitespace().count().max(1) as f32;
    let ident_density = (camel_transitions + snake_or_dotted) as f32 / words;

    // Signal 3: minified shape — mean line length over the sample.
    let lines = sample.lines().count().max(1) as f32;
    let mean_line_len = total / lines;
    let minified = mean_line_len > 500.0 && punct > 0.08;

    let signals = [punct > 0.12, ident_density > 0.6, minified]
        .iter()
        .filter(|&&s| s)
        .count();
    signals >= 2
}
// NOTE (D8): no `#[cfg(test)]` module here by design. All tests for this module —
// fixtures from the G3 shapes (`cmp_03` Closure JS, `ent_01` Adobe JS, `ent_03`
// block title), boundary cases, adversarial inputs — are owned by the testing
// subagent per plan §11.4. Verification of this gate happens via the eval batch.

/// Extracts entity anchors from a query: quoted spans plus version/identifier
/// shaped tokens (`all-MiniLM-L6-v2`, `polyc-egress`, `v2026.9.6`, `3.75%`).
///
/// Anchors are the load-bearing tokens a correct result must contain. G3 `ent_01`:
/// Extracts concrete anchor entities (quoted phrases, versions, identifiers, years).
pub fn entity_anchors(query: &str) -> Vec<String> {
    let mut anchors = Vec::new();
    // Quoted spans first — explicit user intent.
    let mut rest = query;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('"') {
            let span = after[..end].trim();
            if !span.is_empty() {
                anchors.push(span.to_ascii_lowercase());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    // Identifier-shaped tokens: contain a digit, or a hyphen/slash/dot joining
    // alphanumerics (crate names, model ids, versions). Plain English words
    // (`all`, `best`, `reddit`) are deliberately NOT anchors.
    for tok in query.split_whitespace() {
        let t = tok.trim_matches(|c: char| !c.is_alphanumeric());
        if t.len() < 3 || anchors.iter().any(|a| a == &t.to_ascii_lowercase()) {
            continue;
        }
        let has_digit = t.chars().any(|c| c.is_ascii_digit());
        let has_joiner = t.chars().any(|c| matches!(c, '-' | '/' | '.' | '_' | '+'));
        if has_digit || (has_joiner && t.chars().any(|c| c.is_alphabetic())) {
            anchors.push(t.to_ascii_lowercase());
        }
    }
    anchors
}

/// Community/property mentions mapped to the domains that satisfy them.
///
/// G3 `wall_02`: query said "reddit discussion", pipeline served `redditinc.com`.
fn domain_intent_domains(query: &str) -> Vec<&'static str> {
    let q = query.to_ascii_lowercase();
    let mut out = Vec::new();
    let table: &[(&str, &[&str])] = &[
        ("reddit", &["reddit.com"]),
        ("github", &["github.com"]),
        ("stackoverflow", &["stackoverflow.com"]),
        ("stack exchange", &["stackexchange.com"]),
        ("arxiv", &["arxiv.org"]),
        ("wikipedia", &["wikipedia.org"]),
        ("docs.rs", &["docs.rs"]),
        ("crates.io", &["crates.io", "lib.rs"]),
        ("mdn", &["developer.mozilla.org"]),
        ("hacker news", &["news.ycombinator.com"]),
    ];
    for (mention, domains) in table {
        if q.contains(mention) {
            out.extend_from_slice(domains);
        }
    }
    out
}

/// Extracts apex domain from a URL to enforce domain diversity across candidate sources.
pub fn extract_apex_domain(url: &str) -> String {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_ascii_lowercase()))
        .unwrap_or_default();
    if host.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() <= 2 {
        host
    } else {
        // Handle common ccSLDs e.g. co.uk, com.au, org.uk, etc.
        let last = parts[parts.len() - 1];
        let second_last = parts[parts.len() - 2];
        if last.len() == 2 && matches!(second_last, "co" | "com" | "org" | "gov" | "edu" | "ac" | "net") {
            if parts.len() >= 3 {
                format!("{}.{}.{}", parts[parts.len() - 3], second_last, last)
            } else {
                host
            }
        } else {
            format!("{}.{}", parts[parts.len() - 2], last)
        }
    }
}

/// Reorders candidate hits so anchored and intent-matching sources are fetched
/// first, while enforcing domain diversity (max 1-2 per apex domain).
pub fn order_candidates(
    query: &str,
    mut hits: Vec<crate::model::EngineHit>,
    max_candidates: usize,
) -> Vec<String> {
    let anchors = entity_anchors(query);
    let intent_domains = domain_intent_domains(query);

    if !anchors.is_empty() || !intent_domains.is_empty() {
        let mut scored: Vec<(u8, usize)> = Vec::with_capacity(hits.len());
        for (i, h) in hits.iter().enumerate() {
            let haystack = format!("{} {} {}", h.url, h.title, h.snippet).to_ascii_lowercase();
            let mut rank: u8 = 0;
            if anchors.iter().any(|a| haystack.contains(a)) {
                rank += 2;
            }
            if intent_domains.iter().any(|d| haystack.contains(d)) {
                rank += 1;
            }
            scored.push((rank, i));
        }
        // Stable: higher rank first, original order otherwise.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let ordered: Vec<crate::model::EngineHit> =
            scored.into_iter().map(|(_, i)| hits[i].clone()).collect();
        hits = ordered;
    }

    let mut selected_urls = Vec::with_capacity(max_candidates);
    let mut domain_counts = std::collections::HashMap::new();

    // Pass 1: max 1 per apex domain to ensure variety across sources
    for hit in &hits {
        if selected_urls.len() >= max_candidates {
            break;
        }
        let apex = extract_apex_domain(&hit.url);
        let count = domain_counts.entry(apex).or_insert(0usize);
        if *count < 1 {
            *count += 1;
            selected_urls.push(hit.url.clone());
        }
    }

    // Pass 2: if slots remain, allow up to 2 per apex domain
    if selected_urls.len() < max_candidates {
        for hit in &hits {
            if selected_urls.len() >= max_candidates {
                break;
            }
            if selected_urls.iter().any(|u| u == &hit.url) {
                continue;
            }
            let apex = extract_apex_domain(&hit.url);
            let count = domain_counts.entry(apex).or_insert(0usize);
            if *count < 2 {
                *count += 1;
                selected_urls.push(hit.url.clone());
            }
        }
    }

    // Pass 3: fill any remainder if candidate pool is tiny
    if selected_urls.len() < max_candidates {
        for hit in &hits {
            if selected_urls.len() >= max_candidates {
                break;
            }
            if !selected_urls.iter().any(|u| u == &hit.url) {
                selected_urls.push(hit.url.clone());
            }
        }
    }

    selected_urls
}

/// Detects a language mismatch signalled by the URL itself.
///
/// A `hl=`/`lr=`/locale path segment naming a non-Latin script family while the
/// query is plain ASCII is a strong wrong-language signal (G3 `cmp_03`: `?hl=ru`
/// served Russian for an English query). Conservative by design: only fires on
/// explicit URL markers, never on content sniffing.
pub fn is_language_mismatched(url: &str, query: &str) -> bool {
    if !query.is_ascii() {
        return false;
    }
    let u = url.to_ascii_lowercase();
    const NON_LATIN_HL: [&str; 20] = [
        "hl=ru",
        "hl=uk",
        "hl=be",
        "hl=zh",
        "hl=ja",
        "hl=ko",
        "hl=ar",
        "hl=he",
        "hl=hi",
        "hl=th",
        "hl=el",
        "hl=bg",
        "hl=sr",
        "hl=fa",
        "hl=ur",
        "lr=lang_ru",
        "lr=lang_zh",
        "lr=lang_ja",
        "lr=lang_ko",
        "lr=lang_ar",
    ];
    NON_LATIN_HL.iter().any(|m| u.contains(m))
}
