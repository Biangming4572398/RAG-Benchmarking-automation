use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::Read,
    sync::{Arc, LazyLock},
    time::Duration,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AnswerCase, AnswerEvaluation, AnswerReviewRequest, AnswerRun, AnswerRunSummary, Benchmark,
    BenchmarkModule, Case, Document, Error, MetricValues, Result, ReviewFailure, RunStatus,
    StartAnswerRunRequest, StartFailure, Task,
    config::{BenchmarkDefinition, NebulaConfig, ResolvedBenchmark},
    digest,
    server::{Store, generation},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;

/// HotpotQA's dataset contract, distractor selection and answer-only evaluation.
pub struct HotpotQa;
pub static HOTPOTQA: HotpotQa = HotpotQa;

impl BenchmarkModule for HotpotQa {
    fn key(&self) -> &'static str {
        "hotpotqa"
    }
    fn adapter(&self) -> &'static str {
        "hotpotqa_distractor"
    }
    fn metric_kind(&self) -> &'static str {
        HOTPOTQA_EVALUATION
    }
    fn validate_definition(&self, definition: &BenchmarkDefinition) -> Result<()> {
        validate(definition)
    }
    fn initialize(&self, request: &ResolvedBenchmark) -> Result<Benchmark> {
        initialize(request)
    }
    fn answer_evaluation(&self) -> Option<&'static dyn AnswerEvaluation> {
        Some(&HOTPOTQA)
    }
    fn prepare_answers<'a>(
        &self,
        config: &'a NebulaConfig,
        request: StartAnswerRunRequest,
        benchmark: &'a Benchmark,
        fingerprint: String,
    ) -> Task<'a, std::result::Result<AnswerRun, StartFailure>> {
        Box::pin(prepare_answers(config, request, benchmark, fingerprint))
    }
    fn run_answers(
        &self,
        store: Arc<Store>,
        config: NebulaConfig,
        benchmark: Benchmark,
        run: AnswerRun,
    ) -> Task<'static, ()> {
        Box::pin(run_answers(store, config, benchmark, run))
    }
    fn review_answer(
        &self,
        run: &mut AnswerRun,
        case_id: &str,
        review: AnswerReviewRequest,
    ) -> std::result::Result<(), ReviewFailure> {
        // Human assessment is supplementary; it never replaces official answer scores.
        crate::ragtruth::review_answer(run, case_id, review)
    }
}

impl AnswerEvaluation for HotpotQa {
    fn id(&self) -> &'static str {
        HOTPOTQA_EVALUATION
    }
    fn initial_scores(&self) -> Option<MetricValues> {
        Some(AnswerScores::default().into())
    }
    fn candidate_sources(
        &self,
        case: &Case,
        sources: &BTreeMap<String, String>,
    ) -> Result<Vec<String>> {
        case_sources(case, sources)
    }
    fn initialize_case(&self, case: &Case, captured: &mut AnswerCase) {
        captured.reference_answer = case
            .answer_reference
            .as_ref()
            .map(|reference| reference.answer.clone());
        captured.automatic_scores = self.initial_scores();
    }
    fn score_case(&self, captured: &mut AnswerCase) {
        if captured.outcome == "answered"
            && let Some(reference) = &captured.reference_answer
        {
            captured.automatic_scores = Some(
                score_answer(captured.answer.as_deref().unwrap_or_default(), reference).into(),
            );
        }
    }
    fn aggregate(&self, summary: &mut AnswerRunSummary, cases: &[AnswerCase]) {
        let mut means = AnswerScores::default();
        summary.scored = 0;
        // The denominator is every scheduled question. Failed requests, abstentions and
        // unfinished cases contribute zero; processing in dataset order is deterministic.
        for case in cases {
            if let Some(scores) = &case.automatic_scores {
                summary.scored += 1;
                means.exact_match = (means.exact_match
                    + scores.get("exact_match").copied().unwrap_or_default()
                        / summary.total as f64)
                    .min(1.0);
                means.f1 = (means.f1
                    + scores.get("f1").copied().unwrap_or_default() / summary.total as f64)
                    .min(1.0);
            }
        }
        summary.automatic_scores = Some(means.into());
    }
    fn csv_columns(&self) -> &'static [&'static str] {
        &[
            "reviewer",
            "correctness",
            "groundedness",
            "hallucination",
            "citation_accuracy",
            "review_notes",
            "reviewed_at_ms",
            "reference_answer",
            "exact_match",
            "f1",
            "run_status",
            "run_total",
            "run_scored",
            "run_exact_match",
            "run_f1",
        ]
    }
    fn csv_values(&self, run: &AnswerRun, case: &AnswerCase) -> Vec<String> {
        let score = |scores: &Option<MetricValues>, key: &str| {
            scores
                .as_ref()
                .and_then(|scores| scores.get(key))
                .map(ToString::to_string)
                .unwrap_or_default()
        };
        let mut values = crate::ragtruth::review_csv_values(case);
        values.extend([
            case.reference_answer.clone().unwrap_or_default(),
            score(&case.automatic_scores, "exact_match"),
            score(&case.automatic_scores, "f1"),
            match run.summary.status {
                RunStatus::Running => "running",
                RunStatus::Completed => "completed",
                RunStatus::Failed => "failed",
                RunStatus::Interrupted => "interrupted",
            }
            .into(),
            run.summary.total.to_string(),
            run.summary.scored.to_string(),
            score(&run.summary.automatic_scores, "exact_match"),
            score(&run.summary.automatic_scores, "f1"),
        ]);
        values
    }
}

pub async fn prepare_answers(
    config: &NebulaConfig,
    request: StartAnswerRunRequest,
    benchmark: &Benchmark,
    fingerprint: String,
) -> std::result::Result<AnswerRun, StartFailure> {
    if benchmark.metric_kind != HOTPOTQA_EVALUATION {
        return Err(StartFailure::Invalid(
            "HotpotQA requires its answer evaluation snapshot".into(),
        ));
    }
    generation::prepare(config, request, benchmark, fingerprint, &HOTPOTQA).await
}

pub async fn run_answers(
    store: Arc<Store>,
    config: NebulaConfig,
    benchmark: Benchmark,
    run: AnswerRun,
) {
    generation::execute(store, config, benchmark, run, &HOTPOTQA).await;
}

/// Gold labels and the complete distractor set; never index these as corpus text.
#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerReference {
    pub answer: String,
    pub candidate_document_ids: Vec<String>,
    pub supporting_facts: Vec<SupportingFact>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SupportingFact {
    pub title: String,
    pub sentence_index: usize,
}

#[derive(Deserialize)]
struct Example {
    #[serde(rename = "_id")]
    id: String,
    question: String,
    answer: String,
    context: Vec<(String, Vec<String>)>,
    supporting_facts: Vec<(String, usize)>,
}

/// Validate the source contract before reading data or preparing a run.
pub fn validate(definition: &BenchmarkDefinition) -> Result<()> {
    if definition.adapter != "hotpotqa_distractor"
        || definition.evaluation.metric_kind() != HOTPOTQA_EVALUATION
    {
        return Err(Error("Adapter and evaluation do not match".into()));
    }
    if definition.split != "dev" {
        return Err(Error("HotpotQA distractor split must be dev".into()));
    }
    if let Some(checksum) = &definition.source_sha256
        && (checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(Error(
            "source_sha256 must be a 64-digit SHA-256 checksum".into(),
        ));
    }
    if definition.source.contains("://") {
        let url = reqwest::Url::parse(&definition.source).map_err(|_| {
            Error("HotpotQA source must be a local path or public HTTP(S) URL".into())
        })?;
        if !["http", "https"].contains(&url.scheme())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Error("HotpotQA source must be a local path or public HTTP(S) URL without credentials or fragments".into()));
        }
    }
    Ok(())
}

/// Preserve the dataset's order and per-question distractors. Only the passages
/// enter the corpus; reference answers/support annotations remain private labels.
pub fn initialize(request: &ResolvedBenchmark) -> Result<Benchmark> {
    request.definition.validate()?;
    validate(&request.definition)?;
    let bytes = read_source(&request.definition.source)?;
    if request
        .definition
        .source_sha256
        .as_ref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&digest(&bytes)))
    {
        return Err(Error(
            "HotpotQA source SHA-256 does not match the catalog".into(),
        ));
    }
    let examples: Vec<Example> = serde_json::from_slice(&bytes).map_err(|_| {
        Error("Invalid HotpotQA JSON: expected development distractor examples".into())
    })?;
    if examples.is_empty() || examples.len() > 10_000 {
        return Err(Error("HotpotQA must contain 1–10000 examples".into()));
    }
    let mut example_ids = BTreeSet::new();
    for example in &examples {
        if example.id.is_empty()
            || example.id.len() > 128
            || !example
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            || !example_ids.insert(&example.id)
        {
            return Err(Error(
                "HotpotQA example IDs must be unique nonempty identifiers".into(),
            ));
        }
    }
    let mut cases = Vec::new();
    let mut documents = BTreeMap::new();
    for (row, example) in examples
        .into_iter()
        .take(request.definition.defaults.limit)
        .enumerate()
    {
        if example.question.trim().is_empty()
            || example.question.len() > 16 * 1024
            || example.answer.trim().is_empty()
            || example.answer.len() > 16 * 1024
            // The published dev file has a few examples with fewer distractors.
            || !(2..=10).contains(&example.context.len())
            || example.supporting_facts.is_empty()
            || example.supporting_facts.len() > 1000
        {
            return Err(Error(format!(
                "Invalid question, answer, or distractor context at HotpotQA row {row}"
            )));
        }
        let mut titles = BTreeSet::new();
        let mut candidate_document_ids = Vec::new();
        for (title, sentences) in &example.context {
            if title.trim().is_empty()
                || title.len() > 8 * 1024
                || sentences.is_empty()
                || sentences.len() > 1000
                || sentences.iter().any(|sentence| sentence.len() > 64 * 1024)
                || sentences.iter().all(|sentence| sentence.trim().is_empty())
                || !titles.insert(title.as_str())
            {
                return Err(Error(format!(
                    "Invalid or duplicate passage at HotpotQA row {row}"
                )));
            }
            // Include sentence boundaries in identity so distinct versions of the
            // same Wikipedia title cannot silently reuse another case's passage.
            let id = digest(&serde_json::to_vec(&(title, sentences))?);
            candidate_document_ids.push(id.clone());
            documents.entry(id.clone()).or_insert_with(|| {
                let text = format!("# {title}\n\n{}\n", sentences.join("\n"));
                Document {
                    id: id.clone(),
                    filename: format!("hotpotqa-{id}.md"),
                    revision: digest(text.as_bytes()),
                    text,
                }
            });
        }
        // These original annotations are retained, not scored or used to select
        // retrieval sources. The official file includes unresolved sentence IDs.
        for (title, _) in &example.supporting_facts {
            if title.trim().is_empty() || title.len() > 8 * 1024 {
                return Err(Error(format!(
                    "Invalid supporting fact title at HotpotQA row {row}"
                )));
            }
        }
        cases.push(Case {
            id: example.id,
            query: example.question.trim().into(),
            document_id: String::new(),
            reference_outputs: Vec::new(),
            answer_reference: Some(AnswerReference {
                answer: example.answer,
                candidate_document_ids,
                supporting_facts: example
                    .supporting_facts
                    .into_iter()
                    .map(|(title, sentence_index)| SupportingFact {
                        title,
                        sentence_index,
                    })
                    .collect(),
            }),
        });
    }
    Ok(Benchmark {
        id: Uuid::new_v4(),
        source: request.definition.source.clone(),
        split: request.definition.split.clone(),
        metric_kind: request.definition.evaluation.metric_kind().into(),
        configuration: Some(request.clone()),
        cases,
        documents: documents.into_values().collect(),
    })
}

pub fn case_sources(case: &Case, sources: &BTreeMap<String, String>) -> Result<Vec<String>> {
    let reference = case
        .answer_reference
        .as_ref()
        .ok_or_else(|| Error("HotpotQA case is missing its answer reference".into()))?;
    if reference.answer.trim().is_empty()
        || !(2..=10).contains(&reference.candidate_document_ids.len())
        || reference
            .candidate_document_ids
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != reference.candidate_document_ids.len()
    {
        return Err(Error("HotpotQA requires a reference answer and 2–10 distinct candidate documents per question".into()));
    }
    reference
        .candidate_document_ids
        .iter()
        .map(|id| {
            sources.get(id).cloned().ok_or_else(|| {
                Error("HotpotQA candidate document is absent from the prepared corpus".into())
            })
        })
        .collect()
}

fn read_source(source: &str) -> Result<Vec<u8>> {
    let bytes = if source.contains("://") {
        // Loading runs on the server's blocking worker, alongside the synchronous
        // Parquet loader. Keep this private runtime outside the request executor.
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Error("Cannot initialize HotpotQA download".into()))?
            .block_on(download(source))?
    } else {
        let mut bytes = Vec::new();
        File::open(source)
            .and_then(|file| file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes))
            .map_err(|_| {
                Error("Cannot read HotpotQA JSON: check the catalog source path".into())
            })?;
        bytes
    };
    if bytes.len() > MAX_BYTES {
        return Err(Error(
            "HotpotQA JSON exceeds the 64 MiB safety limit".into(),
        ));
    }
    Ok(bytes)
}

async fn download(source: &str) -> Result<Vec<u8>> {
    let failure = || {
        Error("Cannot download HotpotQA JSON: check the public source URL and connectivity".into())
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| failure())?;
    let mut response = client
        .get(source)
        .send()
        .await
        .map_err(|_| failure())?
        .error_for_status()
        .map_err(|_| failure())?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BYTES as u64)
    {
        return Err(Error(
            "HotpotQA JSON exceeds the 64 MiB safety limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
        if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
            return Err(Error(
                "HotpotQA JSON exceeds the 64 MiB safety limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// Answer-only v1 metrics follow the authors' evaluator:
// https://github.com/hotpotqa/hotpot/blob/master/hotpot_evaluate_v1.py
// Score the complete returned answer; supporting-fact evaluation is a separate task.
pub const HOTPOTQA_EVALUATION: &str = "hotpotqa_answer_v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct AnswerScores {
    pub exact_match: f64,
    pub f1: f64,
}

impl From<AnswerScores> for MetricValues {
    fn from(scores: AnswerScores) -> Self {
        BTreeMap::from([
            ("exact_match".into(), scores.exact_match),
            ("f1".into(), scores.f1),
        ])
    }
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
