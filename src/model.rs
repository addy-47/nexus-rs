use std::fmt;

use serde::{Deserialize, Serialize};

/// Supported keyless search engine providers.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// DuckDuckGo HTML search.
    #[default]
    Duckduckgo,
    /// Bing HTML search.
    Bing,
    /// Yahoo HTML search.
    Yahoo,
    /// Mojeek HTML search.
    Mojeek,
}

/// Scoring strategy used to rank retrieved passages.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RankingMode {
    /// Lexical BM25 ranking based on term frequencies.
    #[default]
    Sparse,
    /// Dense vector cosine similarity via caller-provided embedder.
    Dense,
    /// Reciprocal Rank Fusion combining BM25 and dense vector rankings.
    Hybrid,
}

/// Recency filter for search queries across providers.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeFilter {
    /// Any time / no filter.
    #[default]
    Any,
    /// Past 24 hours.
    Day,
    /// Past week.
    Week,
    /// Past month.
    Month,
    /// Past year.
    Year,
}

/// Normalized search snippet result returned by an individual engine scraper.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngineHit {
    /// Page title extracted from SERP.
    pub title: String,
    /// Target destination URL.
    pub url: String,
    /// Display or breadcrumb URL displayed on SERP.
    pub display_url: String,
    /// Short snippet or summary provided by the search engine.
    pub snippet: String,
    /// Search engine provider that yielded this hit.
    pub engine: Engine,
}

/// Full extracted document page fetched from a candidate URL.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RawPage {
    /// Resolved target URL of the fetched page.
    pub url: String,
    /// Extracted page title or fallback title.
    pub title: String,
    /// Clean extracted Markdown text content.
    pub markdown: String,
}

/// Extracted passage segmented from a page and ranked by relevance.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScoredPassage {
    /// Passage text content.
    pub text: String,
    /// Source web URL where this passage originated.
    pub source_url: String,
    /// Title of the source web page.
    pub source_title: String,
    /// Deterministic 0-indexed position within the parent document.
    pub passage_index: usize,
    /// Final relevance score used for primary sorting (higher is better).
    pub score: f32,
    /// Sparse BM25 score, if computed.
    pub sparse_score: Option<f32>,
    /// Dense cosine similarity score, if computed.
    pub dense_score: Option<f32>,
}

/// Complete result envelope returned by a nexus search execution.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NexusSearchResult {
    /// Raw extracted documents from fetched candidate URLs.
    pub raw_pages: Vec<RawPage>,
    /// All chunked passages ranked in descending order of relevance.
    pub scored_passages: Vec<ScoredPassage>,
}

/// Configuration options controlling a search query execution.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NexusSearchOptions {
    /// Recency filter applied to provider search queries.
    pub time_filter: TimeFilter,
    /// Scoring algorithm to apply across extracted passages.
    pub ranking_mode: RankingMode,
    /// Maximum number of candidate search URLs to fetch and extract.
    pub max_candidates: usize,
    /// Target passage chunk size in words.
    pub chunk_size_words: usize,
    /// Word overlap between consecutive passage chunks.
    pub chunk_overlap_words: usize,
    /// Maximum time budget in milliseconds for fetching pages.
    pub fetch_timeout_ms: u64,
    /// Maximum response bytes downloaded per individual candidate page.
    pub max_response_bytes: usize,
}

impl fmt::Display for Engine {
    /// Formats the search engine name as lowercase string slice.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duckduckgo => write!(f, "duckduckgo"),
            Self::Bing => write!(f, "bing"),
            Self::Yahoo => write!(f, "yahoo"),
            Self::Mojeek => write!(f, "mojeek"),
        }
    }
}

impl Default for NexusSearchOptions {
    /// Constructs default search options optimized for latency and precision.
    fn default() -> Self {
        Self {
            time_filter: TimeFilter::Any,
            ranking_mode: RankingMode::Sparse,
            max_candidates: 3,
            chunk_size_words: 150,
            chunk_overlap_words: 30,
            fetch_timeout_ms: 4000,
            max_response_bytes: 524_288,
        }
    }
}

impl Engine {
    /// Returns the static lowercase string identifier of the engine.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duckduckgo => "duckduckgo",
            Self::Bing => "bing",
            Self::Yahoo => "yahoo",
            Self::Mojeek => "mojeek",
        }
    }
}

impl TimeFilter {
    /// Returns provider-specific query parameter code for date filtering.
    pub const fn code(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Day => Some("d"),
            Self::Week => Some("w"),
            Self::Month => Some("m"),
            Self::Year => Some("y"),
        }
    }
}

impl EngineHit {
    /// Creates a newly parsed search result snippet.
    pub fn new(
        title: impl Into<String>,
        url: impl Into<String>,
        display_url: impl Into<String>,
        snippet: impl Into<String>,
        engine: Engine,
    ) -> Self {
        Self {
            title: title.into().trim().to_owned(),
            url: url.into().trim().to_owned(),
            display_url: display_url.into().trim().to_owned(),
            snippet: snippet.into().trim().to_owned(),
            engine,
        }
    }
}
