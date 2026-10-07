use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::builder::NexusSearchBuilder;
use crate::chunking;
use crate::engines::EngineFanout;
use crate::error::NexusError;
use crate::extraction;
use crate::fetcher::EgressFetcher;
use crate::model::{
    NexusSearchMetrics, NexusSearchOptions, NexusSearchResult, PageFetchMetrics, RankingPolicy,
    RawPage, ScoredPassage,
};
use crate::ranking;
use crate::traits::TextEmbedder;

/// Cached search result envelope with intent-aware TTL.
#[derive(Clone, Debug)]
pub struct CachedSearchResult {
    pub result: NexusSearchResult,
    pub expires_at: Instant,
}

pub type InFlightMap = Arc<
    tokio::sync::Mutex<
        HashMap<String, tokio::sync::broadcast::Sender<Result<NexusSearchResult, String>>>,
    >,
>;

pub struct InFlightGuard {
    in_flight: InFlightMap,
    cache_key: Option<String>,
}

impl InFlightGuard {
    pub fn new(in_flight: InFlightMap, cache_key: String) -> Self {
        Self {
            in_flight,
            cache_key: Some(cache_key),
        }
    }

    pub fn disarm(&mut self) {
        self.cache_key = None;
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Some(key) = self.cache_key.take() {
            let in_flight = Arc::clone(&self.in_flight);
            tokio::spawn(async move {
                let mut guard = in_flight.lock().await;
                guard.remove(&key);
            });
        }
    }
}

/// Primary orchestrator for web search, document fetching, and relevance ranking.
#[derive(Clone)]
pub struct NexusSearch {
    fanout: EngineFanout,
    fetcher: EgressFetcher,
    embedder: Option<Arc<dyn TextEmbedder>>,
    fetch_concurrency: usize,
    ranking_policy: RankingPolicy,
    query_cache: Arc<Mutex<HashMap<String, CachedSearchResult>>>,
    in_flight: InFlightMap,
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
        ranking_policy: RankingPolicy,
    ) -> Self {
        Self {
            fanout,
            fetcher,
            embedder,
            fetch_concurrency,
            ranking_policy,
            query_cache: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    fn cache_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedSearchResult>> {
        self.query_cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[doc(hidden)]
    pub async fn in_flight_len(&self) -> usize {
        self.in_flight.lock().await.len()
    }

    #[doc(hidden)]
    pub fn in_flight_map(&self) -> InFlightMap {
        Arc::clone(&self.in_flight)
    }

    #[doc(hidden)]
    pub fn poison_cache_for_test(&self) {
        let cache = Arc::clone(&self.query_cache);
        let _ = std::thread::spawn(move || {
            let _guard = cache.lock().unwrap();
            panic!("Intentional test panic to poison query_cache");
        })
        .join();
    }

    /// Fetches a single page by URL, applying SSRF validation, redirects, and document extraction.
    pub async fn fetch_page(&self, url: &str) -> Result<RawPage, NexusError> {
        let fetched = self.fetcher.fetch_page(url).await;
        match fetched.outcome {
            Ok((final_url, html)) => {
                let doc = extraction::extract_document(
                    &final_url,
                    &html,
                    extraction::DEFAULT_MAX_PAGE_CHARS,
                );
                Ok(doc)
            }
            Err(e) => Err(e),
        }
    }

    /// Extracts focused passages from a raw page using chunking and lexical ranking.
    pub fn extract_focused_passages(
        &self,
        page: &RawPage,
        focus: Option<&str>,
        max_passages: usize,
    ) -> Vec<ScoredPassage> {
        let mut passages = chunking::passage::chunk_document_passages(
            &page.markdown,
            &page.url,
            &page.title,
            150,
            30,
        );
        if let Some(focus_query) = focus {
            let focus_trimmed = focus_query.trim();
            if !focus_trimmed.is_empty() {
                ranking::bm25::rank_bm25(focus_trimmed, &mut passages);
            }
        }
        passages.into_iter().take(max_passages).collect()
    }

    /// Executes end-to-end multi-engine search, extraction, and tri-mode ranking with caching and coalescing.
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
                metrics: NexusSearchMetrics {
                    ranking_mode: options.ranking_mode,
                    ..Default::default()
                },
            });
        }

        let cache_key = format!(
            "{}:{}:{:?}:{:?}:{}:{}:{}:{:?}",
            trimmed_query.to_lowercase(),
            options.max_candidates,
            options.time_filter,
            options.ranking_mode,
            options.chunk_size_words,
            options.chunk_overlap_words,
            options.max_response_bytes,
            options.focus.as_deref().unwrap_or(""),
        );

        // 1. Check query cache
        {
            let mut guard = self.cache_lock();
            if let Some(entry) = guard.get(&cache_key) {
                if Instant::now() < entry.expires_at {
                    log::debug!("[Nexus::Cache] Cache hit for '{trimmed_query}' (0ms network)");
                    return Ok(entry.result.clone());
                } else {
                    guard.remove(&cache_key);
                }
            }
        }

        // 2. Single-flight request coalescing
        let (is_leader, subscriber) = {
            let mut in_flight = self.in_flight.lock().await;
            if let Some(tx) = in_flight.get(&cache_key) {
                (false, Some(tx.subscribe()))
            } else {
                let (tx, _rx) = tokio::sync::broadcast::channel(4);
                in_flight.insert(cache_key.clone(), tx);
                (true, None)
            }
        };

        if !is_leader
            && let Some(mut rx) = subscriber
        {
            log::debug!("[Nexus::Coalesce] Joined in-flight search wave for '{trimmed_query}'");
            let wait_timeout = Duration::from_millis(options.fetch_timeout_ms.clamp(50, 5000));
            match tokio::time::timeout(wait_timeout, rx.recv()).await {
                Ok(Ok(Ok(cached))) => return Ok(cached),
                Ok(Ok(Err(err))) => return Err(NexusError::ScraperTransport(err)),
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                    log::warn!("[Nexus::Coalesce] Broadcast channel lagged ({n}) for '{trimmed_query}', falling back to uncached search");
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                    log::warn!("[Nexus::Coalesce] Broadcast channel closed for '{trimmed_query}', falling back to uncached search");
                }
                Err(_timeout) => {
                    log::warn!("[Nexus::Coalesce] Wait exceeded {:?} for '{trimmed_query}', falling back to uncached search", wait_timeout);
                }
            }
        }

        let mut guard = if is_leader {
            Some(InFlightGuard::new(
                Arc::clone(&self.in_flight),
                cache_key.clone(),
            ))
        } else {
            None
        };

        let search_res = self.search_uncached(trimmed_query, options).await;

        if is_leader {
            if let Some(ref mut g) = guard {
                g.disarm();
            }
            let mut in_flight = self.in_flight.lock().await;
            if let Some(tx) = in_flight.remove(&cache_key) {
                match &search_res {
                    Ok(res) => {
                        let _ = tx.send(Ok(res.clone()));
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err.to_string()));
                    }
                }
            }
        }

        let result = search_res?;

        // 3. Store in query cache with intent-aware TTL
        let lower_query = trimmed_query.to_lowercase();
        let query_words: HashSet<&str> = lower_query.split_whitespace().collect();

        let is_news = options.time_filter == crate::model::TimeFilter::Day
            || options.time_filter == crate::model::TimeFilter::Week
            || query_words.contains("today")
            || query_words.contains("latest")
            || query_words.contains("breaking")
            || query_words.contains("news");

        let is_docs = query_words.contains("documentation")
            || query_words.contains("docs")
            || query_words.contains("api")
            || query_words.contains("rust")
            || query_words.contains("python")
            || query_words.contains("guide");

        let ttl = if is_news {
            Duration::from_secs(15 * 60)
        } else if is_docs {
            Duration::from_secs(4 * 3600)
        } else {
            Duration::from_secs(3600)
        };

        {
            let mut guard = self.cache_lock();
            if guard.len() >= 50 {
                let now = Instant::now();
                guard.retain(|_, v| now < v.expires_at);
                if guard.len() >= 50
                    && let Some((oldest_key, _)) = guard
                        .iter()
                        .min_by_key(|(_, v)| v.expires_at)
                        .map(|(k, v)| (k.clone(), v.clone()))
                {
                    guard.remove(&oldest_key);
                }
            }
            guard.insert(
                cache_key,
                CachedSearchResult {
                    result: result.clone(),
                    expires_at: Instant::now() + ttl,
                },
            );
        }

        Ok(result)
    }

    /// Internal uncached search execution pipeline.
    async fn search_uncached(
        &self,
        trimmed_query: &str,
        options: &NexusSearchOptions,
    ) -> Result<NexusSearchResult, NexusError> {
        let start_time = Instant::now();
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

        let dedup_start = std::time::Instant::now();
        let deduplicated_hits = fanout_outcome.hits.len();
        // P0-5: entity anchors + domain intent reorder candidates so fetch slots
        // go to sources that can satisfy the query. Never starves: unmatched
        // hits follow in original order.
        let fetch_candidate_count = (options.max_candidates + 2).clamp(options.max_candidates, 5);
        let candidate_urls: Vec<String> = extraction::quality::order_candidates(
            trimmed_query,
            fanout_outcome.hits,
            fetch_candidate_count,
        );
        let url_dedup_ms = dedup_start.elapsed().as_millis() as u64;

        let (raw_pages, pages_fetched, pages_extracted, fetch_total_ms, extraction_total_ms) = self
            .fetch_and_extract_pages(&candidate_urls, trimmed_query, options)
            .await;

        let (passages, chunking_total_ms, ranking_outcome) = self
            .chunk_and_rank_passages(
                trimmed_query,
                &raw_pages,
                options,
                Some(&fanout_outcome.consensus_urls),
            )
            .await?;

        let total_pipeline_ms = start_time.elapsed().as_millis() as u64;
        let metrics = NexusSearchMetrics {
            total_pipeline_ms,
            fanout_total_ms,
            engines: fanout_outcome.metrics,
            total_raw_hits: fanout_outcome.total_raw_hits,
            url_dedup_ms,
            deduplicated_hits,
            fetch_total_ms,
            pages_fetched,
            extraction_total_ms,
            pages_extracted,
            chunking_total_ms,
            total_passages_generated: passages.len(),
            ranking_total_ms: ranking_outcome.total_ranking_ms,
            ranking_mode: options.ranking_mode,
            sparse_ranking_ms: ranking_outcome.sparse_ranking_ms,
            dense_ranking_ms: ranking_outcome.dense_ranking_ms,
            rrf_fusion_ms: ranking_outcome.rrf_fusion_ms,
        };

        log::info!(
            "[Nexus::Telemetry] Query='{}' total={}ms | Fanout: {}ms (raw={}, dedup={} in {}ms) | Fetch: {}ms (pages={}) | Extract: {}ms | Chunk: {}ms (passages={}) | Rank: {}ms (mode={:?}, sparse={:?}, dense={:?}, rrf={:?})",
            trimmed_query,
            metrics.total_pipeline_ms,
            metrics.fanout_total_ms,
            metrics.total_raw_hits,
            metrics.deduplicated_hits,
            metrics.url_dedup_ms,
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
        query: &str,
        options: &NexusSearchOptions,
    ) -> (
        Vec<RawPage>,
        Vec<PageFetchMetrics>,
        Vec<crate::model::PageExtractMetrics>,
        u64,
        u64,
    ) {
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
        let mut extract_metrics = Vec::with_capacity(fetch_results.len());
        // P0-5 language fallbacks: pages skipped for language mismatch are kept
        // aside and the first is re-admitted if nothing else survived, so this
        // filter can never starve the corpus on its own.
        let mut language_skipped: Vec<RawPage> = Vec::new();

        for item in fetch_results {
            page_metrics.push(item.metrics);
            match item.outcome {
                Ok((final_url, html)) if !html.trim().is_empty() => {
                    let page_extract_start = std::time::Instant::now();
                    let page = extraction::extract_document(
                        &final_url,
                        &html,
                        extraction::DEFAULT_MAX_PAGE_CHARS,
                    );
                    let page_extract_ms = page_extract_start.elapsed().as_millis() as u64;
                    // P0-4: bot-challenge / block / error shells are never evidence,
                    // regardless of rank. (G3 `ent_03`: block page scored 0.033.)
                    if extraction::quality::is_challenge_or_error_page(&page.title, &page.markdown)
                    {
                        log::warn!(
                            "[Nexus::Pipeline] Rejecting challenge/error page as source: {} (title: {})",
                            final_url,
                            page.title
                        );
                        continue;
                    }
                    // P0-3: code-dominated pages are never evidence. (G3 `cmp_03`:
                    // 22,540 chars of Closure JS; `ent_01`: 41,930 chars of Adobe
                    // Target JS — both delivered as passages.)
                    if extraction::quality::is_code_like(&page.markdown) {
                        log::warn!(
                            "[Nexus::Pipeline] Rejecting code-dominated page as source: {} ({} bytes)",
                            final_url,
                            page.markdown.len()
                        );
                        continue;
                    }
                    // P0-5 language: an explicit non-Latin `hl=` marker for an
                    // ASCII query is a wrong-language signal (G3 `cmp_03`: `?hl=ru`
                    // served Russian). Parked aside, re-admitted only if nothing
                    // else survived (see below).
                    if extraction::quality::is_language_mismatched(&final_url, query) {
                        log::warn!(
                            "[Nexus::Pipeline] Parking language-mismatched page as source: {}",
                            final_url
                        );
                        language_skipped.push(page);
                        continue;
                    }
                    extract_metrics.push(crate::model::PageExtractMetrics {
                        url: final_url.clone(),
                        extraction_ms: page_extract_ms,
                        markdown_bytes: page.markdown.len(),
                    });
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
        // Non-starvation: if every surviving page was parked for language
        // mismatch, re-admit the first rather than returning an empty corpus.
        if raw_pages.is_empty()
            && let Some(page) = language_skipped.into_iter().next()
        {
            log::warn!(
                "[Nexus::Pipeline] Re-admitting language-mismatched page (no alternatives): {}",
                page.url
            );
            raw_pages.push(page);
        }
        // Truncate to caller-configured max_candidates after filtering out unusable pages
        raw_pages.truncate(options.max_candidates);
        let extraction_total_ms = extract_start.elapsed().as_millis() as u64;

        (
            raw_pages,
            page_metrics,
            extract_metrics,
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
        consensus_urls: Option<&std::collections::HashSet<String>>,
    ) -> Result<(Vec<ScoredPassage>, u64, ranking::RankingMetricsOutcome), NexusError> {
        let chunk_start = std::time::Instant::now();
        let mut passages = chunking::chunk_pages(
            raw_pages,
            options.chunk_size_words,
            options.chunk_overlap_words,
        );
        let chunking_total_ms = chunk_start.elapsed().as_millis() as u64;

        if let Some(focus) = options.focus.as_deref().filter(|f| !f.trim().is_empty()) {
            ranking::bm25::rank_bm25(focus.trim(), &mut passages);
        }

        let embedder_ref = self.embedder.as_ref().map(|arc| arc.as_ref());
        let ranking_outcome = ranking::rank_passages(
            query,
            &mut passages,
            options.ranking_mode,
            embedder_ref,
            &self.ranking_policy,
            consensus_urls,
        )
        .await?;

        Ok((passages, chunking_total_ms, ranking_outcome))
    }
}
