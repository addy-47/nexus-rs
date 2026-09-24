use crate::model::ScoredPassage;

const BM25_K1: f32 = 1.5;
const BM25_B: f32 = 0.75;

#[derive(Default)]
struct SimpleTokenizer;

impl bm25::Tokenizer for SimpleTokenizer {
    /// Tokenizes input string into lowercase whitespace tokens.
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_lowercase).collect()
    }
}

struct RawTokenEmbedder;

impl bm25::TokenEmbedder for RawTokenEmbedder {
    type EmbeddingSpace = String;

    /// Embeds a token as its own string representation.
    fn embed(token: &str) -> String {
        token.to_owned()
    }
}

/// Computes raw BM25 scores for passages in their input order.
pub fn score_bm25(query: &str, passages: &[ScoredPassage]) -> Vec<f32> {
    if passages.is_empty() {
        return Vec::new();
    }

    let documents: Vec<String> = passages.iter().map(|p| p.text.to_lowercase()).collect();
    let doc_refs: Vec<&str> = documents.iter().map(String::as_str).collect();

    let embedder = bm25::EmbedderBuilder::<RawTokenEmbedder, SimpleTokenizer>::with_tokenizer_and_fit_to_corpus(
        SimpleTokenizer,
        &doc_refs,
    )
    .k1(BM25_K1)
    .b(BM25_B)
    .build();

    let mut scorer = bm25::Scorer::<usize, String>::new();
    for (idx, doc) in documents.iter().enumerate() {
        scorer.upsert(&idx, embedder.embed(doc));
    }

    let query_embedding = embedder.embed(&query.to_lowercase());

    (0..passages.len())
        .map(|idx| scorer.score(&idx, &query_embedding).unwrap_or(0.0))
        .collect()
}

/// Scores and ranks passage chunks using lexical BM25.
pub fn rank_bm25(query: &str, passages: &mut [ScoredPassage]) {
    if passages.is_empty() {
        return;
    }

    let scores = score_bm25(query, passages);
    for (idx, passage) in passages.iter_mut().enumerate() {
        let score = scores[idx];
        passage.sparse_score = Some(score);
        passage.score = score;
    }

    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
}
