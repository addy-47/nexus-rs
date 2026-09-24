use std::sync::Arc;

use crate::builder::NexusSearchBuilder;
use crate::chunking;
use crate::engines::EngineFanout;
use crate::error::NexusError;
use crate::extraction;
use crate::fetcher::EgressFetcher;
use crate::model::{
    NexusSearchOptions, NexusSearchResult, NexusSearchMetrics, PageFetchMetrics, RawPage,
    ScoredPassage,
};
use crate::ranking;
use crate::traits::TextEmbedder;

/// Primary orchestrator for web search, document fetching, and relevance ranking.
#[derive(Clone)]
pub struct NexusSearch {
    fanout: EngineFanout,
    fetcher: EgressFetcher,
    embedder: Option<Arc<dyn TextEmbedder>>,
    fetch_concurrency: usize,
}

impl NexusSearch {
    /// Returns a new fluent builder for configuring a NexusSearch instance.
    pub fn builder() -> NexusSearchBuilder {
        NexusSearchBuilder::new()
    }

    /// Internal constructor used by NexusSearchBuilder.
    pub(crate) fn new(
        fanout: EngineFanout,
        fetcher: EgressFetcher,
        embedder: Option<Arc<dyn TextEmbedder>>,
        fetch_concurrency: usize,
    ) -> Self {
        Self {
            fanout,
            fetcher,
            embedder,
            fetch_concurrency,
        }
    }

    /// Executes end-to-end multi-engine search, extraction, and tri-mode ranking.
    pub async fn search(
        &self,
        query: &str,
        options: &NexusSearchOptions,
    ) -> Result<NexusSearchResult, NexusError> {
        let start_time = std::time::Instant::now();
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Ok(NexusSearchResult {
                raw_pages: Vec::new(),
                scored_passages: Vec::new(),
                metrics: NexusSearchMetrics {
                    total_pipeline_ms: start_time.elapsed().as_millis() as u64,
                    ranking_mode: options.ranking_mode,
                    ..Default::default()
                },
            });
        }

        let fanout_start = std::time::Instant::now();
        let fanout_outcome = self
            .fanout
            .query_all(trimmed_query, options.time_filter)
            .await?;
        let fanout_total_ms = fanout_start.elapsed().as_millis() as u64;

        if fanout_outcome.hits.is_empty() {
            log::info!(
                "[Nexus::Pipeline] Zero provider hits returned (query_len={}, time_filter={:?})",
                trimmed_query.len(),
                options.time_filter
            );
            return Ok(NexusSearchResult {
                raw_pages: Vec::new(),
                scored_passages: Vec::new(),
                metrics: NexusSearchMetrics {
                    total_pipeline_ms: start_time.elapsed().as_millis() as u64,
                    fanout_total_ms,
                    engines: fanout_outcome.metrics,
                    total_raw_hits: fanout_outcome.total_raw_hits,
                    deduplicated_hits: 0,
                    ranking_mode: options.ranking_mode,
                    ..Default::default()
                },
            });
        }

        let deduplicated_hits = fanout_outcome.hits.len();
        let candidate_urls: Vec<String> = fanout_outcome
            .hits
            .into_iter()
            .take(options.max_candidates)
            .map(|h| h.url)
            .collect();

        let (raw_pages, pages_fetched, fetch_total_ms, extraction_total_ms) =
            self.fetch_and_extract_pages(&candidate_urls, options).await;

        let (passages, chunking_total_ms, ranking_outcome) = self
            .chunk_and_rank_passages(trimmed_query, &raw_pages, options)
            .await?;

        let total_pipeline_ms = start_time.elapsed().as_millis() as u64;
        let metrics = NexusSearchMetrics {
            total_pipeline_ms,
            fanout_total_ms,
            engines: fanout_outcome.metrics,
            total_raw_hits: fanout_outcome.total_raw_hits,
            deduplicated_hits,
            fetch_total_ms,
            pages_fetched,
            extraction_total_ms,
            chunking_total_ms,
            total_passages_generated: passages.len(),
            ranking_total_ms: ranking_outcome.total_ranking_ms,
            ranking_mode: options.ranking_mode,
            sparse_ranking_ms: ranking_outcome.sparse_ranking_ms,
            dense_ranking_ms: ranking_outcome.dense_ranking_ms,
            rrf_fusion_ms: ranking_outcome.rrf_fusion_ms,
        };

        log::info!(
            "[Nexus::Telemetry] Query='{}' total={}ms | Fanout: {}ms (raw={}, dedup={}) | Fetch: {}ms (pages={}) | Extract: {}ms | Chunk: {}ms (passages={}) | Rank: {}ms (mode={:?}, sparse={:?}, dense={:?}, rrf={:?})",
            trimmed_query,
            metrics.total_pipeline_ms,
            metrics.fanout_total_ms,
            metrics.total_raw_hits,
            metrics.deduplicated_hits,
            metrics.fetch_total_ms,
            metrics.pages_fetched.len(),
            metrics.extraction_total_ms,
            metrics.chunking_total_ms,
            metrics.total_passages_generated,
            metrics.ranking_total_ms,
            metrics.ranking_mode,
            metrics.sparse_ranking_ms,
            metrics.dense_ranking_ms,
            metrics.rrf_fusion_ms,
        );

        Ok(NexusSearchResult {
            raw_pages,
            scored_passages: passages,
            metrics,
        })
    }

    /// Fetches HTML from candidate URLs and normalizes content to Markdown with metrics.
    async fn fetch_and_extract_pages(
        &self,
        urls: &[String],
        options: &NexusSearchOptions,
    ) -> (Vec<RawPage>, Vec<PageFetchMetrics>, u64, u64) {
        let fetcher = self.fetcher.with_limits(
            std::time::Duration::from_millis(options.fetch_timeout_ms),
            options.max_response_bytes,
        );

        let fetch_start = std::time::Instant::now();
        let fetch_results = fetcher
            .fetch_all_concurrent(urls, self.fetch_concurrency)
            .await;
        let fetch_total_ms = fetch_start.elapsed().as_millis() as u64;

        let extract_start = std::time::Instant::now();
        let mut raw_pages = Vec::with_capacity(fetch_results.len());
        let mut page_metrics = Vec::with_capacity(fetch_results.len());

        for item in fetch_results {
            page_metrics.push(item.metrics);
            match item.outcome {
                Ok((final_url, html)) if !html.trim().is_empty() => {
                    let page = extraction::extract_document(
                        &final_url,
                        &html,
                        extraction::DEFAULT_MAX_PAGE_CHARS,
                    );
                    raw_pages.push(page);
                }
                Ok((final_url, _)) => {
                    log::warn!("[Nexus::Pipeline] Empty HTML body fetched from {final_url}");
                }
                Err(err) => {
                    log::warn!(
                        "[Nexus::Pipeline] Page fetch failed for {}: {err}",
                        item.requested_url
                    );
                }
            }
        }
        let extraction_total_ms = extract_start.elapsed().as_millis() as u64;

        (
            raw_pages,
            page_metrics,
            fetch_total_ms,
            extraction_total_ms,
        )
    }

    /// Chunks extracted raw pages and scores them according to the configured ranking mode.
    async fn chunk_and_rank_passages(
        &self,
        query: &str,
        raw_pages: &[RawPage],
        options: &NexusSearchOptions,
    ) -> Result<(Vec<ScoredPassage>, u64, ranking::RankingMetricsOutcome), NexusError> {
        let chunk_start = std::time::Instant::now();
        let mut passages = chunking::chunk_pages(
            raw_pages,
            options.chunk_size_words,
            options.chunk_overlap_words,
        );
        let chunking_total_ms = chunk_start.elapsed().as_millis() as u64;

        let embedder_ref = self.embedder.as_ref().map(|arc| arc.as_ref());
        let ranking_outcome =
            ranking::rank_passages(query, &mut passages, options.ranking_mode, embedder_ref).await?;

        Ok((passages, chunking_total_ms, ranking_outcome))
    }
}
