use std::collections::HashSet;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::time::Instant;

use crate::error::NexusError;
use crate::model::{Engine, EngineHit, EngineQueryMetrics, FanoutPolicy, TimeFilter};

pub mod bing;
pub mod duckduckgo;
pub mod google_wml;
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
    /// Normalized URL keys corroborated across >= 2 distinct index families.
    pub consensus_urls: HashSet<String>,
}

/// Multi-engine concurrent fanout orchestrator with adaptive quorum early-exit.
#[derive(Clone, Debug)]
pub struct EngineFanout {
    engines: Vec<Engine>,
    policy: FanoutPolicy,
}

impl Default for EngineFanout {
    /// Constructs default fanout searching DuckDuckGo, Bing, Yahoo, and Mojeek.
    fn default() -> Self {
        Self {
            engines: vec![
                Engine::Duckduckgo,
                Engine::Bing,
                Engine::Yahoo,
                Engine::Mojeek,
            ],
            policy: FanoutPolicy::default(),
        }
    }
}

impl EngineFanout {
    /// Constructs a fanout orchestrator with caller-selected search engines and fanout policy.
    pub fn new(engines: Vec<Engine>, policy: FanoutPolicy) -> Self {
        Self { engines, policy }
    }

    /// Queries all enabled engines concurrently with adaptive quorum early-exit and returns deduplicated hits with metrics.
    pub async fn query_all(
        &self,
        query: &str,
        time_filter: TimeFilter,
    ) -> Result<FanoutOutcome, NexusError> {
        if self.engines.is_empty() {
            return Ok(FanoutOutcome::default());
        }

        let client = build_tls_client()?;

        let mut tasks = FuturesUnordered::new();
        for &engine in &self.engines {
            let client_ref = client.clone();
            let q = query.to_owned();
            tasks.push(async move {
                let start = std::time::Instant::now();
                let res = dispatch_engine_query(&client_ref, engine, &q, time_filter).await;
                let latency_ms = start.elapsed().as_millis() as u64;
                (engine, res, latency_ms)
            });
        }

        let deadline = Instant::now() + Duration::from_millis(self.policy.max_fanout_deadline_ms);
        let mut all_hits = Vec::new();
        let mut engine_metrics = Vec::with_capacity(self.engines.len());
        let mut successful_queries = 0usize;
        let mut total_raw_hits = 0usize;
        let mut seen_domains = HashSet::new();
        let mut seen_canonical = HashSet::new();

        loop {
            let timeout_remaining = deadline.saturating_duration_since(Instant::now());
            if timeout_remaining.is_zero() {
                log::info!(
                    "[Nexus::Engines] Fanout deadline reached ({}ms). Proceeding with {} successful engine responses.",
                    self.policy.max_fanout_deadline_ms,
                    successful_queries
                );
                break;
            }

            tokio::select! {
                next_res = tasks.next() => {
                    let Some((engine, res, latency_ms)) = next_res else {
                        break;
                    };

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

                            for hit in &hits {
                                let domain = helpers::extract_domain(&hit.url);
                                if !domain.is_empty() {
                                    seen_domains.insert(domain);
                                }
                                seen_canonical.insert(helpers::norm_url_key(&hit.url));
                            }
                            all_hits.push(hits);

                            // Check adaptive quorum early-exit condition:
                            if successful_queries >= self.policy.min_reporting_engines
                                && seen_domains.len() >= self.policy.min_distinct_domains
                                && seen_canonical.len() >= self.policy.min_candidate_hits
                            {
                                log::info!(
                                    "[Nexus::Engines] Early-exit quorum satisfied (reporting_engines={}, distinct_domains={}, candidate_hits={}). Dropping tail engine queries.",
                                    successful_queries,
                                    seen_domains.len(),
                                    seen_canonical.len()
                                );
                                break;
                            }
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
                _ = tokio::time::sleep(timeout_remaining) => {
                    log::info!(
                        "[Nexus::Engines] Fanout deadline timeout elapsed ({}ms). Proceeding with {} completed engines.",
                        self.policy.max_fanout_deadline_ms,
                        successful_queries
                    );
                    break;
                }
            }
        }

        if successful_queries == 0 && !self.engines.is_empty() {
            return Err(NexusError::AllProvidersFailed {
                attempted: self.engines.len(),
            });
        }

        let mut family_by_url: std::collections::HashMap<String, HashSet<&'static str>> =
            std::collections::HashMap::new();
        for batch in &all_hits {
            for hit in batch {
                let key = helpers::norm_url_key(&hit.url);
                family_by_url
                    .entry(key)
                    .or_default()
                    .insert(hit.engine.index_family());
            }
        }
        let consensus_urls: HashSet<String> = family_by_url
            .into_iter()
            .filter(|(_, families)| families.len() >= 2)
            .map(|(url, _)| url)
            .collect();

        let deduplicated = interleave_and_deduplicate(all_hits);
        Ok(FanoutOutcome {
            hits: deduplicated,
            metrics: engine_metrics,
            total_raw_hits,
            consensus_urls,
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
        Engine::GoogleWml => google_wml::query_google_wml(client, query, time_filter).await,
        Engine::Brave => Err(NexusError::InvalidConfiguration(
            "Brave search engine is not configured".to_string(),
        )),
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
                let key = helpers::norm_url_key(&hit.url);
                if seen_canonical.insert(key) {
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
