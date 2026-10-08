use nexus::{Engine, NexusSearch, NexusSearchOptions, RankingMode, TimeFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize the Nexus search client with selected search providers.
    let engine = NexusSearch::builder()
        .with_engines(vec![Engine::Duckduckgo, Engine::Bing, Engine::Yahoo])
        .with_fetch_concurrency(3)
        .build()?;

    // 2. Configure query options: recency filter, BM25 sparse ranking, and passage chunk size.
    let options = NexusSearchOptions {
        time_filter: TimeFilter::Month,
        ranking_mode: RankingMode::Sparse,
        max_candidates: 3,
        chunk_size_words: 150,
        chunk_overlap_words: 30,
        fetch_timeout_ms: 4000,
        max_response_bytes: 524_288,
        fanout_deadline_ms: None,
    };

    println!("Querying web search providers for 'rust 2024 edition features'...");
    let result = engine
        .search("rust 2024 edition features", &options)
        .await?;

    println!("\nFetched {} candidate web pages.", result.raw_pages.len());
    for page in &result.raw_pages {
        println!(" - [{}] ({})", page.title, page.url);
    }

    println!("\nTop 5 Scored Passages (BM25):");
    for (i, passage) in result.scored_passages.iter().take(5).enumerate() {
        println!(
            "\n[Passage #{}] Score: {:.4} | Source: {}",
            i + 1,
            passage.score,
            passage.source_title
        );
        println!("URL: {}", passage.source_url);
        println!("Excerpt: {}", passage.text);
    }

    // Answer-presence check. `Absent` means the passages were on-topic but none
    // carried an answer to what the query actually asked. Do not report success
    // in that case: the caller would otherwise answer from its own memory.
    if let Some(nexus::AnswerPresence::Absent) = result.metrics.answer_presence {
        eprintln!(
            "\nWARNING: no delivered passage carries an answer (shape={:?}). \
             Treat as not-found rather than answering from prior knowledge.",
            result.metrics.answer_shape
        );
    }

    Ok(())
}
