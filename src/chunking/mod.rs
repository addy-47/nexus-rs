use crate::model::{RawPage, ScoredPassage};

pub mod passage;

/// Chunks all raw pages into passage chunks for downstream ranking.
pub fn chunk_pages(
    pages: &[RawPage],
    chunk_size_words: usize,
    chunk_overlap_words: usize,
) -> Vec<ScoredPassage> {
    let mut all_passages = Vec::new();
    for page in pages {
        let mut chunks = passage::chunk_document_passages(
            &page.markdown,
            &page.url,
            &page.title,
            chunk_size_words,
            chunk_overlap_words,
        );
        all_passages.append(&mut chunks);
    }
    all_passages
}
