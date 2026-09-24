//! ============================================================================
//! chunking_ranking_test.rs — High-impact test suite for passage chunking & tri-mode ranking
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::chunking, nexus::ranking
//! Prerequisites: None (hermetic algorithmic test)
//! Execution    : cargo test --test chunking_ranking_test
//! Metrics      : Passage window accuracy, BM25 rank order, RRF precision
//! ============================================================================

use std::time::Duration;

use async_trait::async_trait;
use nexus::chunking::chunk_pages;
use nexus::model::RankingPolicy;
use nexus::ranking::bm25::rank_bm25;
use nexus::ranking::dense::{cosine_similarity, rank_dense};
use nexus::ranking::hybrid::rank_hybrid;
use nexus::{NexusError, RawPage, ScoredPassage, TextEmbedder};

struct DeterministicMockEmbedder;

#[async_trait]
impl TextEmbedder for DeterministicMockEmbedder {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, NexusError> {
        let t = text.to_lowercase();
        if t.contains("compiler") || t.contains("rust") {
            Ok(vec![1.0, 0.0, 0.0])
        } else if t.contains("python") {
            Ok(vec![0.0, 1.0, 0.0])
        } else {
            Ok(vec![0.0, 0.0, 1.0])
        }
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NexusError> {
        let mut results = Vec::new();
        for text in texts {
            results.push(self.embed_text(text).await?);
        }
        Ok(results)
    }
}

#[tokio::test]
async fn test_passage_chunking_sliding_window() {
    tokio::time::timeout(Duration::from_secs(5), async {
        // Generate a 350-word document
        let words: Vec<String> = (0..350).map(|i| format!("word{i}")).collect();
        let body = words.join(" ");

        let page = RawPage {
            url: "https://example.com/test".to_string(),
            title: "Test Page".to_string(),
            markdown: body,
        };

        // Window: 150 words, Overlap: 30 words -> Step: 120 words
        // Chunks:
        // Chunk 0: [0..150]
        // Chunk 1: [120..270]
        // Chunk 2: [240..350] (110 words)
        let passages = chunk_pages(&[page], 150, 30);

        assert_eq!(passages.len(), 3, "Expected 3 passages for 350 words");
        assert_eq!(passages[0].passage_index, 0);
        assert_eq!(passages[1].passage_index, 1);
        assert_eq!(passages[2].passage_index, 2);

        // Verify overlap between chunk 0 and chunk 1
        assert!(passages[0].text.contains("word120"));
        assert!(passages[1].text.contains("word120"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_passage_chunking_handles_short_and_empty_documents() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let short_page = RawPage {
            url: "https://example.com/short".to_string(),
            title: "Short Page".to_string(),
            markdown: "Just a few words here.".to_string(),
        };

        let empty_page = RawPage {
            url: "https://example.com/empty".to_string(),
            title: "Empty Page".to_string(),
            markdown: "    ".to_string(),
        };

        let short_passages = chunk_pages(&[short_page], 150, 30);
        assert_eq!(short_passages.len(), 1);
        assert_eq!(short_passages[0].text, "Just a few words here.");

        let empty_passages = chunk_pages(&[empty_page], 150, 30);
        assert_eq!(empty_passages.len(), 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_bm25_ranking_ranks_relevant_passage_highest() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut passages = vec![
            ScoredPassage {
                text: "The weather today is warm and sunny with zero precipitation.".to_string(),
                source_url: "url1".to_string(),
                source_title: "Weather".to_string(),
                passage_index: 0,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
            ScoredPassage {
                text: "Rust memory safety eliminates data races and use-after-free bugs."
                    .to_string(),
                source_url: "url2".to_string(),
                source_title: "Rust Safety".to_string(),
                passage_index: 1,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
            ScoredPassage {
                text: "Cooking Italian pasta requires salted boiling water and semolina flour."
                    .to_string(),
                source_url: "url3".to_string(),
                source_title: "Cooking".to_string(),
                passage_index: 2,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
        ];

        rank_bm25("rust memory safety", &mut passages);

        assert_eq!(
            passages[0].source_title, "Rust Safety",
            "BM25 must rank the relevant passage first"
        );
        assert!(passages[0].score > passages[1].score);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_cosine_similarity_edge_cases() {
    tokio::time::timeout(Duration::from_secs(5), async {
        // Parallel identical vectors -> 1.0
        let a = [1.0, 2.0, 3.0];
        let sim_ident = cosine_similarity(&a, &a);
        assert!((sim_ident - 1.0).abs() < 1e-5);

        // Orthogonal vectors -> 0.0
        let v1 = [1.0, 0.0];
        let v2 = [0.0, 1.0];
        assert_eq!(cosine_similarity(&v1, &v2), 0.0);

        // Opposite vectors -> -1.0
        let v3 = [1.0, 0.0];
        let v4 = [-1.0, 0.0];
        assert!((cosine_similarity(&v3, &v4) - (-1.0)).abs() < 1e-5);

        // Zero-norm vector guard -> 0.0 without NaN
        let zero = [0.0, 0.0];
        assert_eq!(cosine_similarity(&v1, &zero), 0.0);

        // Length mismatch guard -> 0.0
        let mismatch = [1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&v1, &mismatch), 0.0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_hybrid_rrf_scoring_math() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let embedder = DeterministicMockEmbedder;

        let mut passages = vec![
            ScoredPassage {
                text: "Rust compiler borrow checker enforces memory safety rules.".to_string(),
                source_url: "url1".to_string(),
                source_title: "Rust".to_string(),
                passage_index: 0,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
            ScoredPassage {
                text: "Python dynamic typing and interpreter runtime execution.".to_string(),
                source_url: "url2".to_string(),
                source_title: "Python".to_string(),
                passage_index: 1,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
        ];

        rank_hybrid(
            "rust compiler",
            &mut passages,
            &embedder,
            &RankingPolicy::default(),
            None,
        )
        .await
        .unwrap();

        // For passage 0 ("Rust"):
        // Sparse Rank: 1 (matches both terms) -> 1 / (60 + 1) = 1/61
        // Dense Rank: 1 (vector [1, 0, 0] matches query [1, 0, 0]) -> 1 / (60 + 1) = 1/61
        // Expected RRF = 2/61 ≈ 0.03278688
        assert_eq!(passages[0].source_title, "Rust");
        let expected_rrf = (1.0 / 61.0) + (1.0 / 61.0);
        assert!(
            (passages[0].score - expected_rrf).abs() < 1e-5,
            "RRF score {:.6} should equal expected {:.6}",
            passages[0].score,
            expected_rrf
        );

        // Dense rank for Python: rank 2 -> 1/62 + 1/62 ≈ 0.032258
        assert!(passages[0].score > passages[1].score);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_dense_ranking_scores_and_sorts_by_cosine_similarity() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let embedder = DeterministicMockEmbedder;
        let mut passages = vec![
            ScoredPassage {
                text: "Python scripting language and interpreter.".to_string(),
                source_url: "url1".to_string(),
                source_title: "Python".to_string(),
                passage_index: 0,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
            ScoredPassage {
                text: "Rust compiler and borrow checker.".to_string(),
                source_url: "url2".to_string(),
                source_title: "Rust".to_string(),
                passage_index: 1,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
        ];

        rank_dense("rust compiler", &mut passages, &embedder)
            .await
            .unwrap();

        assert_eq!(passages[0].source_title, "Rust");
        assert!(passages[0].score > passages[1].score);
        assert_eq!(passages[0].dense_score, Some(1.0));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_hybrid_rrf_handles_duplicate_metadata_without_aliasing() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let embedder = DeterministicMockEmbedder;
        let mut passages = vec![
            ScoredPassage {
                text: "Rust memory safety compiler borrow checker.".to_string(),
                source_url: "https://example.com/same".to_string(),
                source_title: "Same Page".to_string(),
                passage_index: 0,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
            ScoredPassage {
                text: "Python dynamic runtime interpreter.".to_string(),
                source_url: "https://example.com/same".to_string(),
                source_title: "Same Page".to_string(),
                passage_index: 0,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
        ];

        rank_hybrid(
            "rust compiler",
            &mut passages,
            &embedder,
            &RankingPolicy::default(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(passages.len(), 2);
        assert!(passages[0].text.contains("Rust"));
        assert!(passages[1].text.contains("Python"));
        assert!(passages[0].score > passages[1].score);
        assert!(passages[0].score > 0.0);
        assert!(passages[1].score > 0.0);
        assert_ne!(passages[0].score, passages[1].score);
    })
    .await
    .expect("test timed out");
}

struct FlawedEmbedder {
    short_batch: bool,
    nan_vector: bool,
}

#[async_trait::async_trait]
impl TextEmbedder for FlawedEmbedder {
    async fn embed_text(&self, _text: &str) -> Result<Vec<f32>, NexusError> {
        if self.nan_vector {
            Ok(vec![f32::NAN, 0.0, 0.0])
        } else {
            Ok(vec![1.0, 0.0, 0.0])
        }
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NexusError> {
        if self.short_batch {
            Ok(vec![vec![1.0, 0.0, 0.0]]) // Returns 1 vector even if texts has 2
        } else if self.nan_vector {
            Ok(vec![vec![f32::NAN, 0.0, 0.0]; texts.len()])
        } else {
            Ok(vec![vec![1.0, 0.0, 0.0]; texts.len()])
        }
    }
}

#[tokio::test]
async fn test_dense_ranking_enforces_embedder_batch_and_value_contracts() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let short_embedder = FlawedEmbedder {
            short_batch: true,
            nan_vector: false,
        };
        let mut passages = vec![
            ScoredPassage {
                text: "Passage 1".to_string(),
                source_url: "url1".to_string(),
                source_title: "Title 1".to_string(),
                passage_index: 0,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
            ScoredPassage {
                text: "Passage 2".to_string(),
                source_url: "url2".to_string(),
                source_title: "Title 2".to_string(),
                passage_index: 1,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            },
        ];

        let result = rank_dense("query", &mut passages, &short_embedder).await;
        assert!(matches!(result, Err(NexusError::Embedding(_))));

        let nan_embedder = FlawedEmbedder {
            short_batch: false,
            nan_vector: true,
        };
        let nan_result = rank_dense("query", &mut passages, &nan_embedder).await;
        assert!(matches!(nan_result, Err(NexusError::Embedding(_))));
    })
    .await
    .expect("test timed out");
}

#[test]
fn test_builder_configuration_validation() {
    let empty_engines_builder = nexus::NexusSearchBuilder::new().with_engines(Vec::new());
    assert!(matches!(
        empty_engines_builder.build(),
        Err(NexusError::InvalidConfiguration(_))
    ));

    let zero_timeout_builder =
        nexus::NexusSearchBuilder::new().with_fetch_timeout(Duration::from_millis(0));
    assert!(matches!(
        zero_timeout_builder.build(),
        Err(NexusError::InvalidConfiguration(_))
    ));

    let zero_bytes_builder = nexus::NexusSearchBuilder::new().with_max_response_bytes(0);
    assert!(matches!(
        zero_bytes_builder.build(),
        Err(NexusError::InvalidConfiguration(_))
    ));
}
