//! Benchmark contracts and registration. Dataset policy belongs in the named modules.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub mod abstentionbench;
pub mod config;
pub mod hotpotqa;
pub mod init;
pub mod longmemeval;
pub mod multihop_rag;
pub mod qasper;
pub mod ragbench;
pub mod ragtruth;
pub mod server;
pub mod temprageval;

use config::{BenchmarkDefinition, NebulaConfig, ResolvedBenchmark};
use hotpotqa::AnswerReference;
use ragtruth::ReferenceOutput;
use server::Store;

/// Result columns are declared by the evaluator, not by the shared runner.
pub type MetricValues = BTreeMap<String, f64>;
pub type Task<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A registered benchmark owns its definition, snapshot preparation and run policies.
/// Register a new implementation here; HTTP routes and storage do not dispatch on names.
pub trait BenchmarkModule: Sync {
    fn key(&self) -> &'static str;
    fn adapter(&self) -> &'static str;
    fn metric_kind(&self) -> &'static str;
    fn validate_definition(&self, definition: &BenchmarkDefinition) -> Result<()>;
    fn initialize(&self, request: &ResolvedBenchmark) -> Result<Benchmark>;

    fn answer_evaluation(&self) -> Option<&'static dyn AnswerEvaluation> {
        None
    }
    fn prepare_retrieval(
        &self,
        _request: RunRequest,
        _benchmark: &Benchmark,
        _fingerprint: String,
    ) -> Result<Run> {
        Err(Error(
            "This benchmark does not support Retrieval; use its supported run mode".into(),
        ))
    }
    fn run_retrieval(
        &self,
        store: Arc<Store>,
        _config: NebulaConfig,
        _benchmark: Benchmark,
        mut run: Run,
    ) -> Task<'static, ()> {
        Box::pin(async move {
            run.status = RunStatus::Failed;
            run.error = Some("This benchmark has no retrieval runner".into());
            run.finished_at_ms = Some(now_ms());
            if let Err(error) = store.save_run(&run) {
                eprintln!("Cannot persist run {}: {error}", run.id);
            }
        })
    }
    fn prepare_answers<'a>(
        &self,
        _config: &'a NebulaConfig,
        _request: StartAnswerRunRequest,
        _benchmark: &'a Benchmark,
        _fingerprint: String,
    ) -> Task<'a, std::result::Result<AnswerRun, StartFailure>> {
        Box::pin(async {
            Err(StartFailure::Invalid(
                "This benchmark has no answer runner".into(),
            ))
        })
    }
    fn run_answers(
        &self,
        store: Arc<Store>,
        _config: NebulaConfig,
        _benchmark: Benchmark,
        mut run: AnswerRun,
    ) -> Task<'static, ()> {
        Box::pin(async move {
            run.summary.status = RunStatus::Failed;
            run.summary.error = Some("This benchmark has no answer runner".into());
            run.summary.finished_at_ms = Some(now_ms());
            if let Err(error) = store.save_answer_run(&run) {
                eprintln!("Cannot persist run {}: {error}", run.summary.id);
            }
        })
    }
    fn review_answer(
        &self,
        _run: &mut AnswerRun,
        _case_id: &str,
        _review: AnswerReviewRequest,
    ) -> std::result::Result<(), ReviewFailure> {
        Err(ReviewFailure::Invalid(
            "This benchmark does not support human review".into(),
        ))
    }
}

/// Benchmark-owned selection and scoring invoked by the shared Nebula executor.
/// Only query text and selected source IDs reach Nebula; labels stay in the snapshot.
pub trait AnswerEvaluation: Sync {
    fn id(&self) -> &'static str;
    fn initial_scores(&self) -> Option<MetricValues> {
        None
    }
    fn candidate_sources(
        &self,
        case: &Case,
        sources: &BTreeMap<String, String>,
    ) -> Result<Vec<String>>;
    fn initialize_case(&self, _case: &Case, _captured: &mut AnswerCase) {}
    fn score_case(&self, _captured: &mut AnswerCase) {}
    fn aggregate(&self, _summary: &mut AnswerRunSummary, _cases: &[AnswerCase]) {}
    /// Benchmark columns inserted after shared answer columns in CSV exports.
    fn csv_columns(&self) -> &'static [&'static str];
    fn csv_values(&self, run: &AnswerRun, case: &AnswerCase) -> Vec<String>;
}

pub static BENCHMARKS: &[&dyn BenchmarkModule] = &[
    &ragtruth::RAGTRUTH,
    &hotpotqa::HOTPOTQA,
    &longmemeval::LONGMEMEVAL,
    &temprageval::TEMPRAGEVAL,
    &qasper::QASPER,
    &abstentionbench::ABSTENTIONBENCH,
    &multihop_rag::MULTIHOP_RAG,
    &ragbench::RAGBENCH,
];

pub fn module_for_definition(
    key: Option<&str>,
    definition: &BenchmarkDefinition,
) -> Result<&'static dyn BenchmarkModule> {
    let candidates: Vec<_> = BENCHMARKS
        .iter()
        .copied()
        .filter(|module| module.adapter() == definition.adapter)
        .collect();
    let module = if candidates.len() == 1 {
        candidates.first().copied()
    } else if let Some(key) = key {
        candidates
            .iter()
            .copied()
            .find(|module| module.key() == key)
    } else if candidates.len() > 1 {
        return Err(Error(
            "Benchmark adapter matches multiple modules; specify its catalog key".into(),
        ));
    } else {
        None
    };
    let module = module
        .ok_or_else(|| Error("No benchmark module is registered for this catalog entry".into()))?;
    if module.metric_kind() != definition.evaluation.metric_kind() {
        return Err(Error("Adapter and evaluation do not match".into()));
    }
    Ok(module)
}

pub fn module_for_snapshot(benchmark: &Benchmark) -> Result<&'static dyn BenchmarkModule> {
    if let Some(configuration) = &benchmark.configuration {
        let module = module_for_definition(Some(&configuration.key), &configuration.definition)?;
        if module.metric_kind() != benchmark.metric_kind {
            return Err(Error(
                "Snapshot evaluation does not match its benchmark module".into(),
            ));
        }
        return Ok(module);
    }
    // Snapshots predating the catalog have no configuration; their metric identifies the module.
    let mut modules = BENCHMARKS
        .iter()
        .copied()
        .filter(|module| module.metric_kind() == benchmark.metric_kind);
    match (modules.next(), modules.next()) {
        (Some(module), None) => Ok(module),
        _ => Err(Error(
            "Snapshot has no unambiguous registered benchmark module".into(),
        )),
    }
}

pub fn module_for_answer_run(run: &AnswerRun) -> Result<&'static dyn BenchmarkModule> {
    let mut modules = BENCHMARKS.iter().copied().filter(|module| {
        module
            .answer_evaluation()
            .is_some_and(|evaluation| evaluation.id() == run.summary.evaluation)
    });
    match (modules.next(), modules.next()) {
        (Some(module), None) => Ok(module),
        (None, _) => Err(Error("Answer run has no registered evaluator".into())),
        _ => Err(Error(
            "Answer evaluator matches multiple benchmark modules; its snapshot is required".into(),
        )),
    }
}

/// Prepare an immutable snapshot. Startup initialization remains reserved for init.rs.
pub fn initialize_benchmark(request: &ResolvedBenchmark) -> Result<Benchmark> {
    request.definition.validate_for(&request.key)?;
    module_for_definition(Some(&request.key), &request.definition)?.initialize(request)
}

impl Run {
    pub fn new(request: RunRequest, benchmark: &Benchmark, fingerprint: String) -> Result<Self> {
        module_for_snapshot(benchmark)?.prepare_retrieval(request, benchmark, fingerprint)
    }
}

impl AnswerRun {
    pub fn review(
        &mut self,
        case_id: &str,
        review: AnswerReviewRequest,
    ) -> std::result::Result<(), ReviewFailure> {
        module_for_answer_run(self)
            .map_err(|error| ReviewFailure::Invalid(error.to_string()))?
            .review_answer(self, case_id, review)
    }

    pub fn csv(&self) -> Result<Vec<u8>> {
        let evaluation = module_for_answer_run(self)?
            .answer_evaluation()
            .ok_or_else(|| Error("Answer run has no registered evaluator".into()))?;
        server::generation::csv(self, evaluation)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub query: String,
    pub document_id: String,
    // Annotations describe these historical outputs, never a new Nebula response.
    pub reference_outputs: Vec<ReferenceOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_reference: Option<AnswerReference>,
}

/// Corpus content exported for indexing; reference labels belong to benchmark cases.
#[derive(Clone, Deserialize, Serialize)]
pub struct Document {
    pub id: String,
    pub filename: String,
    pub text: String,
    pub revision: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Benchmark {
    pub id: Uuid,
    pub source: String,
    pub split: String,
    pub metric_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<ResolvedBenchmark>,
    pub cases: Vec<Case>,
    pub documents: Vec<Document>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub benchmark_id: Uuid,
    pub top_k: usize,
    #[serde(default)]
    pub label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRunRequest {
    pub benchmark_id: Uuid,
    pub top_k: Option<usize>,
    #[serde(default)]
    pub label: String,
}

impl StartRunRequest {
    pub fn resolve(self, benchmark: &Benchmark) -> RunRequest {
        RunRequest {
            benchmark_id: self.benchmark_id,
            top_k: self.top_k.unwrap_or_else(|| {
                benchmark
                    .configuration
                    .as_ref()
                    .map(|configuration| configuration.definition.defaults.top_k)
                    .unwrap_or(8)
            }), // Snapshots written before the catalog used 8.
            label: self.label,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Run {
    pub id: Uuid,
    pub request: RunRequest,
    pub metric_kind: String,
    pub benchmark_fingerprint: String,
    pub status: RunStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub means: Option<MetricValues>,
    pub error: Option<String>,
    pub scope: Option<Value>,
    pub watermark: Option<Value>,
    pub source_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EmbeddingModel {
    pub id: String,
    pub revision: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerProfile {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Serialize)]
pub struct AnswerRuntime {
    pub available: bool,
    pub reason: Option<String>,
    pub embedding_model: Option<EmbeddingModel>,
    pub profiles: Vec<AnswerProfile>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartAnswerRunRequest {
    pub benchmark_id: Uuid,
    pub architecture_label: String,
    pub profile_id: String,
}

impl StartAnswerRunRequest {
    pub fn validate(&self) -> Result<()> {
        if !valid_label(self.architecture_label.trim(), 256) || !valid_label(&self.profile_id, 128)
        {
            return Err(Error(
                "architecture_label must contain 1–256 bytes and profile_id 1–128 bytes, without surrounding whitespace".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct GenerationModel {
    pub profile_id: String,
    pub label: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerRunSummary {
    pub id: Uuid,
    pub request: StartAnswerRunRequest,
    pub benchmark_fingerprint: String,
    pub evaluation: String,
    pub embedding_model: EmbeddingModel,
    pub generation_model: GenerationModel,
    pub top_k: usize,
    #[serde(default = "legacy_max_in_flight")]
    pub max_in_flight: usize,
    pub status: RunStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub answered: usize,
    pub reviewed: usize,
    pub means: Option<MetricValues>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_scores: Option<MetricValues>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub scored: usize,
    pub error: Option<String>,
    pub scope: Value,
    pub watermark: Value,
    pub source_ids: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerRun {
    #[serde(flatten)]
    pub summary: AnswerRunSummary,
    pub cases: Vec<AnswerCase>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerEvidence {
    pub id: String,
    pub ordinal: usize,
    pub source_id: String,
    pub source_title: String,
    pub source_revision: String,
    pub location: String,
    pub excerpt: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerLineage {
    pub id: String,
    pub claim: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReceipt {
    pub profile_id: String,
    pub route: String,
    pub model_label: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerReviewRequest {
    pub reviewer: String,
    pub correctness: bool,
    pub groundedness: bool,
    pub hallucination: bool,
    pub citation_accuracy: bool,
    pub notes: String,
}

impl AnswerReviewRequest {
    pub fn validate(&self) -> Result<()> {
        if !valid_label(self.reviewer.trim(), 128) || self.notes.len() > 4_000 {
            return Err(Error(
                "reviewer must contain 1–128 bytes without surrounding whitespace; notes must be at most 4000 bytes".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerReview {
    #[serde(flatten)]
    pub review: AnswerReviewRequest,
    pub reviewed_at_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerCase {
    pub case_id: String,
    pub query: String,
    pub status: String,
    pub outcome: String,
    pub answer: Option<String>,
    pub reason: Option<String>,
    pub evidence: Vec<AnswerEvidence>,
    pub lineage: Vec<AnswerLineage>,
    pub model_receipt: Option<ModelReceipt>,
    pub latency_ms: u64,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<AnswerFailure>,
    pub review: Option<AnswerReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_scores: Option<MetricValues>,
}

/// Only locally generated text and allowlisted diagnostics enter durable failure logs.
#[derive(Clone, Deserialize, Serialize)]
pub struct AnswerFailure {
    pub timestamp_ms: u64,
    pub operation: String,
    pub http_status: Option<u16>,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic: Option<ProviderDiagnostic>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDiagnostic {
    pub provider: String,
    pub category: String,
    pub upstream_status: Option<u16>,
    pub upstream_code: Option<String>,
    pub upstream_type: Option<String>,
    pub request_bytes: u64,
    pub max_output_tokens: u64,
    pub attempt: u64,
    pub elapsed_ms: u64,
    pub response_truncated: bool,
}

fn legacy_max_in_flight() -> usize {
    1
}

pub enum StartFailure {
    Unavailable(String),
    Invalid(String),
}

pub enum ReviewFailure {
    NotFound,
    Running,
    Invalid(String),
    Storage(Error),
}

fn valid_label(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && value.trim() == value
}
fn is_zero(value: &usize) -> bool {
    *value == 0
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<csv::Error> for Error {
    fn from(error: csv::Error) -> Self {
        Self(error.to_string())
    }
}
