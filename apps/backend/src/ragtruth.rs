//! RAGTruth dataset preparation, paired-context retrieval, and answer review.
//!
//! Historical output annotations describe recorded model responses. New answers
//! are evaluated only through explicit review; those labels are never reused.
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use polars::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AnswerCase, AnswerEvaluation, AnswerReview, AnswerReviewRequest, AnswerRun, Benchmark,
    BenchmarkModule, Case, Document, Error, MetricValues, Result, ReviewFailure, Run, RunRequest,
    RunStatus, StartAnswerRunRequest, StartFailure, Task,
    config::{BenchmarkDefinition, NebulaConfig, ResolvedBenchmark},
    digest, now_ms,
    server::{Store, generation},
};

pub const METRIC_KIND: &str = "paired_context_recovery_v1";
pub const ANSWER_EVALUATION: &str = "manual_review_v1";

pub struct Ragtruth;
pub static RAGTRUTH: Ragtruth = Ragtruth;

impl BenchmarkModule for Ragtruth {
    fn key(&self) -> &'static str {
        "ragtruth-qa"
    }
    fn adapter(&self) -> &'static str {
        "ragtruth_qa"
    }
    fn metric_kind(&self) -> &'static str {
        METRIC_KIND
    }

    fn validate_definition(&self, definition: &BenchmarkDefinition) -> Result<()> {
        validate_definition(definition)
    }

    fn initialize(&self, request: &ResolvedBenchmark) -> Result<Benchmark> {
        initialize(request)
    }

    fn answer_evaluation(&self) -> Option<&'static dyn AnswerEvaluation> {
        Some(&RAGTRUTH)
    }

    fn prepare_retrieval(
        &self,
        request: RunRequest,
        benchmark: &Benchmark,
        fingerprint: String,
    ) -> Result<Run> {
        prepare_retrieval(request, benchmark, fingerprint)
    }

    fn run_retrieval(
        &self,
        store: Arc<Store>,
        config: NebulaConfig,
        benchmark: Benchmark,
        run: Run,
    ) -> Task<'static, ()> {
        Box::pin(execute_retrieval(store, config, benchmark, run))
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
        Box::pin(execute_answers(store, config, benchmark, run))
    }

    fn review_answer(
        &self,
        run: &mut AnswerRun,
        case_id: &str,
        review: AnswerReviewRequest,
    ) -> std::result::Result<(), ReviewFailure> {
        review_answer(run, case_id, review)
    }
}

impl AnswerEvaluation for Ragtruth {
    fn id(&self) -> &'static str {
        ANSWER_EVALUATION
    }

    fn candidate_sources(
        &self,
        _case: &Case,
        sources: &BTreeMap<String, String>,
    ) -> Result<Vec<String>> {
        // Retrieval must search the complete corpus, never the known positive context alone.
        Ok(sources.values().cloned().collect())
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
        let mut values = review_csv_values(case);
        values.extend([
            String::new(),
            String::new(),
            String::new(),
            match run.summary.status {
                RunStatus::Running => "running",
                RunStatus::Completed => "completed",
                RunStatus::Failed => "failed",
                RunStatus::Interrupted => "interrupted",
            }
            .into(),
            run.summary.total.to_string(),
            run.summary.scored.to_string(),
            String::new(),
            String::new(),
        ]);
        values
    }
}

pub fn validate_definition(definition: &BenchmarkDefinition) -> Result<()> {
    if definition.adapter != "ragtruth_qa" || definition.evaluation.metric_kind() != METRIC_KIND {
        return Err(Error("Adapter and evaluation do not match".into()));
    }
    if !["train", "test"].contains(&definition.split.as_str()) {
        return Err(Error("RAGTruth split must be train or test".into()));
    }
    if definition.source_sha256.is_some() {
        return Err(Error(
            "source_sha256 is supported for HotpotQA JSON only".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ReferenceOutput {
    pub id: String,
    pub output: String,
    pub model: String,
    pub quality: String,
    pub hallucination_labels: serde_json::Value,
}

pub fn initialize(request: &ResolvedBenchmark) -> Result<Benchmark> {
    request.definition.validate()?;
    validate_definition(&request.definition)?;
    let definition = &request.definition;
    let source = definition.source.clone();
    let columns = [
        "id",
        "query",
        "context",
        "output",
        "task_type",
        "quality",
        "model",
        "hallucination_labels",
    ];
    let frame = LazyFrame::scan_parquet(PlRefPath::new(&source), ScanArgsParquet::default())
        .and_then(|frame| {
            frame
                .select(columns.iter().map(|name| col(*name)).collect::<Vec<_>>())
                .filter(col("task_type").eq(lit("QA")))
                .limit(100_001)
                .collect()
        })
        // Cloud errors may contain URLs/credentials. Do not persist the raw error.
        .map_err(|_| {
            Error("Cannot load RAGTruth Parquet: check source, access, and required columns".into())
        })?;
    if frame.height() > 100_000 {
        return Err(Error(
            "Dataset exceeds the 100000 QA-row safety limit".into(),
        ));
    }
    let string_columns = columns
        .iter()
        .map(|name| {
            frame
                .column(name)
                .and_then(|column| column.str())
                .map_err(|_| Error(format!("Column {name} must contain strings")))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut cases = Vec::<Case>::new();
    let mut case_positions = BTreeMap::new();
    let mut documents = BTreeMap::new();
    for row in 0..frame.height() {
        let values = string_columns
            .iter()
            .zip(columns)
            .map(|(column, name)| {
                column
                    .get(row)
                    .ok_or_else(|| Error(format!("Null {name} at QA row {row}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let [
            id,
            raw_query,
            raw_context,
            output,
            _task_type,
            quality,
            model,
            raw_labels,
        ]: [&str; 8] = values.try_into().expect("eight selected RAGTruth columns");
        let query = raw_query.trim();
        let context = raw_context.trim();
        if query.is_empty() || query.len() > 16 * 1024 || context.is_empty() {
            return Err(Error(format!(
                "Empty/oversized query or empty context at QA row {row}"
            )));
        }
        let document_id = digest(context.as_bytes());
        let case_id = digest(&serde_json::to_vec(&(query, &document_id))?);
        let position = if let Some(position) = case_positions.get(&case_id) {
            *position
        } else {
            if cases.len() >= definition.defaults.limit {
                continue;
            }
            let position = cases.len();
            case_positions.insert(case_id.clone(), position);
            documents
                .entry(document_id.clone())
                .or_insert_with(|| Document {
                    id: document_id.clone(),
                    filename: format!("ragtruth-{document_id}.md"),
                    text: context.into(),
                    revision: document_id.clone(),
                });
            cases.push(Case {
                id: case_id,
                query: query.into(),
                document_id,
                reference_outputs: vec![],
                answer_reference: None,
            });
            position
        };
        let labels: serde_json::Value = serde_json::from_str(raw_labels)
            .map_err(|_| Error(format!("Invalid hallucination_labels JSON at QA row {row}")))?;
        if !labels.is_array() {
            return Err(Error(format!(
                "hallucination_labels must be an array at QA row {row}"
            )));
        }
        cases[position].reference_outputs.push(ReferenceOutput {
            id: id.into(),
            output: output.into(),
            model: model.into(),
            quality: quality.into(),
            hallucination_labels: labels,
        });
    }
    if cases.is_empty() {
        return Err(Error("No QA cases found in the dataset".into()));
    }
    Ok(Benchmark {
        id: Uuid::new_v4(),
        source,
        split: definition.split.clone(),
        metric_kind: METRIC_KIND.into(),
        configuration: Some(request.clone()),
        cases,
        documents: documents.into_values().collect(),
    })
}

pub fn prepare_retrieval(
    request: RunRequest,
    benchmark: &Benchmark,
    fingerprint: String,
) -> Result<Run> {
    if benchmark.metric_kind != METRIC_KIND {
        return Err(Error("This benchmark evaluates generated answers; use Generated answers instead of Retrieval".into()));
    }
    if !(1..=100).contains(&request.top_k) || request.label.len() > 256 {
        return Err(Error(
            "top_k must be 1–100 and label at most 256 bytes".into(),
        ));
    }
    Ok(Run {
        id: Uuid::new_v4(),
        request,
        metric_kind: benchmark.metric_kind.clone(),
        benchmark_fingerprint: fingerprint,
        status: RunStatus::Running,
        started_at_ms: now_ms(),
        finished_at_ms: None,
        total: benchmark.cases.len(),
        completed: 0,
        failed: 0,
        means: None,
        error: None,
        scope: None,
        watermark: None,
        source_ids: vec![],
    })
}

#[derive(Deserialize)]
struct Workspace {
    scope: Value,
    status: KnowledgeStatus,
    sources: Vec<Source>,
}
#[derive(Deserialize)]
struct KnowledgeStatus {
    phase: String,
    watermark: Option<Value>,
}
#[derive(Deserialize)]
struct Source {
    id: String,
    title: String,
    revision: String,
    indexed: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Conversation {
    source_scope: SourceScope,
}
#[derive(Deserialize)]
struct SourceScope {
    r#ref: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Retrieval {
    scope: Value,
    conversation_scope: Value,
    watermark: Value,
    evidence: Vec<Evidence>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Evidence {
    id: String,
    source_id: String,
    source_revision: String,
    excerpt: String,
    score: f64,
}

/// A flat schema keeps every score directly usable by spreadsheet/dashboard tools.
#[derive(Serialize)]
struct ScoreRow<'a> {
    run_id: Uuid,
    case_id: &'a str,
    query: &'a str,
    expected_document_id: &'a str,
    expected_source_id: &'a str,
    top_k: usize,
    status: &'a str,
    latency_ms: u128,
    context_hit_at_k: Option<f64>,
    reciprocal_rank_at_k: Option<f64>,
    ndcg_at_k: Option<f64>,
    retrieved_evidence_json: String,
    error: &'a str,
}

pub async fn execute_retrieval(
    store: Arc<Store>,
    config: NebulaConfig,
    benchmark: Benchmark,
    mut run: Run,
) {
    match execute_inner(&store, &config, &benchmark, &mut run).await {
        Ok(()) => run.status = RunStatus::Completed,
        Err(error) => {
            run.status = RunStatus::Failed;
            run.error = Some(error.to_string());
        }
    }
    run.finished_at_ms = Some(now_ms());
    if let Err(error) = store.save_run(&run) {
        eprintln!("Cannot persist terminal status for run {}: {error}", run.id);
    }
}

async fn execute_inner(
    store: &Store,
    config: &NebulaConfig,
    benchmark: &Benchmark,
    run: &mut Run,
) -> Result<()> {
    config.validate()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| Error("Cannot create Nebula client".into()))?;
    let base = config.base_url.trim_end_matches('/');
    let workspace: Workspace = response(
        client
            .get(format!("{base}/workspace"))
            .bearer_auth(&config.token),
    )
    .await?;
    if workspace.status.phase != "ready" {
        return Err(Error(
            "Nebula is not ready; index the exported corpus before starting a run".into(),
        ));
    }
    let watermark = workspace
        .status
        .watermark
        .filter(|value| value.is_object())
        .ok_or_else(|| Error("Nebula did not provide a knowledge watermark".into()))?;
    let mut sources = BTreeMap::new();
    let mut revisions = HashMap::new();
    for document in &benchmark.documents {
        let matches = workspace
            .sources
            .iter()
            .filter(|source| {
                source.indexed
                    && source.title == document.filename
                    && source.revision == document.revision
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(Error(format!(
                "Expected one indexed copy of {}; check the Nebula corpus and revisions",
                document.filename
            )));
        }
        sources.insert(document.id.clone(), matches[0].id.clone());
        revisions.insert(matches[0].id.clone(), matches[0].revision.clone());
    }
    // Select every benchmark context, never just this question's positive context.
    let source_ids = sources.values().cloned().collect::<Vec<_>>();
    let selection = json!({"scope": workspace.scope, "sourceIds": source_ids});
    if serde_json::to_vec(&selection)?.len() > 64 * 1024 {
        return Err(Error("Corpus selection exceeds Nebula's 64 KiB request limit; load a smaller benchmark limit".into()));
    }
    let conversation: Conversation = response(
        client
            .post(format!("{base}/conversations"))
            .bearer_auth(&config.token)
            .json(&selection),
    )
    .await?;
    run.scope = Some(workspace.scope.clone());
    run.watermark = Some(watermark.clone());
    run.source_ids = source_ids;
    store.save_run(run)?;
    let mut csv = csv::Writer::from_writer(store.create_scores(run.id)?);
    let mut totals = Scores::default();
    for case in &benchmark.cases {
        let expected = &sources[&case.document_id];
        let start = Instant::now();
        let result: Result<Retrieval> = response(
            client
                .post(format!("{base}/retrieve"))
                .bearer_auth(&config.token)
                .json(&json!({
                    "scope": workspace.scope, "conversationScope": conversation.source_scope.r#ref,
                    "query": case.query, "topK": run.request.top_k,
                })),
        )
        .await;
        let latency_ms = start.elapsed().as_millis();
        let result = result.and_then(|retrieval| {
            if retrieval.scope != workspace.scope
                || retrieval.watermark != watermark
                || retrieval.conversation_scope != conversation.source_scope.r#ref
            {
                return Err(Error(
                    "Nebula scope/index changed during the run; start a fresh run".into(),
                ));
            }
            if retrieval.evidence.len() > run.request.top_k
                || retrieval.evidence.iter().any(|item| {
                    revisions.get(&item.source_id) != Some(&item.source_revision)
                        || !item.score.is_finite()
                })
            {
                return Err(Error(
                    "Nebula returned evidence outside the pinned benchmark corpus".into(),
                ));
            }
            Ok(retrieval)
        });
        let (scores, evidence, error) = match result {
            Ok(retrieval) => {
                let ranked = retrieval
                    .evidence
                    .iter()
                    .map(|item| item.source_id.clone())
                    .collect::<Vec<_>>();
                (
                    Some(score_context(expected, &ranked, run.request.top_k)),
                    retrieval.evidence,
                    String::new(),
                )
            }
            Err(error) => (None, vec![], error.to_string()),
        };
        csv.serialize(ScoreRow {
            run_id: run.id,
            case_id: &case.id,
            query: &case.query,
            expected_document_id: &case.document_id,
            expected_source_id: expected,
            top_k: run.request.top_k,
            status: if scores.is_some() { "ok" } else { "error" },
            latency_ms,
            context_hit_at_k: scores.map(|s| s.context_hit_at_k),
            reciprocal_rank_at_k: scores.map(|s| s.reciprocal_rank_at_k),
            ndcg_at_k: scores.map(|s| s.ndcg_at_k),
            retrieved_evidence_json: serde_json::to_string(&evidence)?,
            error: &error,
        })?;
        csv.flush()?;
        csv.get_ref().sync_data()?;
        if let Some(scores) = scores {
            run.completed += 1;
            totals.add(scores);
            run.means = totals.mean(run.completed).map(Scores::metrics);
        } else {
            run.failed += 1;
        }
        store.save_run(run)?;
        if !error.is_empty() {
            return Err(Error(error));
        }
    }
    Ok(())
}

async fn response<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    let response = request
        .send()
        .await
        .map_err(|_| Error("Nebula request failed or timed out".into()))?;
    if !response.status().is_success() {
        return Err(Error(format!(
            "Nebula returned HTTP {}",
            response.status().as_u16()
        )));
    }
    response
        .json()
        .await
        .map_err(|_| Error("Nebula returned an invalid response".into()))
}

/// The paired context is the single positive document. Repeated chunks keep
/// their original ranks and earn credit only at the first matching position.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Scores {
    pub context_hit_at_k: f64,
    pub reciprocal_rank_at_k: f64,
    pub ndcg_at_k: f64,
}

pub fn score_context(expected_source: &str, ranked_sources: &[String], top_k: usize) -> Scores {
    let rank = ranked_sources
        .iter()
        .take(top_k)
        .position(|source| source == expected_source);
    match rank {
        Some(index) => Scores {
            context_hit_at_k: 1.0,
            reciprocal_rank_at_k: 1.0 / (index + 1) as f64,
            ndcg_at_k: 1.0 / ((index + 2) as f64).log2(),
        },
        None => Scores::default(),
    }
}

impl Scores {
    pub fn metrics(self) -> MetricValues {
        BTreeMap::from([
            ("context_hit_at_k".into(), self.context_hit_at_k),
            ("reciprocal_rank_at_k".into(), self.reciprocal_rank_at_k),
            ("ndcg_at_k".into(), self.ndcg_at_k),
        ])
    }

    pub fn add(&mut self, other: Self) {
        self.context_hit_at_k += other.context_hit_at_k;
        self.reciprocal_rank_at_k += other.reciprocal_rank_at_k;
        self.ndcg_at_k += other.ndcg_at_k;
    }

    pub fn mean(self, count: usize) -> Option<Self> {
        (count > 0).then(|| Self {
            context_hit_at_k: self.context_hit_at_k / count as f64,
            reciprocal_rank_at_k: self.reciprocal_rank_at_k / count as f64,
            ndcg_at_k: self.ndcg_at_k / count as f64,
        })
    }
}

/// Human-review fields shared only by benchmark modules opting into this evaluation.
pub fn review_csv_values(case: &AnswerCase) -> Vec<String> {
    let review = case.review.as_ref();
    let boolean = |value: fn(&AnswerReviewRequest) -> bool| {
        review
            .map(|review| value(&review.review).to_string())
            .unwrap_or_default()
    };
    vec![
        review
            .map(|review| review.review.reviewer.clone())
            .unwrap_or_default(),
        boolean(|review| review.correctness),
        boolean(|review| review.groundedness),
        boolean(|review| review.hallucination),
        boolean(|review| review.citation_accuracy),
        review
            .map(|review| review.review.notes.clone())
            .unwrap_or_default(),
        review
            .map(|review| review.reviewed_at_ms.to_string())
            .unwrap_or_default(),
    ]
}

pub fn review_answer(
    run: &mut AnswerRun,
    case_id: &str,
    mut review: AnswerReviewRequest,
) -> std::result::Result<(), ReviewFailure> {
    if run.summary.status == RunStatus::Running {
        return Err(ReviewFailure::Running);
    }
    review
        .validate()
        .map_err(|error| ReviewFailure::Invalid(error.to_string()))?;
    review.reviewer = review.reviewer.trim().to_owned();
    let case = run
        .cases
        .iter_mut()
        .find(|case| case.case_id == case_id)
        .ok_or(ReviewFailure::NotFound)?;
    if case.status != "ok" || case.outcome != "answered" {
        return Err(ReviewFailure::Invalid(
            "Only successfully generated answers can be reviewed".into(),
        ));
    }
    case.review = Some(AnswerReview {
        review,
        reviewed_at_ms: now_ms(),
    });
    let reviews = run
        .cases
        .iter()
        .filter(|case| case.status == "ok" && case.outcome == "answered")
        .filter_map(|case| case.review.as_ref())
        .collect::<Vec<_>>();
    run.summary.reviewed = reviews.len();
    run.summary.means = (!reviews.is_empty()).then(|| {
        let mean = |field: fn(&AnswerReviewRequest) -> bool| {
            reviews
                .iter()
                .filter(|review| field(&review.review))
                .count() as f64
                / reviews.len() as f64
        };
        BTreeMap::from([
            ("correctness".into(), mean(|review| review.correctness)),
            ("groundedness".into(), mean(|review| review.groundedness)),
            (
                "hallucination_rate".into(),
                mean(|review| review.hallucination),
            ),
            (
                "citation_accuracy".into(),
                mean(|review| review.citation_accuracy),
            ),
        ])
    });
    Ok(())
}

/// Generated RAGTruth answers use the full prepared corpus and explicit review.
pub async fn prepare_answers(
    config: &NebulaConfig,
    request: StartAnswerRunRequest,
    benchmark: &Benchmark,
    fingerprint: String,
) -> std::result::Result<AnswerRun, StartFailure> {
    if benchmark.metric_kind != METRIC_KIND {
        return Err(StartFailure::Invalid(
            "RAGTruth requires its paired-context snapshot".into(),
        ));
    }
    generation::prepare(config, request, benchmark, fingerprint, &RAGTRUTH).await
}

pub async fn execute_answers(
    store: Arc<Store>,
    config: NebulaConfig,
    benchmark: Benchmark,
    run: AnswerRun,
) {
    generation::execute(store, config, benchmark, run, &RAGTRUTH).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_not_promoted_when_a_document_has_multiple_chunks() {
        let sources = ["b", "b", "a", "a"].map(String::from);
        let score = score_context("a", &sources, 4);
        assert_eq!(score.context_hit_at_k, 1.0);
        assert!((score.reciprocal_rank_at_k - 0.3333333333333333).abs() < 1e-12);
        assert_eq!(score.ndcg_at_k, 0.5);
        assert_eq!(score_context("a", &sources, 2).context_hit_at_k, 0.0);
        assert_eq!(score_context("a", &[], 4).context_hit_at_k, 0.0);
    }
}
