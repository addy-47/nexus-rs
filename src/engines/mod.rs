use std::collections::HashSet;

use futures_util::future::join_all;

use crate::error::NexusError;
use crate::model::{Engine, EngineHit, TimeFilter};

pub mod bing;
pub mod duckduckgo;
pub mod helpers;
pub mod mojeek;
pub mod yahoo;

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
        let valid_engines = if engines.is_empty() {
            vec![Engine::Duckduckgo, Engine::Bing, Engine::Yahoo]
        } else {
            engines
        };
        Self {
            engines: valid_engines,
        }
    }

    /// Queries all enabled engines concurrently and returns deduplicated hits.
    pub async fn query_all(&self, query: &str, time_filter: TimeFilter) -> Vec<EngineHit> {
        let client = match build_tls_client() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[Nexus::Engines] Failed to build TLS impersonation client: {e}");
                return Vec::new();
            }
        };

        let mut tasks = Vec::with_capacity(self.engines.len());
        for &engine in &self.engines {
            let client_ref = client.clone();
            let q = query.to_owned();
            tasks.push(tokio::spawn(async move {
                dispatch_engine_query(&client_ref, engine, &q, time_filter).await
            }));
        }

        let task_results = join_all(tasks).await;
        let mut all_hits = Vec::new();

        for (idx, res) in task_results.into_iter().enumerate() {
            let engine = self.engines[idx];
            match res {
                Ok(Ok(hits)) => {
                    log::info!("[Nexus::Engines] {engine} yielded {} hits", hits.len());
                    all_hits.push(hits);
                }
                Ok(Err(e)) => {
                    log::warn!("[Nexus::Engines] {engine} query error: {e}");
                }
                Err(join_err) => {
                    log::warn!("[Nexus::Engines] {engine} task panic/cancel: {join_err}");
                }
            }
        }

        interleave_and_deduplicate(all_hits)
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
        Engine::Bing => bing::query_bing(client, query).await,
        Engine::Yahoo => yahoo::query_yahoo(client, query, time_filter).await,
        Engine::Mojeek => mojeek::query_mojeek(client, query).await,
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
        (primp::Impersonate::ChromeV146, primp::ImpersonateOS::Windows),
        (primp::Impersonate::ChromeV146, primp::ImpersonateOS::MacOS),
        (primp::Impersonate::FirefoxV146, primp::ImpersonateOS::Windows),
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
