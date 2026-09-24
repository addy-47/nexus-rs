use std::net::IpAddr;

use thiserror::Error;

/// Top-level error type for all nexus operations.
#[derive(Debug, Error)]
pub enum NexusError {
    /// Provided URL could not be parsed or contains an unsupported scheme.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Domain could not be resolved to an IP address.
    #[error("DNS resolution failed for host '{host}': {message}")]
    DnsResolution {
        /// Target host name.
        host: String,
        /// Detail of the DNS resolution failure.
        message: String,
    },

    /// Resolved address was rejected by the SSRF egress guard.
    #[error("Connection blocked by SSRF egress guard for IP: {0}")]
    PrivateIpBlocked(IpAddr),

    /// HTTP request exceeded the configured maximum redirect hops.
    #[error("Redirect limit exceeded ({max_hops} hops) for URL: {url}")]
    TooManyRedirects {
        /// Maximum hops allowed.
        max_hops: usize,
        /// Terminal URL attempted.
        url: String,
    },

    /// Download stream exceeded the configured maximum response byte limit.
    #[error("Response body exceeded byte limit of {limit_bytes} bytes for URL: {url}")]
    ResponseTooLarge {
        /// Maximum allowed bytes.
        limit_bytes: usize,
        /// Target URL.
        url: String,
    },

    /// HTTP transport error encountered during network egress.
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    /// Search scraper network transport error via TLS impersonation.
    #[error("Search scraper network error: {0}")]
    ScraperTransport(String),

    /// Search engine SERP HTML structure was unrecognizable or blocked.
    #[error("SERP parsing error for engine '{engine}': {message}")]
    SerpParse {
        /// Engine identifier.
        engine: String,
        /// Description of the parsing failure.
        message: String,
    },

    /// HTTP response returned a non-success status code.
    #[error("HTTP status {status} for URL: {url}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Request URL.
        url: String,
    },

    /// All enabled search engine providers failed to return results.
    #[error("All {attempted} search providers failed")]
    AllProvidersFailed {
        /// Number of providers attempted.
        attempted: usize,
    },

    /// Request exceeded the overall deadline.
    #[error("Operation timed out after {timeout_ms}ms for URL: {url}")]
    Timeout {
        /// Request URL.
        url: String,
        /// Timeout duration in milliseconds.
        timeout_ms: u64,
    },

    /// Downstream embedding generation failed.
    #[error("Text embedding failure: {0}")]
    Embedding(String),

    /// Invalid parameter supplied to the search pipeline.
    #[error("Configuration error: {0}")]
    InvalidConfiguration(String),

    /// Standard I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
