//! Generated answers, reference-answer scoring, and explicit human review.
//! Historical RAGTruth labels never score new answers.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    Error, Result,
    answer_scores::{AnswerScores, HOTPOTQA_EVALUATION, score_answer},
    config::NebulaConfig,
    load_benchmarks::{Benchmark, Case},
    storage::Store,
    trials::{RunStatus, now_ms},
};

pub const EVALUATION: &str = "manual_review_v1";
const GENERATION_BATCH_SIZE: usize = 4;
const QUERY_TOP_K: usize = 8; // Nebula /query fixes this value; it is not a request option.

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

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct AnswerMeans {
    pub correctness: f64,
    pub groundedness: f64,
    pub hallucination_rate: f64,
    pub citation_accuracy: f64,
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
    pub means: Option<AnswerMeans>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_scores: Option<AnswerScores>,
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
    pub automatic_scores: Option<AnswerScores>,
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

struct CaseFailure {
    diagnostic: AnswerFailure,
    fatal: bool,
}

impl CaseFailure {
    fn local(operation: &str, code: &str, message: impl Into<String>, fatal: bool) -> Self {
        Self {
            diagnostic: AnswerFailure {
                timestamp_ms: now_ms(),
                operation: operation.into(),
                http_status: None,
                code: code.into(),
                message: message.into(),
                provider_diagnostic: None,
            },
            fatal,
        }
    }
}

fn is_zero(value: &usize) -> bool {
    *value == 0
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

#[derive(Deserialize)]
struct Workspace {
    scope: Value,
    status: KnowledgeStatus,
    sources: Vec<Source>,
    #[serde(default)]
    profiles: Vec<RuntimeProfile>,
}

#[derive(Deserialize)]
struct KnowledgeStatus {
    phase: String,
    watermark: Option<Value>,
    // Nebula's HTTP adapter flattens the portable engine's embedding diagnostics.
    // See Nebula apps/backend/internal/protocol/types.go KnowledgeStatus.
    model: Option<RuntimeEmbedding>,
}

#[derive(Deserialize)]
struct RuntimeEmbedding {
    id: String,
    revision: String,
    phase: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProfile {
    id: String,
    label: String,
    enabled: bool,
    disabled_reason: Option<String>,
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
    id: String,
    source_scope: SourceScope,
}

#[derive(Deserialize)]
struct SourceScope {
    r#ref: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedAnswer {
    scope: Value,
    conversation_scope: Value,
    outcome: String,
    answer: Option<String>,
    reason: Option<String>,
    evidence: Vec<AnswerEvidence>,
    lineage: Vec<AnswerLineage>,
    watermark: Option<Value>,
    model_receipt: Option<ModelReceipt>,
}

fn valid_label(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && value.trim() == value
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| Error("Cannot create Nebula client".into()))
}

async fn response<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    let mut response = request
        .send()
        .await
        .map_err(|_| Error("Nebula request failed or timed out".into()))?;
    if !response.status().is_success() {
        return Err(Error(format!(
            "Nebula returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Error("Cannot read Nebula response".into()))?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(Error("Nebula response exceeds the 8 MiB limit".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error("Nebula returned an invalid response".into()))
}

// Never persist arbitrary upstream error bodies: providers may echo prompts or credentials.
fn public_error(code: &str) -> Option<(&'static str, bool)> {
    Some(match code {
        "reasoning_unavailable" => ("The remote reasoning service is unavailable", false),
        "reasoning_authentication_failed" => ("Remote reasoning authentication failed", false),
        "reasoning_rate_limited" => (
            "The remote reasoning service rate limited this request",
            false,
        ),
        "reasoning_rejected" => ("The remote reasoning service rejected this request", false),
        "reasoning_invalid_response" => (
            "The remote reasoning service returned an invalid response",
            false,
        ),
        "reasoning_failed" => ("Remote reasoning failed", false),
        "scope_mismatch" => ("Nebula workspace scope changed", true),
        "knowledge_not_ready" => ("Nebula knowledge is no longer ready", true),
        "knowledge_unavailable" => ("Nebula knowledge is unavailable", true),
        "knowledge_changed" => ("Nebula knowledge changed during generation", true),
        "conversation_not_found" => ("Nebula conversation is no longer available", true),
        "conversation_scope_changed" => ("Nebula conversation scope changed", true),
        "source_selection_invalid" => ("Nebula rejected the pinned source selection", true),
        "invalid_request" => ("Nebula rejected this request", false),
        _ => return None,
    })
}

fn provider_diagnostic(value: &Value) -> Option<ProviderDiagnostic> {
    let category = value["category"].as_str()?;
    if value["provider"] != "moonshot"
        || ![
            "rejected",
            "rate_limited",
            "authentication",
            "unavailable",
            "invalid_response",
            "incomplete_response",
        ]
        .contains(&category)
    {
        return None;
    }
    let known_symbol = |value: &Value| {
        value
            .as_str()
            .filter(|code| {
                [
                    "invalid_request_error",
                    "context_length_exceeded",
                    "context_window_exceeded",
                    "prompt_too_long",
                    "rate_limit_exceeded",
                    "rate_limit_reached",
                    "insufficient_quota",
                    "invalid_api_key",
                    "content_filter",
                    "content_policy_violation",
                    "model_not_found",
                    "invalid_parameter",
                    "unsupported_parameter",
                    "engine_overloaded",
                    "server_error",
                    "authentication_error",
                    "rate_limit_error",
                    "permission_error",
                    "not_found_error",
                    "api_error",
                    "overloaded_error",
                ]
                .contains(code)
            })
            .map(str::to_owned)
    };
    // These records are also consumed by JavaScript. Invalid metadata must never make
    // the otherwise useful HTTP failure unreadable in the dashboard.
    let safe_count = |field: &str| {
        value[field]
            .as_u64()
            .filter(|count| *count <= 9_007_199_254_740_991)
    };
    let max_output_tokens = safe_count("maxOutputTokens").filter(|count| *count > 0)?;
    let attempt = safe_count("attempt").filter(|count| *count > 0)?;
    Some(ProviderDiagnostic {
        provider: "moonshot".into(),
        category: category.into(),
        upstream_status: value["upstreamStatus"]
            .as_u64()
            .filter(|status| (100..=599).contains(status))
            .map(|status| status as u16),
        upstream_code: known_symbol(&value["upstreamCode"]),
        upstream_type: known_symbol(&value["upstreamType"]),
        request_bytes: safe_count("requestBytes")?,
        max_output_tokens,
        attempt,
        elapsed_ms: safe_count("elapsedMs")?,
        response_truncated: value["responseTruncated"].as_bool().unwrap_or(false),
    })
}

async fn case_response<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
    operation: &str,
) -> std::result::Result<T, CaseFailure> {
    let mut response = request.send().await.map_err(|_| {
        CaseFailure::local(
            operation,
            "request_failed",
            "Nebula request failed or timed out",
            false,
        )
    })?;
    let status = response.status();
    let limit = if status.is_success() {
        8 * 1024 * 1024
    } else {
        16 * 1024
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        CaseFailure::local(
            operation,
            "response_read_failed",
            "Cannot read Nebula response",
            false,
        )
    })? {
        if bytes.len() + chunk.len() > limit {
            if status.is_success() {
                return Err(CaseFailure::local(
                    operation,
                    "response_too_large",
                    "Nebula response exceeds the 8 MiB limit",
                    false,
                ));
            }
            // Even malformed/oversized error responses retain their safe HTTP status.
            bytes.clear();
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let code = value["error"]["code"].as_str().unwrap_or_default();
        let known = public_error(code);
        let mut failure = CaseFailure::local(
            operation,
            if known.is_some() { code } else { "http_error" },
            known
                .map(|(message, _)| message.to_owned())
                .unwrap_or_else(|| format!("Nebula returned HTTP {}", status.as_u16())),
            known.is_some_and(|(_, fatal)| fatal),
        );
        failure.diagnostic.http_status = Some(status.as_u16());
        failure.diagnostic.provider_diagnostic = provider_diagnostic(&value["error"]["diagnostic"]);
        return Err(failure);
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        CaseFailure::local(
            operation,
            "invalid_response",
            "Nebula returned an invalid response",
            false,
        )
    })
}

async fn workspace(config: &NebulaConfig) -> Result<Workspace> {
    config.validate()?;
    response(
        client()?
            .get(format!(
                "{}/workspace",
                config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&config.token)
            .timeout(Duration::from_secs(10)),
    )
    .await
}

fn describe_runtime(workspace: &Workspace) -> AnswerRuntime {
    let model = workspace.status.model.as_ref();
    let embedding_model = model
        .filter(|model| valid_label(&model.id, 256) && valid_label(&model.revision, 256))
        .map(|model| EmbeddingModel {
            id: model.id.clone(),
            revision: model.revision.clone(),
        });
    let profiles: Vec<_> = workspace
        .profiles
        .iter()
        .filter(|profile| valid_label(&profile.id, 128) && valid_label(&profile.label, 256))
        .map(|profile| AnswerProfile {
            id: profile.id.clone(),
            label: profile.label.clone(),
            enabled: profile.enabled,
            disabled_reason: profile.disabled_reason.clone(),
        })
        .collect();
    let reason = if workspace.status.phase != "ready"
        || !workspace
            .status
            .watermark
            .as_ref()
            .is_some_and(Value::is_object)
    {
        Some("Nebula must finish indexing before generating answers".into())
    } else if embedding_model.is_none() || !model.is_some_and(|model| model.phase == "ready") {
        Some("Nebula has not reported a ready embedding model and revision".into())
    } else if !profiles.iter().any(|profile| profile.enabled) {
        Some("Enable a generation profile in the Nebula backend to generate answers".into())
    } else {
        None
    };
    AnswerRuntime {
        available: reason.is_none(),
        reason,
        embedding_model,
        profiles,
    }
}

pub async fn runtime(config: Option<&NebulaConfig>) -> AnswerRuntime {
    let unavailable = |reason: String| AnswerRuntime {
        available: false,
        reason: Some(reason),
        embedding_model: None,
        profiles: vec![],
    };
    let Some(config) = config else {
        return unavailable("Configure a Nebula backend before generating answers".into());
    };
    match workspace(config).await {
        Ok(workspace) => describe_runtime(&workspace),
        Err(error) => unavailable(error.to_string()),
    }
}

fn selected_sources(
    workspace: &Workspace,
    benchmark: &Benchmark,
) -> Result<BTreeMap<String, String>> {
    let mut selected = BTreeMap::new();
    for document in &benchmark.documents {
        let matching = workspace
            .sources
            .iter()
            .filter(|source| {
                source.indexed
                    && source.title == document.filename
                    && source.revision == document.revision
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 || !valid_label(&matching[0].id, 256) {
            return Err(Error(format!(
                "Expected one indexed copy of {}; check the Nebula corpus and revisions",
                document.filename
            )));
        }
        selected.insert(document.id.clone(), matching[0].id.clone());
    }
    let ids: Vec<_> = selected.values().collect();
    if ids.is_empty() || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(Error(
            "Nebula returned an invalid benchmark source selection".into(),
        ));
    }
    Ok(selected)
}

fn case_sources(
    case: &Case,
    benchmark: &Benchmark,
    sources: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    if benchmark.metric_kind != HOTPOTQA_EVALUATION {
        return Ok(sources.values().cloned().collect());
    }
    let reference = case
        .answer_reference
        .as_ref()
        .ok_or_else(|| Error("HotpotQA case is missing its answer reference".into()))?;
    if reference.answer.trim().is_empty()
        || !(2..=10).contains(&reference.candidate_document_ids.len())
        || reference
            .candidate_document_ids
            .iter()
            .collect::<HashSet<_>>()
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

fn validate_selection(scope: &Value, ids: &[String]) -> Result<()> {
    let selection = json!({"scope":scope, "sourceIds":ids});
    if serde_json::to_vec(&selection)?.len() > 64 * 1024 {
        return Err(Error(
            "Corpus selection exceeds Nebula's 64 KiB request limit".into(),
        ));
    }
    Ok(())
}

pub async fn prepare(
    config: &NebulaConfig,
    mut request: StartAnswerRunRequest,
    benchmark: &Benchmark,
    fingerprint: String,
) -> std::result::Result<AnswerRun, StartFailure> {
    request
        .validate()
        .map_err(|error| StartFailure::Invalid(error.to_string()))?;
    request.architecture_label = request.architecture_label.trim().to_owned();
    let automatic = benchmark.metric_kind == HOTPOTQA_EVALUATION;
    if !automatic && benchmark.metric_kind != "paired_context_recovery_v1" {
        return Err(StartFailure::Invalid(
            "Unsupported benchmark answer evaluation".into(),
        ));
    }
    let workspace = workspace(config)
        .await
        .map_err(|error| StartFailure::Unavailable(error.to_string()))?;
    let runtime = describe_runtime(&workspace);
    if !runtime.available {
        return Err(StartFailure::Unavailable(
            runtime.reason.unwrap_or_default(),
        ));
    }
    let profile = runtime
        .profiles
        .iter()
        .find(|profile| profile.id == request.profile_id && profile.enabled)
        .ok_or_else(|| {
            StartFailure::Invalid("Select an enabled Nebula generation profile".into())
        })?;
    let sources = selected_sources(&workspace, benchmark)
        .map_err(|error| StartFailure::Invalid(error.to_string()))?;
    for case in &benchmark.cases {
        let ids = case_sources(case, benchmark, &sources)
            .map_err(|error| StartFailure::Invalid(error.to_string()))?;
        validate_selection(&workspace.scope, &ids)
            .map_err(|error| StartFailure::Invalid(error.to_string()))?;
    }
    if !workspace.scope.is_object() || benchmark.cases.is_empty() {
        return Err(StartFailure::Invalid(
            "Benchmark or Nebula scope is empty".into(),
        ));
    }
    Ok(AnswerRun {
        summary: AnswerRunSummary {
            id: Uuid::new_v4(),
            generation_model: GenerationModel {
                profile_id: profile.id.clone(),
                label: profile.label.clone(),
            },
            request,
            benchmark_fingerprint: fingerprint,
            evaluation: if automatic {
                HOTPOTQA_EVALUATION
            } else {
                EVALUATION
            }
            .into(),
            embedding_model: runtime
                .embedding_model
                .expect("available runtime has model"),
            top_k: QUERY_TOP_K,
            max_in_flight: GENERATION_BATCH_SIZE,
            status: RunStatus::Running,
            started_at_ms: now_ms(),
            finished_at_ms: None,
            total: benchmark.cases.len(),
            completed: 0,
            failed: 0,
            answered: 0,
            reviewed: 0,
            means: None,
            automatic_scores: automatic.then(AnswerScores::default),
            scored: 0,
            error: None,
            scope: workspace.scope,
            watermark: workspace
                .status
                .watermark
                .expect("available runtime has watermark"),
            source_ids: sources.into_values().collect(),
        },
        cases: vec![],
    })
}

pub async fn execute(
    store: Arc<Store>,
    config: NebulaConfig,
    benchmark: Benchmark,
    mut run: AnswerRun,
) {
    match execute_inner(&store, &config, &benchmark, &mut run).await {
        Ok(()) => run.summary.status = RunStatus::Completed,
        Err(error) => {
            run.summary.status = RunStatus::Failed;
            run.summary.error = Some(error.to_string());
        }
    }
    run.summary.finished_at_ms = Some(now_ms());
    if let Err(error) = store.save_answer_run(&run) {
        eprintln!("Cannot persist answer run {}: {error}", run.summary.id);
    }
}

async fn execute_inner(
    store: &Store,
    config: &NebulaConfig,
    benchmark: &Benchmark,
    run: &mut AnswerRun,
) -> Result<()> {
    let workspace = workspace(config).await?;
    let runtime = describe_runtime(&workspace);
    let sources = selected_sources(&workspace, benchmark)?;
    if !runtime.available
        || runtime.embedding_model.as_ref() != Some(&run.summary.embedding_model)
        || workspace.scope != run.summary.scope
        || workspace.status.watermark.as_ref() != Some(&run.summary.watermark)
        || sources.values().cloned().collect::<Vec<_>>() != run.summary.source_ids
        || !runtime.profiles.iter().any(|profile| {
            profile.enabled
                && profile.id == run.summary.generation_model.profile_id
                && profile.label == run.summary.generation_model.label
        })
    {
        return Err(Error(
            "Nebula runtime changed before answer generation; start a fresh run".into(),
        ));
    }
    let revisions: HashMap<_, _> = workspace
        .sources
        .iter()
        .filter(|source| run.summary.source_ids.contains(&source.id))
        .map(|source| (source.id.clone(), source.revision.clone()))
        .collect();
    let context = Arc::new(CaseContext {
        client: client()?,
        config: config.clone(),
        benchmark: benchmark.clone(),
        summary: run.summary.clone(),
        sources,
        revisions,
        conversations: Mutex::new(HashSet::new()),
    });
    let mut finished = BTreeMap::new();
    for batch in benchmark.cases.chunks(GENERATION_BATCH_SIZE) {
        let mut tasks = tokio::task::JoinSet::new();
        for case in batch {
            let context = context.clone();
            let case = case.clone();
            let index = finished.len() + tasks.len();
            tasks.spawn(async move { (index, execute_case(&context, &case).await) });
        }
        let mut fatal = None;
        // Drain the entire batch before opening more conversations. This also prevents a
        // slow question from being evicted from Nebula's bounded conversation history.
        while let Some(result) = tasks.join_next().await {
            let (index, (captured, failure)) = match result {
                Ok(result) => result,
                Err(_) => {
                    fatal = Some(Error(
                        "Answer generation worker stopped unexpectedly".into(),
                    ));
                    continue;
                }
            };
            let log_result = if let Some(failure) = failure {
                if failure.fatal && fatal.is_none() {
                    fatal = Some(Error(failure.diagnostic.message.clone()));
                }
                run.summary.failed += 1;
                store.append_answer_failure(run.summary.id, &captured)
            } else {
                run.summary.completed += 1;
                if captured.outcome == "answered" {
                    run.summary.answered += 1;
                }
                Ok(())
            };
            finished.insert(index, captured);
            run.cases = finished.values().cloned().collect();
            // Completion order must not change floating-point scores or invent a winner
            // between identical runs. Recompute every partial aggregate in dataset order.
            if let Some(scores) = &mut run.summary.automatic_scores {
                *scores = AnswerScores::default();
                run.summary.scored = 0;
                for case in &run.cases {
                    record_scores(&mut run.summary, case);
                }
            }
            if let Err(error) = log_result {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err(error);
            }
            if let Err(error) = store.save_answer_run(run) {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err(error);
            }
        }
        if let Some(error) = fatal {
            return Err(error);
        }
    }
    if run.summary.failed > 0 {
        return Err(Error(format!(
            "Finished all {} questions with {} failed requests; inspect the failure log",
            run.summary.total, run.summary.failed
        )));
    }
    Ok(())
}

struct CaseContext {
    client: reqwest::Client,
    config: NebulaConfig,
    benchmark: Benchmark,
    summary: AnswerRunSummary,
    sources: BTreeMap<String, String>,
    revisions: HashMap<String, String>,
    conversations: Mutex<HashSet<String>>,
}

async fn execute_case(context: &CaseContext, case: &Case) -> (AnswerCase, Option<CaseFailure>) {
    let start = Instant::now();
    let mut captured = AnswerCase {
        case_id: case.id.clone(),
        query: case.query.clone(),
        status: "error".into(),
        outcome: "error".into(),
        answer: None,
        reason: None,
        evidence: vec![],
        lineage: vec![],
        model_receipt: None,
        latency_ms: 0,
        error: None,
        failure: None,
        review: None,
        reference_answer: (context.summary.evaluation == HOTPOTQA_EVALUATION)
            .then(|| {
                case.answer_reference
                    .as_ref()
                    .map(|reference| reference.answer.clone())
            })
            .flatten(),
        automatic_scores: (context.summary.evaluation == HOTPOTQA_EVALUATION)
            .then(AnswerScores::default),
    };
    let result: std::result::Result<(), CaseFailure> = async {
        let pinned = |error: Error| {
            CaseFailure::local(
                "validate_answer",
                "provenance_changed",
                error.to_string(),
                true,
            )
        };
        let source_ids =
            case_sources(case, &context.benchmark, &context.sources).map_err(pinned)?;
        validate_selection(&context.summary.scope, &source_ids).map_err(pinned)?;
        let selected_revisions = context
            .revisions
            .iter()
            .filter(|(id, _)| source_ids.contains(id))
            .map(|(id, revision)| (id.clone(), revision.clone()))
            .collect();
        let base = context.config.base_url.trim_end_matches('/');
        let conversation: Conversation = case_response(
            context
                .client
                .post(format!("{base}/conversations"))
                .bearer_auth(&context.config.token)
                .json(&json!({"scope":context.summary.scope,"sourceIds":source_ids})),
            "create_conversation",
        )
        .await?;
        if !conversation.source_scope.r#ref.is_object()
            || conversation.source_scope.r#ref["conversationId"] != conversation.id
            || !context
                .conversations
                .lock()
                .map_err(|_| pinned(Error("Conversation identities are unavailable".into())))?
                .insert(conversation.id.clone())
        {
            return Err(pinned(Error(
                "Nebula returned an invalid conversation identity".into(),
            )));
        }
        let generated: GeneratedAnswer = case_response(
            context
                .client
                .post(format!("{base}/query"))
                .bearer_auth(&context.config.token)
                .json(&json!({
                    "scope":context.summary.scope,
                    "conversationId":conversation.id,
                    "conversationScope":conversation.source_scope.r#ref,
                    "query":case.query,
                    "profileId":context.summary.generation_model.profile_id,
                    "strict":true,
                })),
            "generate_answer",
        )
        .await?;
        captured.outcome = if ["answered", "refused", "evidence-only", "not-ready"]
            .contains(&generated.outcome.as_str())
        {
            generated.outcome.clone()
        } else {
            "error".into()
        };
        captured.answer = generated.answer.clone();
        captured.reason = generated.reason.clone();
        captured.evidence = generated.evidence.clone();
        captured.lineage = generated.lineage.clone();
        captured.model_receipt = generated.model_receipt.clone();
        validate_answer(
            &generated,
            &conversation,
            &context.summary,
            &selected_revisions,
        )
        .map_err(pinned)?;
        Ok(())
    }
    .await;
    captured.latency_ms = start.elapsed().as_millis().min(u64::MAX as u128) as u64;
    match result {
        Ok(()) => {
            captured.status = "ok".into();
            if captured.outcome == "answered"
                && let Some(reference) = &captured.reference_answer
            {
                captured.automatic_scores = Some(score_answer(
                    captured.answer.as_deref().unwrap_or_default(),
                    reference,
                ));
            }
            (captured, None)
        }
        Err(failure) => {
            captured.error = Some(failure.diagnostic.message.clone());
            captured.failure = Some(failure.diagnostic.clone());
            (captured, Some(failure))
        }
    }
}

fn record_scores(summary: &mut AnswerRunSummary, case: &AnswerCase) {
    if let (Some(means), Some(scores)) = (&mut summary.automatic_scores, case.automatic_scores) {
        summary.scored += 1;
        means.exact_match =
            (means.exact_match + scores.exact_match / summary.total as f64).min(1.0);
        means.f1 = (means.f1 + scores.f1 / summary.total as f64).min(1.0);
    }
}

fn validate_answer(
    answer: &GeneratedAnswer,
    conversation: &Conversation,
    run: &AnswerRunSummary,
    revisions: &HashMap<String, String>,
) -> Result<()> {
    if answer.scope != run.scope
        || answer.watermark.as_ref() != Some(&run.watermark)
        || answer.conversation_scope != conversation.source_scope.r#ref
    {
        return Err(Error(
            "Nebula scope/index changed during answer generation".into(),
        ));
    }
    if answer.evidence.len() > QUERY_TOP_K
        || answer.evidence.iter().any(|item| {
            revisions.get(&item.source_id) != Some(&item.source_revision)
                || item.id.is_empty()
                || item.excerpt.is_empty()
        })
    {
        return Err(Error(
            "Nebula returned answer evidence outside the pinned benchmark corpus".into(),
        ));
    }
    if !["answered", "refused", "evidence-only"].contains(&answer.outcome.as_str()) {
        return Err(Error("Nebula did not return a ready answer outcome".into()));
    }
    if answer.outcome == "answered"
        && (answer
            .answer
            .as_ref()
            .is_none_or(|text| text.trim().is_empty())
            || answer.model_receipt.is_none())
    {
        return Err(Error(
            "Nebula returned an answer without text or model provenance".into(),
        ));
    }
    if answer.model_receipt.as_ref().is_some_and(|receipt| {
        receipt.profile_id != run.generation_model.profile_id
            || receipt.model_label != run.generation_model.label
            || receipt.route != "remote"
    }) {
        return Err(Error(
            "Nebula generation model changed during the run".into(),
        ));
    }
    let evidence_ids: HashSet<_> = answer.evidence.iter().map(|item| &item.id).collect();
    if evidence_ids.len() != answer.evidence.len()
        || answer.lineage.iter().any(|claim| {
            claim.evidence_ids.is_empty()
                || claim
                    .evidence_ids
                    .iter()
                    .any(|id| !evidence_ids.contains(id))
        })
    {
        return Err(Error(
            "Nebula returned an unresolved answer citation".into(),
        ));
    }
    Ok(())
}

impl AnswerRun {
    pub fn review(
        &mut self,
        case_id: &str,
        mut review: AnswerReviewRequest,
    ) -> std::result::Result<(), ReviewFailure> {
        if self.summary.status == RunStatus::Running {
            return Err(ReviewFailure::Running);
        }
        review
            .validate()
            .map_err(|error| ReviewFailure::Invalid(error.to_string()))?;
        review.reviewer = review.reviewer.trim().to_owned();
        let case = self
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
        let reviews = self
            .cases
            .iter()
            .filter(|case| case.status == "ok" && case.outcome == "answered")
            .filter_map(|case| case.review.as_ref())
            .collect::<Vec<_>>();
        self.summary.reviewed = reviews.len();
        self.summary.means = (!reviews.is_empty()).then(|| {
            let mean = |field: fn(&AnswerReviewRequest) -> bool| {
                reviews
                    .iter()
                    .filter(|review| field(&review.review))
                    .count() as f64
                    / reviews.len() as f64
            };
            AnswerMeans {
                correctness: mean(|review| review.correctness),
                groundedness: mean(|review| review.groundedness),
                hallucination_rate: mean(|review| review.hallucination),
                citation_accuracy: mean(|review| review.citation_accuracy),
            }
        });
        Ok(())
    }

    pub fn csv(&self) -> Result<Vec<u8>> {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(vec![]);
        writer.write_record([
            "run_id",
            "benchmark_id",
            "benchmark_fingerprint",
            "architecture_label",
            "embedding_model",
            "embedding_revision",
            "generation_profile",
            "generation_model",
            "evaluation",
            "top_k",
            "case_id",
            "query",
            "status",
            "outcome",
            "answer",
            "reason",
            "latency_ms",
            "evidence_json",
            "lineage_json",
            "model_receipt_json",
            "error",
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
            "max_in_flight",
            "failure_json",
        ])?;
        for case in &self.cases {
            let review = case.review.as_ref();
            let boolean = |value: fn(&AnswerReviewRequest) -> bool| {
                review
                    .map(|review| value(&review.review).to_string())
                    .unwrap_or_default()
            };
            writer.write_record([
                self.summary.id.to_string(),
                self.summary.request.benchmark_id.to_string(),
                self.summary.benchmark_fingerprint.clone(),
                self.summary.request.architecture_label.clone(),
                self.summary.embedding_model.id.clone(),
                self.summary.embedding_model.revision.clone(),
                self.summary.generation_model.profile_id.clone(),
                self.summary.generation_model.label.clone(),
                self.summary.evaluation.clone(),
                self.summary.top_k.to_string(),
                case.case_id.clone(),
                case.query.clone(),
                case.status.clone(),
                case.outcome.clone(),
                case.answer.clone().unwrap_or_default(),
                case.reason.clone().unwrap_or_default(),
                case.latency_ms.to_string(),
                serde_json::to_string(&case.evidence)?,
                serde_json::to_string(&case.lineage)?,
                serde_json::to_string(&case.model_receipt)?,
                case.error.clone().unwrap_or_default(),
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
                case.reference_answer.clone().unwrap_or_default(),
                case.automatic_scores
                    .map(|scores| scores.exact_match.to_string())
                    .unwrap_or_default(),
                case.automatic_scores
                    .map(|scores| scores.f1.to_string())
                    .unwrap_or_default(),
                match self.summary.status {
                    RunStatus::Running => "running",
                    RunStatus::Completed => "completed",
                    RunStatus::Failed => "failed",
                    RunStatus::Interrupted => "interrupted",
                }
                .to_owned(),
                self.summary.total.to_string(),
                self.summary.scored.to_string(),
                self.summary
                    .automatic_scores
                    .map(|scores| scores.exact_match.to_string())
                    .unwrap_or_default(),
                self.summary
                    .automatic_scores
                    .map(|scores| scores.f1.to_string())
                    .unwrap_or_default(),
                self.summary.max_in_flight.to_string(),
                case.failure
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?
                    .unwrap_or_default(),
            ])?;
        }
        writer.flush()?;
        writer
            .into_inner()
            .map_err(|error| Error(error.to_string()))
    }
}
