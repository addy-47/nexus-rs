use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{FuturesOrdered, StreamExt};
use tokio::sync::Semaphore;

use crate::error::NexusError;

pub mod client;
pub mod connector;
pub mod dns;
pub mod redirect;

pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 512_000;
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_millis(4000);
pub const DEFAULT_FETCH_CONCURRENCY: usize = 3;

/// Hardened egress fetcher that guards against SSRF, DNS rebinding, and oversized payloads.
#[derive(Clone, Debug)]
pub struct EgressFetcher {
    timeout: Duration,
    max_response_bytes: usize,
}

impl Default for EgressFetcher {
    /// Constructs default egress fetcher with standard safety bounds.
    fn default() -> Self {
        Self {
            timeout: DEFAULT_FETCH_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

/// Result item for an individual candidate page fetch attempt.
#[derive(Debug)]
pub struct FetchedPageResult {
    /// Requested URL.
    pub requested_url: String,
    /// Result containing `(final_url, body_text)` on success or error on failure.
    pub outcome: Result<(String, String), NexusError>,
    /// Telemetry and latency metrics for this fetch attempt.
    pub metrics: crate::model::PageFetchMetrics,
}

impl FetchedPageResult {
    /// Returns true if the page fetch was successful.
    pub fn is_ok(&self) -> bool {
        self.outcome.is_ok()
    }

    /// Returns true if the page fetch resulted in an error.
    pub fn is_err(&self) -> bool {
        self.outcome.is_err()
    }
}

impl EgressFetcher {
    /// Constructs an egress fetcher with caller-specified timeout and byte bounds.
    pub fn new(timeout: Duration, max_response_bytes: usize) -> Self {
        Self {
            timeout,
            max_response_bytes,
        }
    }

    /// Returns a new EgressFetcher instance configured with overriding limits.
    pub fn with_limits(&self, timeout: Duration, max_response_bytes: usize) -> Self {
        Self {
            timeout,
            max_response_bytes,
        }
    }

    /// Fetches a single page by URL, applying full SSRF and redirect validation with metrics.
    pub async fn fetch_page(&self, url: &str) -> FetchedPageResult {
        let start = std::time::Instant::now();
        match redirect::fetch_with_redirect_vetting(url, self.timeout, self.max_response_bytes).await {
            Ok(res) => FetchedPageResult {
                requested_url: url.to_string(),
                outcome: Ok((res.final_url, res.body)),
                metrics: res.metrics,
            },
            Err(e) => FetchedPageResult {
                requested_url: url.to_string(),
                metrics: crate::model::PageFetchMetrics {
                    requested_url: url.to_string(),
                    final_url: None,
                    dns_resolution_ms: 0,
                    total_fetch_ms: start.elapsed().as_millis() as u64,
                    bytes_read: 0,
                    hop_count: 0,
                    success: false,
                    error: Some(e.to_string()),
                },
                outcome: Err(e),
            },
        }
    }

    /// Fetches multiple URLs concurrently while strictly preserving input order.
    pub async fn fetch_all_concurrent(
        &self,
        urls: &[String],
        concurrency: usize,
    ) -> Vec<FetchedPageResult> {
        let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
        let mut futures = FuturesOrdered::new();

        for url in urls {
            let sem = Arc::clone(&semaphore);
            let fetcher = self.clone();
            let target_url = url.clone();

            futures.push_back(async move {
                let _permit = sem.acquire().await;
                fetcher.fetch_page(&target_url).await
            });
        }

        let mut results = Vec::with_capacity(urls.len());
        while let Some(item) = futures.next().await {
            results.push(item);
        }
        results
    }
}
