pub mod builder;
pub mod chunking;
pub mod engines;
pub mod error;
pub mod extraction;
pub mod fetcher;
pub mod model;
pub mod pipeline;
pub mod ranking;
pub mod traits;

pub use builder::NexusSearchBuilder;
pub use error::NexusError;
pub use fetcher::EgressFetcher;
pub use model::{
    Engine, EngineHit, NexusSearchMetrics, NexusSearchOptions, NexusSearchResult, RankingMode,
    RawPage, ScoredPassage, TimeFilter,
};
pub use pipeline::NexusSearch;
pub use traits::TextEmbedder;
