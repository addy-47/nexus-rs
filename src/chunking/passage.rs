use crate::model::ScoredPassage;

const DEFAULT_CHUNK_SIZE_WORDS: usize = 150;
const DEFAULT_CHUNK_OVERLAP_WORDS: usize = 30;

/// Splits document text into overlapping word-bounded passage chunks.
pub fn chunk_document_passages(
    text: &str,
    source_url: &str,
    source_title: &str,
    chunk_size_words: usize,
    chunk_overlap_words: usize,
) -> Vec<ScoredPassage> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let size = if chunk_size_words == 0 {
        DEFAULT_CHUNK_SIZE_WORDS
    } else {
        chunk_size_words
    };

    let overlap = if chunk_overlap_words >= size {
        DEFAULT_CHUNK_OVERLAP_WORDS.min(size.saturating_sub(1))
    } else {
        chunk_overlap_words
    };

    let step = (size - overlap).max(1);
    let mut passages = Vec::new();
    let mut passage_index = 0;
    let mut start = 0;

    while start < words.len() {
        let end = (start + size).min(words.len());
        let chunk_words = &words[start..end];
        let chunk_text = chunk_words.join(" ");

        passages.push(ScoredPassage {
            text: chunk_text,
            source_url: source_url.to_owned(),
            source_title: source_title.to_owned(),
            passage_index,
            score: 0.0,
            sparse_score: None,
            dense_score: None,
        });

        passage_index += 1;
        if end >= words.len() {
            break;
        }
        start += step;
    }

    passages
}
