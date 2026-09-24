use std::collections::HashSet;

use futures_util::future::join_all;

use crate::error::NexusError;
use crate::model::{Engine, EngineHit, EngineQueryMetrics, TimeFilter};

pub mod bing;
pub mod duckduckgo;
pub mod helpers;
pub mod mojeek;
pub mod yahoo;

/// Result outcome containing deduplicated hits and granular per-engine telemetry.
#[derive(Clone, Debug, Default)]
pub struct FanoutOutcome {
    /// Deduplicated candidate hits.
    pub hits: Vec<EngineHit>,
    /// Per-engine query metrics.
    pub metrics: Vec<EngineQueryMetrics>,
    /// Total raw hits aggregated before deduplication.
    pub total_raw_hits: usize,
}

/// Multi-engine concurrent fanout orchestrator.
#[derive(Clone, Debug)]
pub struct EngineFanout {
    engines: Vec<Engine>,
}

impl Default for EngineFanout {
    /// Constructs default fanout searching DuckDuckGo, Bing, and Yahoo.
    fn default() -> Self {
        Self {
            engines: vec![Engine::Duckduckgo, Engine::Bing, Engine::Yahoo],
        }
    }
}

impl EngineFanout {
    /// Constructs a fanout orchestrator with caller-selected search engines.
    pub fn new(engines: Vec<Engine>) -> Self {
        Self { engines }
    }

    /// Queries all enabled engines concurrently and returns deduplicated hits with metrics.
    pub async fn query_all(
        &self,
        query: &str,
        time_filter: TimeFilter,
    ) -> Result<FanoutOutcome, NexusError> {
        if self.engines.is_empty() {
            return Ok(FanoutOutcome::default());
        }

        let client = build_tls_client()?;

        let tasks = self.engines.iter().map(|&engine| {
            let client_ref = client.clone();
            let q = query.to_owned();
            async move {
                let start = std::time::Instant::now();
                let res = dispatch_engine_query(&client_ref, engine, &q, time_filter).await;
                let latency_ms = start.elapsed().as_millis() as u64;
                (engine, res, latency_ms)
            }
        });

        let task_results = join_all(tasks).await;
        let mut all_hits = Vec::new();
        let mut engine_metrics = Vec::with_capacity(self.engines.len());
        let mut successful_queries = 0usize;
        let mut total_raw_hits = 0usize;

        for (engine, res, latency_ms) in task_results {
            match res {
                Ok(hits) => {
                    successful_queries += 1;
                    total_raw_hits += hits.len();
                    log::info!(
                        "[Nexus::Engines] {engine} yielded {} hits in {}ms",
                        hits.len(),
                        latency_ms
                    );
                    engine_metrics.push(EngineQueryMetrics {
                        engine,
                        latency_ms,
                        hit_count: hits.len(),
                        success: true,
                        error: None,
                    });
                    all_hits.push(hits);
                }
                Err(e) => {
                    log::warn!(
                        "[Nexus::Engines] {engine} query error after {}ms: {e}",
                        latency_ms
                    );
                    engine_metrics.push(EngineQueryMetrics {
                        engine,
                        latency_ms,
                        hit_count: 0,
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        if successful_queries == 0 && !self.engines.is_empty() {
            return Err(NexusError::AllProvidersFailed {
                attempted: self.engines.len(),
            });
        }

        let deduplicated = interleave_and_deduplicate(all_hits);
        Ok(FanoutOutcome {
            hits: deduplicated,
            metrics: engine_metrics,
            total_raw_hits,
        })
    }
}

/// Dispatches query to the appropriate engine module.
async fn dispatch_engine_query(
    client: &primp::Client,
    engine: Engine,
    query: &str,
    time_filter: TimeFilter,
) -> Result<Vec<EngineHit>, NexusError> {
    match engine {
        Engine::Duckduckgo => duckduckgo::query_duckduckgo(client, query, time_filter).await,
        Engine::Bing => bing::query_bing(client, query, time_filter).await,
        Engine::Yahoo => yahoo::query_yahoo(client, query, time_filter).await,
        Engine::Mojeek => mojeek::query_mojeek(client, query, time_filter).await,
    }
}

/// Interleaves hits from multiple engines in round-robin fashion while deduplicating URLs.
fn interleave_and_deduplicate(engine_batches: Vec<Vec<EngineHit>>) -> Vec<EngineHit> {
    let mut seen_canonical = HashSet::new();
    let mut deduplicated = Vec::new();
    let max_len = engine_batches.iter().map(Vec::len).max().unwrap_or(0);

    for i in 0..max_len {
        for batch in &engine_batches {
            if let Some(hit) = batch.get(i) {
                let canonical = helpers::canonicalize_url(&hit.url);
                if seen_canonical.insert(canonical) {
                    deduplicated.push(hit.clone());
                }
            }
        }
    }

    deduplicated
}

/// Builds a primp client with randomized TLS browser fingerprint impersonation.
fn build_tls_client() -> Result<primp::Client, NexusError> {
    let profiles = [
        (
            primp::Impersonate::ChromeV146,
            primp::ImpersonateOS::Windows,
        ),
        (primp::Impersonate::ChromeV146, primp::ImpersonateOS::MacOS),
        (
            primp::Impersonate::FirefoxV146,
            primp::ImpersonateOS::Windows,
        ),
    ];
    let pick = rand::random_range(0..profiles.len());
    let (browser, os) = profiles[pick];

    primp::Client::builder()
        .impersonate(browser)
        .impersonate_os(os)
        .timeout(std::time::Duration::from_millis(4500))
        .no_proxy()
        .build()
        .map_err(|e| NexusError::ScraperTransport(format!("Primp TLS client init failed: {e}")))
}
