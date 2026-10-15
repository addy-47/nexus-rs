pub mod answer_presence;
pub mod builder;

/// Crate version for run manifests and diagnostics. Reads the package version
/// at compile time so eval reports can pin the exact retrieval stack.
pub const NEXUS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Answer-presence verdicts and query shapes, re-exported at the crate root so
/// callers can match on them without reaching into `model`.
pub use answer_presence::{AnswerPresence, AnswerShape, verify_answer_presence};
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
    Engine, EngineHit, FailureClass, NexusSearchMetrics, NexusSearchOptions, NexusSearchResult,
    RankingMode, RawPage, ScoredPassage, TimeFilter,
};
pub use pipeline::NexusSearch;
pub use traits::TextEmbedder;
