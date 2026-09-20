//! HotpotQA answer-only v1 metrics, following the authors' evaluator:
//! https://github.com/hotpotqa/hotpot/blob/master/hotpot_evaluate_v1.py
//! Score the complete returned answer. Evidence/supporting-fact scores are separate tasks.
use std::{collections::HashMap, sync::LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

pub const HOTPOTQA_EVALUATION: &str = "hotpotqa_answer_v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct AnswerScores {
    pub exact_match: f64,
    pub f1: f64,
}

fn normalize(answer: &str) -> String {
    // Python's Unicode \w is letters/numbers/underscore, rather than Rust's broader
    // Alphabetic property. ASCII punctuation is removed before article boundaries.
    static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\pL\pN_]+").unwrap());
    let lower: String = answer
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_ascii_punctuation())
        .collect();
    let without_articles = WORDS.replace_all(&lower, |word: &regex::Captures<'_>| match &word[0] {
        "a" | "an" | "the" => " ".to_owned(),
        _ => word[0].to_owned(),
    });
    without_articles
        .split(|character: char| {
            character.is_whitespace() || ('\u{001c}'..='\u{001f}').contains(&character)
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn score_answer(prediction: &str, reference: &str) -> AnswerScores {
    let prediction = normalize(prediction);
    let reference = normalize(reference);
    let exact_match = f64::from(prediction == reference);
    let categorical = |text: &str| matches!(text, "yes" | "no" | "noanswer");
    if prediction != reference && (categorical(&prediction) || categorical(&reference)) {
        return AnswerScores {
            exact_match,
            f1: 0.0,
        };
    }
    let predicted: Vec<_> = prediction.split_whitespace().collect();
    let expected: Vec<_> = reference.split_whitespace().collect();
    let mut counts = HashMap::new();
    for token in &expected {
        *counts.entry(*token).or_insert(0usize) += 1;
    }
    let mut common = 0;
    for token in &predicted {
        if let Some(count) = counts.get_mut(token)
            && *count > 0
        {
            common += 1;
            *count -= 1;
        }
    }
    let f1 = if common == 0 {
        0.0
    } else {
        let precision = common as f64 / predicted.len() as f64;
        let recall = common as f64 / expected.len() as f64;
        2.0 * precision * recall / (precision + recall)
    };
    AnswerScores { exact_match, f1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_normalization_preserves_unicode_and_only_removes_ascii_punctuation() {
        for (answer, expected) in [
            (" The,   Café! an A ", "café"),
            ("the-cat's_name", "thecatsname"),
            ("‘The’ café—an", "‘ ’ café—"),
            ("İ A ΟΣ ①THE the①", "i\u{307} ος ①the the①"),
            ("the\u{345}the", "\u{345}"),
            ("a/the/an", "athean"),
            ("Alpha\u{001c} Beta\u{0085}Gamma", "alpha beta gamma"),
        ] {
            assert_eq!(normalize(answer), expected, "{answer:?}");
        }
    }

    #[test]
    fn official_exact_match_and_bag_of_tokens_f1() {
        assert_eq!(
            score_answer("The London!", "London"),
            AnswerScores {
                exact_match: 1.0,
                f1: 1.0
            }
        );
        let repeated = score_answer("red red blue", "red blue blue");
        assert_eq!(repeated.exact_match, 0.0);
        assert!((repeated.f1 - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(
            score_answer("a", "the"),
            AnswerScores {
                exact_match: 1.0,
                f1: 0.0
            }
        );
        for reference in ["yes", "no", "noanswer"] {
            assert_eq!(score_answer(reference, reference).f1, 1.0);
            assert_eq!(
                score_answer(&format!("{reference} extra"), reference).f1,
                0.0
            );
            assert_eq!(
                score_answer(reference, &format!("{reference} extra")).f1,
                0.0
            );
        }
        // Never extract a short answer using the reference: full prose receives lower F1.
        let prose = score_answer("The answer is London.", "London");
        assert_eq!(prose.exact_match, 0.0);
        assert_eq!(prose.f1, 0.5);
    }
}
