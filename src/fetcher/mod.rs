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

    /// Fetches a single page by URL, applying full SSRF and redirect validation.
    /// Returns `(final_url, body_content)`.
    pub async fn fetch_page(&self, url: &str) -> Result<(String, String), NexusError> {
        redirect::fetch_with_redirect_vetting(url, self.timeout, self.max_response_bytes).await
    }

    /// Fetches multiple URLs concurrently while strictly preserving input order.
    pub async fn fetch_all_concurrent(
        &self,
        urls: &[String],
        concurrency: usize,
    ) -> Vec<(String, Result<(String, String), NexusError>)> {
        let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
        let mut futures = FuturesOrdered::new();

        for url in urls {
            let sem = Arc::clone(&semaphore);
            let fetcher = self.clone();
            let target_url = url.clone();

            futures.push_back(async move {
                let _permit = sem.acquire().await;
                let result = fetcher.fetch_page(&target_url).await;
                (target_url, result)
            });
        }

        let mut results = Vec::with_capacity(urls.len());
        while let Some(item) = futures.next().await {
            results.push(item);
        }
        results
    }
}
