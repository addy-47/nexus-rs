//! Answer-presence verification.
//!
//! Mechanism B failure: the correct page is fetched and ranked, but the specific
//! answer sentence never survives chunk selection, so the delivered evidence is
//! topical background with no answer. The caller then reports success and the
//! model fills the gap from parametric memory.
//!
//! This module decides whether a delivered passage set plausibly *contains* an
//! answer to the query, so the pipeline can say so honestly instead of implying
//! retrieval succeeded.

use serde::{Deserialize, Serialize};

use crate::extraction::quality::entity_anchors;

/// What kind of answer the query appears to demand.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerShape {
    /// The query does not ask for a specific value (discussion, explanation, how-to).
    NotValueSeeking,
    /// The query asks for a count, quantity, measurement, or version number.
    Numeric,
    /// The query asks for a calendar year or date.
    Year,
    /// The query asks for a named entity (person, place, organization, title).
    Entity,
}

/// Query tokens that merely frame a question and must never be treated as the subject.
const FRAMING: &[&str] = &[
    "how", "what", "when", "where", "who", "which", "why", "is", "are", "was", "were", "do",
    "does", "did", "the", "a", "an", "of", "in", "on", "at", "to", "for", "from", "by", "with",
    "about", "and", "or", "that", "this", "these", "those", "it", "its", "there", "their", "his",
    "her", "be", "been", "being", "has", "have", "had",
];

/// Tokens that signal a question expects a concrete value back.
const NUMERIC_CUES: &[&str] = &[
    "how many",
    "how much",
    "how long",
    "how far",
    "how fast",
    "how old",
    "how tall",
    "how deep",
    "how wide",
    "how heavy",
    "population",
    "temperature",
    "speed",
    "distance",
    "weight",
    "height",
    "length",
    "area",
    "volume",
    "capacity",
    "salary",
    "cost",
    "price",
    "percentage",
    "ratio",
    "average",
    "total",
    "count",
    "size",
    "radius",
    "diameter",
    "frequency",
    "wavelength",
    "voltage",
    "current",
    "power",
    "pressure",
    "altitude",
    "latitude",
    "longitude",
    "duration",
    "age",
    "capacity",
    "throughput",
    // "<quantity> number of <subject>" asks for a count even with no wh-word,
    // e.g. "atomic number of gold", "octal number of 42".
    " number of ",
];

/// Classifies the answer shape a query is expected to return.
pub fn classify_answer_shape(query: &str) -> AnswerShape {
    let q = query.to_ascii_lowercase();
    let trimmed = q.trim();

    // A bare question with no subject token is not answerable by retrieval shape.
    let subject_tokens = trimmed
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty() && t.len() >= 3 && !FRAMING.contains(t))
        .count();
    if subject_tokens == 0 {
        return AnswerShape::NotValueSeeking;
    }

    let is_question = trimmed.contains('?')
        || FRAMING
            .iter()
            .any(|w| trimmed.split_whitespace().next() == Some(*w));

    // Numeric cues win first: "how many players" is numeric even though "players" is a noun.
    if NUMERIC_CUES.iter().any(|c| trimmed.contains(c)) {
        return AnswerShape::Numeric;
    }
    if trimmed.contains("what year")
        || trimmed.contains("which year")
        || trimmed.contains("in what year")
        || trimmed.contains("when did")
        || trimmed.contains("when was")
        || trimmed.contains("what date")
    {
        return AnswerShape::Year;
    }
    if is_question && entity_anchors(query).is_empty() {
        // A question with no proper noun, acronym, or identifier: the answer is
        // most likely a common noun phrase rather than a citable entity.
        return AnswerShape::NotValueSeeking;
    }
    if is_question {
        return AnswerShape::Entity;
    }
    AnswerShape::NotValueSeeking
}

/// Extracts the set of numeric tokens from text, normalized to bare digits so
/// `1,280`, `1280`, and `1280 fl oz` all collapse to `1280`.
fn numeric_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            current.push(c);
        } else if (c == ',' || c == '.' || c == ':' || c == '/')
            && chars.peek().is_some_and(|n| n.is_ascii_digit())
            && !current.is_empty()
        {
            // Separator inside a number (1,280 / 3.14 / 12:30) - keep, digits follow.
            current.push(c);
        } else {
            if !current.is_empty() {
                let normalized: String = current.chars().filter(|c| c.is_ascii_digit()).collect();
                if !normalized.is_empty() && !out.contains(&normalized) {
                    out.push(normalized);
                }
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        let normalized: String = current.chars().filter(|c| c.is_ascii_digit()).collect();
        if !normalized.is_empty() && !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

/// True when `text` contains a numeric token that is not already present in the query.
///
/// This is the core signal: a passage that only restates the query's own numbers
/// (page numbers, "2" in "player 2") does not constitute an answer.
fn has_novel_number(text: &str, query_numbers: &[String]) -> bool {
    numeric_tokens(text)
        .into_iter()
        .any(|n| !query_numbers.contains(&n))
}

/// True when `text` contains a capitalized proper noun absent from the query.
fn has_novel_proper_noun(text: &str, query_lower: &str) -> bool {
    for (idx, c) in text.char_indices() {
        if !c.is_uppercase() || !c.is_alphabetic() {
            continue;
        }
        // Must be at a word boundary.
        let prev_is_boundary = text[..idx]
            .chars()
            .next_back()
            .is_none_or(|p| !p.is_alphanumeric());
        if !prev_is_boundary {
            continue;
        }
        let rest = &text[idx..];
        let word: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
        if word.len() < 3 {
            continue;
        }
        // Skip sentence-initial ordinary words; a real proper noun is not a
        // stopword and does not appear verbatim in the query.
        let lower = word.to_ascii_lowercase();
        if FRAMING.contains(&lower.as_str()) {
            continue;
        }
        if !query_lower.contains(&lower) {
            return true;
        }
    }
    false
}

/// Outcome of answer-presence verification over a delivered passage set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerPresence {
    /// The query does not demand a specific value; no verification applies.
    NotApplicable,
    /// At least one delivered passage plausibly carries the answer.
    Present,
    /// Passages were delivered, but none plausibly carry the answer.
    Absent,
}

/// Verifies whether `passages` plausibly contain an answer to `query`.
///
/// Deliberately conservative: it only reports `Absent` when the query clearly
/// demands a value and *no* passage shows any candidate answer. A false
/// `Present` costs a wasted turn; a false `Absent` costs a correct answer.
pub fn verify_answer_presence<F>(query: &str, shape: AnswerShape, passages: &[F]) -> AnswerPresence
where
    F: AsRef<str>,
{
    match shape {
        AnswerShape::NotValueSeeking => AnswerPresence::NotApplicable,
        AnswerShape::Numeric | AnswerShape::Year => {
            let query_numbers = numeric_tokens(query);
            let found = passages
                .iter()
                .any(|p| has_novel_number(p.as_ref(), &query_numbers));
            if found {
                AnswerPresence::Present
            } else {
                AnswerPresence::Absent
            }
        }
        AnswerShape::Entity => {
            let query_lower = query.to_ascii_lowercase();
            let found = passages
                .iter()
                .any(|p| has_novel_proper_noun(p.as_ref(), &query_lower));
            if found {
                AnswerPresence::Present
            } else {
                AnswerPresence::Absent
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn classifies_numeric_questions() {
        assert_eq!(
            classify_answer_shape(
                "how many players from each team are on the field in association football"
            ),
            AnswerShape::Numeric
        );
        assert_eq!(
            classify_answer_shape("temperature at which water boils at sea level in Fahrenheit"),
            AnswerShape::Numeric
        );
        assert_eq!(
            classify_answer_shape("atomic number of gold"),
            AnswerShape::Numeric
        );
    }

    #[test]
    fn classifies_year_questions() {
        assert_eq!(
            classify_answer_shape("what year did the Berlin Wall fall"),
            AnswerShape::Year
        );
        assert_eq!(
            classify_answer_shape("when did the Titanic sink"),
            AnswerShape::Year
        );
    }

    #[test]
    fn classifies_entity_questions() {
        assert_eq!(
            classify_answer_shape("what is the capital of Australia"),
            AnswerShape::Entity
        );
    }

    #[test]
    fn ignores_non_value_seeking_queries() {
        assert_eq!(
            classify_answer_shape("how does a diesel engine work"),
            AnswerShape::NotValueSeeking
        );
        assert_eq!(
            classify_answer_shape("best practices for code review"),
            AnswerShape::NotValueSeeking
        );
        assert_eq!(
            classify_answer_shape("what is"),
            AnswerShape::NotValueSeeking
        );
    }

    #[test]
    fn detects_answer_presence_for_numeric_query() {
        let p = texts(&["A US gallon equals 128 fluid ounces."]);
        assert_eq!(
            verify_answer_presence(
                "how many fluid ounces in one US gallon",
                AnswerShape::Numeric,
                &p
            ),
            AnswerPresence::Present
        );
    }

    #[test]
    fn reports_absence_when_only_query_numbers_reappear() {
        // Passages mention "2" but that is the only number and it came from the query.
        let p = texts(&["A team fields 2 players in this variant."]);
        let q = "how many players are on the field for team 2";
        assert_eq!(
            verify_answer_presence(q, AnswerShape::Numeric, &p),
            AnswerPresence::Absent
        );
    }

    #[test]
    fn reports_absence_for_topical_background_only() {
        let p = texts(&[
            "Water boils when its vapour pressure equals atmospheric pressure.",
            "Elevation and depth are measured from sea level.",
        ]);
        assert_eq!(
            verify_answer_presence(
                "temperature at which water boils at sea level in Fahrenheit",
                AnswerShape::Numeric,
                &p
            ),
            AnswerPresence::Absent
        );
    }

    #[test]
    fn numeric_tokens_normalizes_separators() {
        assert_eq!(numeric_tokens("1,280 dollars"), vec!["1280".to_string()]);
        assert_eq!(numeric_tokens("3.14"), vec!["314".to_string()]);
        assert_eq!(numeric_tokens("no digits here"), Vec::<String>::new());
    }

    #[test]
    fn detects_novel_proper_noun_for_entity_query() {
        let p = texts(&["The capital city is Canberra, in the ACT."]);
        assert_eq!(
            verify_answer_presence("what is the capital of Australia", AnswerShape::Entity, &p),
            AnswerPresence::Present
        );
    }

    #[test]
    fn reports_entity_absence_when_only_framing_words_reappear() {
        let p = texts(&["Australia is a country. It has several cities."]);
        assert_eq!(
            verify_answer_presence("what is the capital of Australia", AnswerShape::Entity, &p),
            AnswerPresence::Absent
        );
    }

    #[test]
    fn not_applicable_for_non_value_seeking() {
        let p = texts(&["Nothing relevant here at all."]);
        assert_eq!(
            verify_answer_presence(
                "how does a diesel engine work",
                AnswerShape::NotValueSeeking,
                &p
            ),
            AnswerPresence::NotApplicable
        );
    }
}
