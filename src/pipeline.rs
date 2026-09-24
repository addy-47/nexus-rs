use std::sync::Arc;

use crate::builder::NexusSearchBuilder;
use crate::chunking;
use crate::engines::EngineFanout;
use crate::error::NexusError;
use crate::extraction;
use crate::fetcher::EgressFetcher;
use crate::model::{NexusSearchOptions, NexusSearchResult, RawPage, ScoredPassage};
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
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Ok(NexusSearchResult {
                raw_pages: Vec::new(),
                scored_passages: Vec::new(),
            });
        }

        let hits = self.fanout.query_all(trimmed_query, options.time_filter).await;
        if hits.is_empty() {
            log::info!("[Nexus::Pipeline] Zero hits returned for query: '{trimmed_query}'");
            return Ok(NexusSearchResult {
                raw_pages: Vec::new(),
                scored_passages: Vec::new(),
            });
        }

        let candidate_urls: Vec<String> = hits
            .into_iter()
            .take(options.max_candidates)
            .map(|h| h.url)
            .collect();

        let raw_pages = self.fetch_and_extract_pages(&candidate_urls).await;
        let passages = self.chunk_and_rank_passages(trimmed_query, &raw_pages, options).await?;

        Ok(NexusSearchResult {
            raw_pages,
            scored_passages: passages,
        })
    }

    /// Fetches HTML from candidate URLs and normalizes content to Markdown.
    async fn fetch_and_extract_pages(&self, urls: &[String]) -> Vec<RawPage> {
        let fetch_results = self
            .fetcher
            .fetch_all_concurrent(urls, self.fetch_concurrency)
            .await;

        let mut raw_pages = Vec::with_capacity(fetch_results.len());
        for (url, res) in fetch_results {
            match res {
                Ok(html) if !html.trim().is_empty() => {
                    let page = extraction::extract_document(
                        &url,
                        &html,
                        extraction::DEFAULT_MAX_PAGE_CHARS,
                    );
                    raw_pages.push(page);
                }
                Ok(_) => {
                    log::warn!("[Nexus::Pipeline] Empty HTML body fetched from {url}");
                }
                Err(err) => {
                    log::warn!("[Nexus::Pipeline] Page fetch failed for {url}: {err}");
                }
            }
        }
        raw_pages
    }

    /// Chunks extracted raw pages and scores them according to the configured ranking mode.
    async fn chunk_and_rank_passages(
        &self,
        query: &str,
        raw_pages: &[RawPage],
        options: &NexusSearchOptions,
    ) -> Result<Vec<ScoredPassage>, NexusError> {
        let mut passages = chunking::chunk_pages(
            raw_pages,
            options.chunk_size_words,
            options.chunk_overlap_words,
        );

        let embedder_ref = self.embedder.as_ref().map(|arc| arc.as_ref());
        ranking::rank_passages(query, &mut passages, options.ranking_mode, embedder_ref).await?;

        Ok(passages)
    }
}
