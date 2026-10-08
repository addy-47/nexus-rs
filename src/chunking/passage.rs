use crate::model::ScoredPassage;

const DEFAULT_CHUNK_SIZE_WORDS: usize = 150;
const DEFAULT_CHUNK_OVERLAP_WORDS: usize = 30;

/// Splits document text into overlapping passage chunks, respecting paragraph and block boundaries.
pub fn chunk_document_passages(
    text: &str,
    source_url: &str,
    source_title: &str,
    chunk_size_words: usize,
    chunk_overlap_words: usize,
) -> Vec<ScoredPassage> {
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

    // First break text into structural blocks (paragraphs, markdown tables, lists)
    let raw_blocks: Vec<&str> = text
        .split("\n\n")
        .map(|b| b.trim())
        .filter(|b| !b.is_empty())
        .collect();

    // If text does not contain multiple paragraphs, fallback to whitespace words
    if raw_blocks.len() <= 1 {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return Vec::new();
        }

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
        return passages;
    }

    // Paragraph-aware aggregation:
    // Group paragraphs up to `size` words. Overlap takes the tail paragraph(s).
    let mut passages = Vec::new();
    let mut passage_index = 0;
    let mut current_chunk: Vec<&str> = Vec::new();
    let mut current_words = 0;

    for block in raw_blocks {
        let block_words = block.split_whitespace().count();
        if block_words == 0 {
            continue;
        }

        // If a single block exceeds size, flush current and break down block by words
        if block_words > size {
            if !current_chunk.is_empty() {
                passages.push(ScoredPassage {
                    text: current_chunk.join("\n\n"),
                    source_url: source_url.to_owned(),
                    source_title: source_title.to_owned(),
                    passage_index,
                    score: 0.0,
                    sparse_score: None,
                    dense_score: None,
                });
                passage_index += 1;
                current_chunk.clear();
                current_words = 0;
            }

            let b_words: Vec<&str> = block.split_whitespace().collect();
            let step = (size - overlap).max(1);
            let mut start = 0;
            while start < b_words.len() {
                let end = (start + size).min(b_words.len());
                passages.push(ScoredPassage {
                    text: b_words[start..end].join(" "),
                    source_url: source_url.to_owned(),
                    source_title: source_title.to_owned(),
                    passage_index,
                    score: 0.0,
                    sparse_score: None,
                    dense_score: None,
                });
                passage_index += 1;
                if end >= b_words.len() {
                    break;
                }
                start += step;
            }
            continue;
        }

        if current_words + block_words > size && !current_chunk.is_empty() {
            passages.push(ScoredPassage {
                text: current_chunk.join("\n\n"),
                source_url: source_url.to_owned(),
                source_title: source_title.to_owned(),
                passage_index,
                score: 0.0,
                sparse_score: None,
                dense_score: None,
            });
            passage_index += 1;

            // Retain tail paragraphs for overlap
            while current_words > overlap && current_chunk.len() > 1 {
                let removed = current_chunk.remove(0);
                current_words = current_words.saturating_sub(removed.split_whitespace().count());
            }
        }

        current_chunk.push(block);
        current_words += block_words;
    }

    if !current_chunk.is_empty() {
        passages.push(ScoredPassage {
            text: current_chunk.join("\n\n"),
            source_url: source_url.to_owned(),
            source_title: source_title.to_owned(),
            passage_index,
            score: 0.0,
            sparse_score: None,
            dense_score: None,
        });
    }

    passages
}
