use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    Error, Result,
    analysis::{Scores, score_context},
    config::NebulaConfig,
    load_benchmarks::Benchmark,
    storage::Store,
};

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
    pub means: Option<Scores>,
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

impl Run {
    pub fn new(request: RunRequest, benchmark: &Benchmark, fingerprint: String) -> Result<Self> {
        if !(1..=100).contains(&request.top_k) || request.label.len() > 256 {
            return Err(Error(
                "top_k must be 1–100 and label at most 256 bytes".into(),
            ));
        }
        Ok(Self {
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
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

pub async fn execute(store: Arc<Store>, config: NebulaConfig, benchmark: Benchmark, mut run: Run) {
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
            run.means = totals.mean(run.completed);
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
