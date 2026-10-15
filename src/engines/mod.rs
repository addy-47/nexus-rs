use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::time::Instant;

use crate::error::NexusError;
use crate::model::{Engine, EngineHit, EngineQueryMetrics, FanoutPolicy, TimeFilter};

pub mod bing;
pub mod brave;
pub mod duckduckgo;
pub mod google_wml;
pub mod health;
pub mod helpers;
pub mod mojeek;
pub mod searxng;
pub mod verticals;
pub mod wikipedia;
pub mod yahoo;

pub use health::EngineHealthTracker;

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
    health: Arc<RwLock<EngineHealthTracker>>,
    searxng_url: Option<String>,
}

impl Default for EngineFanout {
    /// Constructs default fanout searching DuckDuckGo, Bing, and Yahoo.
    ///
    /// D7 (G3 baseline evidence): Mojeek serves a CAPTCHA wall and GoogleWml
    /// returns HTTP 403 on every query — both burn ~550-700ms of the 1200ms
    /// fanout budget for zero results. Removed from the active set until N2
    /// engine health + quarantine can gate them. Modules retained.
    fn default() -> Self {
        Self {
            engines: vec![
                Engine::Duckduckgo,
                Engine::Bing,
                Engine::Yahoo,
                Engine::Brave,
                Engine::Wikipedia,
            ],
            policy: FanoutPolicy::default(),
            health: Arc::new(RwLock::new(EngineHealthTracker::new())),
            searxng_url: None,
        }
    }
}

impl EngineFanout {
    /// Constructs a fanout orchestrator with caller-selected search engines and fanout policy.
    pub fn new(engines: Vec<Engine>, policy: FanoutPolicy) -> Self {
        Self {
            engines,
            policy,
            health: Arc::new(RwLock::new(EngineHealthTracker::new())),
            searxng_url: None,
        }
    }

    /// Constructs fanout with an explicitly shared health tracker.
    pub fn with_health(
        engines: Vec<Engine>,
        policy: FanoutPolicy,
        health: Arc<RwLock<EngineHealthTracker>>,
    ) -> Self {
        Self {
            engines,
            policy,
            health,
            searxng_url: None,
        }
    }

    /// Sets the SearXNG instance URL. The SearXNG engine is skipped at
    /// fanout time while this remains unset.
    pub fn with_searxng_url(mut self, url: impl Into<String>) -> Self {
        let trimmed = url.into().trim().to_string();
        self.searxng_url = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        self
    }

    /// Borrows the configured engine set.
    pub fn engines(&self) -> &[Engine] {
        &self.engines
    }

    /// Borrows the health tracker.
    pub fn health(&self) -> Arc<RwLock<EngineHealthTracker>> {
        Arc::clone(&self.health)
    }

    /// Queries all enabled engines concurrently with adaptive quorum early-exit and returns deduplicated hits with metrics.
    pub async fn query_all(
        &self,
        query: &str,
        time_filter: TimeFilter,
    ) -> Result<FanoutOutcome, NexusError> {
        self.query_all_with_deadline(query, time_filter, None).await
    }

    /// Queries all enabled engines concurrently with an explicit deadline and adaptive quorum early-exit.
    pub async fn query_all_with_deadline(
        &self,
        query: &str,
        time_filter: TimeFilter,
        deadline_override_ms: Option<u64>,
    ) -> Result<FanoutOutcome, NexusError> {
        if self.engines.is_empty() {
            return Ok(FanoutOutcome::default());
        }

        let client = build_tls_client()?;

        let mut tasks = FuturesUnordered::new();
        let mut engine_metrics = Vec::with_capacity(self.engines.len());

        for &engine in &self.engines {
            if engine == Engine::Searxng && self.searxng_url.is_none() {
                log::info!("[Nexus::Engines] Skipping SearXNG engine (no instance URL configured)");
                engine_metrics.push(EngineQueryMetrics {
                    engine,
                    latency_ms: 0,
                    hit_count: 0,
                    success: false,
                    error: Some("unconfigured".to_string()),
                });
                continue;
            }

            // Verticals fire only when the query looks like their material;
            // otherwise they burn budget and pollute the merge.
            if engine.is_vertical() && !verticals::should_query(engine, query) {
                log::info!(
                    "[Nexus::Engines] Skipping gated vertical {engine:?} (query shape mismatch)"
                );
                engine_metrics.push(EngineQueryMetrics {
                    engine,
                    latency_ms: 0,
                    hit_count: 0,
                    success: false,
                    error: Some("gated".to_string()),
                });
                continue;
            }

            if self
                .health
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .is_quarantined(engine)
            {
                log::info!("[Nexus::Engines] Skipping quarantined engine {:?}", engine);
                engine_metrics.push(EngineQueryMetrics {
                    engine,
                    latency_ms: 0,
                    hit_count: 0,
                    success: false,
                    error: Some("quarantined".to_string()),
                });
                continue;
            }

            let client_ref = client.clone();
            let q = query.to_owned();
            let searxng_url = self.searxng_url.clone();
            tasks.push(async move {
                let start = std::time::Instant::now();
                let res_unwind = AssertUnwindSafe(dispatch_engine_query(
                    &client_ref,
                    engine,
                    &q,
                    time_filter,
                    searxng_url.as_deref(),
                ))
                .catch_unwind()
                .await;
                let latency_ms = start.elapsed().as_millis() as u64;
                let res = match res_unwind {
                    Ok(r) => r,
                    Err(p) => {
                        let panic_msg = if let Some(s) = p.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = p.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Unknown panic in engine dispatcher".to_string()
                        };
                        log::error!(
                            "[Nexus::Engines] Engine {:?} panicked: {}",
                            engine,
                            panic_msg
                        );
                        Err(NexusError::SerpParse {
                            engine: engine.to_string(),
                            message: format!("engine panicked during query: {}", panic_msg),
                        })
                    }
                };
                (engine, res, latency_ms)
            });
        }

        let deadline_ms = deadline_override_ms.unwrap_or(self.policy.max_fanout_deadline_ms);
        let deadline = Instant::now() + Duration::from_millis(deadline_ms);
        let mut all_hits = Vec::new();
        let mut successful_queries = 0usize;
        // Engines that actually contributed candidates. Empty-but-successful
        // responses (e.g. a gated vertical's API with no matches) must not
        // satisfy quorum: they carry no domains or hits, yet previously
        // counted toward early exit and dropped slower engines mid-flight.
        let mut contributing_engines = 0usize;
        let mut total_raw_hits = 0usize;
        let mut seen_domains = HashSet::new();
        let mut seen_canonical = HashSet::new();

        loop {
            let timeout_remaining = deadline.saturating_duration_since(Instant::now());
            if timeout_remaining.is_zero() {
                log::info!(
                    "[Nexus::Engines] Fanout deadline reached ({}ms). Proceeding with {} successful engine responses.",
                    deadline_ms,
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
                            let mut h = self.health.write().unwrap_or_else(|e| e.into_inner());
                            h.record_success(engine, latency_ms);
                            successful_queries += 1;
                            if !hits.is_empty() {
                                contributing_engines += 1;
                            }
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
                            // only contributing engines count — empty responses carry no candidates to exit on.
                            if contributing_engines >= self.policy.min_reporting_engines
                                && seen_domains.len() >= self.policy.min_distinct_domains
                                && seen_canonical.len() >= self.policy.min_candidate_hits
                            {
                                log::info!(
                                    "[Nexus::Engines] Early-exit quorum satisfied (contributing_engines={}, distinct_domains={}, candidate_hits={}). Dropping tail engine queries.",
                                    contributing_engines,
                                    seen_domains.len(),
                                    seen_canonical.len()
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            let err_str = e.to_string();
                            let is_hard_block = {
                                let lower = err_str.to_ascii_lowercase();
                                lower.contains("captcha")
                                    || lower.contains("challenge")
                                    || lower.contains("403")
                                    || lower.contains("429")
                                    || lower.contains("blocked")
                            };
                            let mut h = self.health.write().unwrap_or_else(|e| e.into_inner());
                            h.record_failure(engine, is_hard_block);
                            log::warn!(
                                "[Nexus::Engines] {engine} query error after {}ms: {err_str}",
                                latency_ms
                            );
                            engine_metrics.push(EngineQueryMetrics {
                                engine,
                                latency_ms,
                                hit_count: 0,
                                success: false,
                                error: Some(err_str),
                            });
                        }
                    }
                }
                _ = tokio::time::sleep(timeout_remaining) => {
                    log::info!(
                        "[Nexus::Engines] Fanout deadline timeout elapsed ({}ms). Proceeding with {} completed engines.",
                        deadline_ms,
                        successful_queries
                    );
                    break;
                }
            }
        }

        if successful_queries == 0 && !self.engines.is_empty() {
            // Nothing was attempted (every engine skipped: quarantined or
            // unconfigured). That is an empty result with reasons attached,
            // not a transport failure — return the metrics, not an error.
            let attempted = engine_metrics
                .iter()
                .filter(|m| {
                    !matches!(
                        m.error.as_deref(),
                        Some("quarantined") | Some("unconfigured") | Some("gated")
                    )
                })
                .count();
            if attempted == 0 {
                return Ok(FanoutOutcome {
                    hits: Vec::new(),
                    metrics: engine_metrics,
                    total_raw_hits,
                    consensus_urls: HashSet::new(),
                });
            }
            return Err(NexusError::AllProvidersFailed {
                attempted,
                metrics: engine_metrics,
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
    searxng_url: Option<&str>,
) -> Result<Vec<EngineHit>, NexusError> {
    if std::env::var_os("NEXUS_TEST_HOOKS").is_some() {
        if query == "__test_panic_engine__" {
            if engine == Engine::Mojeek {
                panic!("Simulated engine panic in Mojeek parser");
            } else if engine == Engine::Duckduckgo {
                return Ok(vec![EngineHit {
                    title: "Healthy Engine Result".to_string(),
                    url: "https://example.com/healthy".to_string(),
                    display_url: "https://example.com/healthy".to_string(),
                    snippet: "This result survived another engine's panic.".to_string(),
                    engine: Engine::Duckduckgo,
                }]);
            }
        }

        if query.starts_with("__test_hermetic_") {
            return Ok(Vec::new());
        }
    }

    match engine {
        Engine::Duckduckgo => duckduckgo::query_duckduckgo(client, query, time_filter).await,
        Engine::Bing => bing::query_bing(client, query, time_filter).await,
        Engine::Yahoo => yahoo::query_yahoo(client, query, time_filter).await,
        Engine::Mojeek => mojeek::query_mojeek(client, query, time_filter).await,
        Engine::GoogleWml => google_wml::query_google_wml(client, query, time_filter).await,
        Engine::Brave => brave::query_brave(client, query, time_filter).await,
        Engine::Searxng => match searxng_url {
            Some(url) => searxng::query_searxng(client, url, query, time_filter).await,
            None => Err(NexusError::InvalidConfiguration(
                "SearXNG engine enabled without an instance URL".to_string(),
            )),
        },
        Engine::Arxiv
        | Engine::Scholar
        | Engine::StackExchange
        | Engine::Mdn
        | Engine::Github
        | Engine::HackerNews
        | Engine::NewsRss => verticals::query_vertical(client, engine, query, time_filter).await,
        Engine::Wikipedia => wikipedia::query_wikipedia(client, query).await,
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
