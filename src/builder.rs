use std::sync::Arc;
use std::time::Duration;

use crate::engines::EngineFanout;
use crate::error::NexusError;
use crate::fetcher::{
    DEFAULT_FETCH_CONCURRENCY, DEFAULT_FETCH_TIMEOUT, DEFAULT_MAX_RESPONSE_BYTES, EgressFetcher,
};
use crate::model::{Engine, FanoutPolicy, RankingPolicy};
use crate::pipeline::NexusSearch;
use crate::traits::TextEmbedder;

/// Fluent builder for constructing a configured NexusSearch engine.
#[derive(Clone, Default)]
pub struct NexusSearchBuilder {
    engines: Option<Vec<Engine>>,
    fanout_policy: Option<FanoutPolicy>,
    ranking_policy: Option<RankingPolicy>,
    embedder: Option<Arc<dyn TextEmbedder>>,
    fetch_concurrency: Option<usize>,
    fetch_timeout: Option<Duration>,
    max_response_bytes: Option<usize>,
}

impl NexusSearchBuilder {
    /// Creates an empty builder with standard defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures the list of search engines enabled for fanout queries.
    pub fn with_engines(mut self, engines: Vec<Engine>) -> Self {
        self.engines = Some(engines);
        self
    }

    /// Sets the adaptive fanout and early-exit quorum policy.
    pub fn with_fanout_policy(mut self, policy: FanoutPolicy) -> Self {
        self.fanout_policy = Some(policy);
        self
    }

    /// Sets the passage relevance scoring and 2-stage re-ranking policy.
    pub fn with_ranking_policy(mut self, policy: RankingPolicy) -> Self {
        self.ranking_policy = Some(policy);
        self
    }

    /// Attaches an embedding model implementation for dense and hybrid ranking.
    pub fn with_embedder(mut self, embedder: Arc<dyn TextEmbedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Sets the maximum concurrent HTTP connections for downloading candidate pages.
    pub fn with_fetch_concurrency(mut self, concurrency: usize) -> Self {
        self.fetch_concurrency = Some(concurrency.max(1));
        self
    }

    /// Sets the maximum time budget for fetching an individual candidate page.
    pub fn with_fetch_timeout(mut self, timeout: Duration) -> Self {
        self.fetch_timeout = Some(timeout);
        self
    }

    /// Sets the byte threshold above which response streams are aborted.
    pub fn with_max_response_bytes(mut self, max_bytes: usize) -> Self {
        self.max_response_bytes = Some(max_bytes);
        self
    }

    /// Validates configuration and constructs the finalized NexusSearch instance.
    pub fn build(self) -> Result<NexusSearch, NexusError> {
        if let Some(ref engines) = self.engines
            && engines.is_empty()
        {
            return Err(NexusError::InvalidConfiguration(
                "Engine list cannot be empty when explicitly configured".to_owned(),
            ));
        }

        if let Some(timeout) = self.fetch_timeout
            && timeout.is_zero()
        {
            return Err(NexusError::InvalidConfiguration(
                "fetch_timeout cannot be zero".to_owned(),
            ));
        }

        if let Some(bytes) = self.max_response_bytes
            && bytes == 0
        {
            return Err(NexusError::InvalidConfiguration(
                "max_response_bytes cannot be zero".to_owned(),
            ));
        }

        let fanout_policy = self.fanout_policy.unwrap_or_default();
        let ranking_policy = self.ranking_policy.unwrap_or_default();

        let fanout = match self.engines {
            Some(engines) => EngineFanout::new(engines, fanout_policy),
            None => EngineFanout::default(),
        };

        let fetcher = EgressFetcher::new(
            self.fetch_timeout.unwrap_or(DEFAULT_FETCH_TIMEOUT),
            self.max_response_bytes
                .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES),
        );

        let concurrency = self.fetch_concurrency.unwrap_or(DEFAULT_FETCH_CONCURRENCY);

        Ok(NexusSearch::new(
            fanout,
            fetcher,
            self.embedder,
            concurrency,
            ranking_policy,
        ))
    }
}
