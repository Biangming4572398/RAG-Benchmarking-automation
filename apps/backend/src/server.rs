//! HTTP assembly and shared persistence/Nebula execution infrastructure.
use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use tokio::sync::Semaphore;
use uuid::Uuid;

pub use crate::server::persistence::{BenchmarkInfo, Store};
use crate::{
    AnswerReviewRequest, Error, Result, ReviewFailure, RunStatus, StartAnswerRunRequest,
    StartFailure, StartRunRequest,
    config::{Catalog, LoadRequest, NebulaConfig},
    initialize_benchmark, module_for_snapshot,
};

struct AppState {
    store: Arc<Store>,
    nebula: Option<NebulaConfig>,
    nebula_corpus: Option<PathBuf>,
    catalog: Catalog,
    run_slot: Arc<Semaphore>,
    load_slot: Arc<Semaphore>,
}

pub fn router(store: Arc<Store>, nebula: Option<NebulaConfig>, catalog: Catalog) -> Result<Router> {
    router_with_corpus(store, nebula, catalog, None)
}

pub fn router_with_corpus(
    store: Arc<Store>,
    nebula: Option<NebulaConfig>,
    catalog: Catalog,
    nebula_corpus: Option<PathBuf>,
) -> Result<Router> {
    if let Some(config) = &nebula {
        config.validate()?;
    }
    let state = Arc::new(AppState {
        store,
        nebula,
        nebula_corpus,
        catalog,
        run_slot: Arc::new(Semaphore::new(1)),
        load_slot: Arc::new(Semaphore::new(1)),
    });
    Ok(Router::new()
        .route(
            "/api/benchmarks/v1/health",
            get(|| async { Json(json!({"status": "ok"})) }),
        )
        .route(
            "/api/benchmarks/v1/benchmarks",
            get(list_benchmarks).post(load),
        )
        .route("/api/benchmarks/v1/benchmarks/{id}", get(benchmark))
        .route("/api/benchmarks/v1/catalog", get(get_catalog))
        .route("/api/benchmarks/v1/results", get(result_tables))
        .route(
            "/api/benchmarks/v1/results/{benchmark}/scores.csv",
            get(result_csv),
        )
        .route(
            "/api/benchmarks/v1/suite-runs",
            get(list_suites).post(start_suite),
        )
        .route("/api/benchmarks/v1/suite-runs/{id}", get(suite))
        .route("/api/benchmarks/v1/runs", get(list_runs).post(start_run))
        .route("/api/benchmarks/v1/runs/{id}", get(run))
        .route("/api/benchmarks/v1/runs/{id}/scores.csv", get(scores))
        .route("/api/benchmarks/v1/answer-runtime", get(answer_runtime))
        .route(
            "/api/benchmarks/v1/answer-runs",
            get(list_answer_runs).post(start_answer_run),
        )
        .route("/api/benchmarks/v1/answer-runs/{id}", get(answer_run))
        .route(
            "/api/benchmarks/v1/answer-runs/{id}/scores.csv",
            get(answer_scores),
        )
        .route(
            "/api/benchmarks/v1/answer-runs/{id}/cases/{case_id}/review",
            axum::routing::post(review_answer),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state))
}

type ApiResult<T> = std::result::Result<T, ApiError>;
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

async fn list_benchmarks(State(state): State<Arc<AppState>>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.benchmarks()?))
}

async fn get_catalog(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let module_keys: std::collections::BTreeMap<_, _> = state
        .catalog
        .benchmarks
        .iter()
        .filter_map(|(key, definition)| {
            crate::module_for_definition(Some(key), definition)
                .ok()
                .map(|module| (key, module.key()))
        })
        .collect();
    Json(json!({ "benchmarks": state.catalog.benchmarks, "module_keys": module_keys }))
}

async fn load(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoadRequest>,
) -> ApiResult<impl IntoResponse> {
    let request = state
        .catalog
        .resolve(&request)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let permit = state.load_slot.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A benchmark is already loading".into(),
        )
    })?;
    let info = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let benchmark = initialize_benchmark(&request)
            .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
        state
            .store
            .save_benchmark(&benchmark)
            .map_err(ApiError::from)
    })
    .await
    .map_err(|_| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Benchmark loading failed".into(),
        )
    })??;
    Ok((StatusCode::CREATED, Json(info)))
}

async fn benchmark(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.benchmark(id).map_err(|_| {
        ApiError(StatusCode::NOT_FOUND, "Benchmark not found".into())
    })?))
}

async fn result_tables(State(state): State<Arc<AppState>>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.result_tables()?))
}

async fn result_csv(
    State(state): State<Arc<AppState>>,
    Path(benchmark): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let bytes = state.store.result_csv(&benchmark).map_err(|_| {
        ApiError(
            StatusCode::NOT_FOUND,
            "Benchmark result table not found".into(),
        )
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{benchmark}.csv\""),
            ),
        ],
        bytes,
    ))
}

async fn list_suites(State(state): State<Arc<AppState>>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.suites()?))
}

async fn suite(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.suite(id).map_err(|_| {
        ApiError(StatusCode::NOT_FOUND, "Suite run not found".into())
    })?))
}

async fn start_suite(
    State(state): State<Arc<AppState>>,
    Json(request): Json<crate::suite::StartSuiteRequest>,
) -> ApiResult<impl IntoResponse> {
    request
        .validate()
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let config = state.nebula.clone().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Configure a Nebula backend before starting benchmark runs".into(),
        )
    })?;
    let permit = state.run_slot.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A benchmark run is already active".into(),
        )
    })?;
    let mut suite = crate::suite::plan(&state.store, &state.catalog, request)?;
    let load_permit = state.load_slot.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A benchmark is already loading".into(),
        )
    })?;
    state.store.create_suite(&mut suite)?;
    let accepted = suite.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let id = suite.id;
        let worker_store = state.store.clone();
        let corpus = state.nebula_corpus.clone();
        let outcome = tokio::spawn(async move {
            let needs_preparation = suite.preparations.iter().any(|item| {
                item.benchmark_id.is_none() && item.status == crate::suite::SuiteItemStatus::Queued
            });
            let benchmarks = crate::suite::prepare(worker_store.clone(), &mut suite).await?;
            drop(load_permit);
            if !benchmarks.is_empty() {
                if let Some(corpus) = corpus {
                    suite.phase = crate::suite::SuitePhase::Indexing;
                    worker_store.save_suite(&suite)?;
                    crate::corpus::prepare(corpus, config.clone(), benchmarks).await?;
                } else if needs_preparation {
                    return Err(Error("Automatic indexing requires BENCHMARK_NEBULA_CORPUS pointing to Nebula's dedicated benchmark corpus. Start the dashboard through the managed launcher, or configure that directory and enable Nebula's -benchmark-reindex option.".into()));
                }
            }
            crate::suite::execute(worker_store, config, suite).await
        }).await;
        let error = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error.to_string(),
            Err(_) => "Suite worker stopped unexpectedly".into(),
        };
        if let Ok(mut failed) = state.store.suite(id) {
            failed.status = RunStatus::Failed;
            failed.phase = crate::suite::SuitePhase::Finished;
            failed.error = Some(error.clone());
            failed.finished_at_ms = Some(crate::now_ms());
            for item in &mut failed.items {
                if matches!(
                    item.status,
                    crate::suite::SuiteItemStatus::Queued | crate::suite::SuiteItemStatus::Running
                ) {
                    item.status = crate::suite::SuiteItemStatus::Failed;
                    item.reason = Some(error.clone());
                }
            }
            for preparation in &mut failed.preparations {
                if matches!(
                    preparation.status,
                    crate::suite::SuiteItemStatus::Queued | crate::suite::SuiteItemStatus::Running
                ) {
                    preparation.status = crate::suite::SuiteItemStatus::Failed;
                    preparation.reason = Some(error.clone());
                }
            }
            if let Err(error) = state.store.save_suite(&failed) {
                eprintln!("Cannot persist failed suite {id}: {error}");
            }
        }
    });
    Ok((StatusCode::ACCEPTED, Json(accepted)))
}

async fn list_runs(State(state): State<Arc<AppState>>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.runs()?))
}

async fn run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.run(id).map_err(|_| {
        ApiError(StatusCode::NOT_FOUND, "Run not found".into())
    })?))
}

async fn start_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartRunRequest>,
) -> ApiResult<impl IntoResponse> {
    let config = state.nebula.clone().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Configure NEBULA_API_BASE and NEBULA_API_TOKEN before starting runs".into(),
        )
    })?;
    let benchmark = state
        .store
        .benchmark(request.benchmark_id)
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Benchmark not found".into()))?;
    let fingerprint = state.store.benchmark_info(benchmark.id)?.fingerprint;
    let module = module_for_snapshot(&benchmark)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let run = module
        .prepare_retrieval(request.resolve(&benchmark), &benchmark, fingerprint)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let permit = state.run_slot.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A benchmark run is already active".into(),
        )
    })?;
    state.store.create_run(&run)?;
    let job_run = run.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let id = job_run.id;
        let task =
            tokio::spawn(module.run_retrieval(state.store.clone(), config, benchmark, job_run));
        if task.await.is_err()
            && let Ok(mut failed) = state.store.run(id)
        {
            failed.status = RunStatus::Failed;
            failed.error = Some("Benchmark worker stopped unexpectedly".into());
            failed.finished_at_ms = Some(crate::now_ms());
            let _ = state.store.save_run(&failed);
        }
    });
    Ok((StatusCode::ACCEPTED, Json(run)))
}

async fn scores(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let run = state
        .store
        .run(id)
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Run not found".into()))?;
    if run.status == RunStatus::Running {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "CSV is available after the run stops".into(),
        ));
    }
    let bytes = state.store.scores(id).map_err(|_| {
        ApiError(
            StatusCode::NOT_FOUND,
            "This run produced no CSV rows".into(),
        )
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{id}.csv\""),
            ),
        ],
        bytes,
    ))
}

async fn answer_runtime(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(generation::runtime(state.nebula.as_ref()).await)
}

async fn list_answer_runs(State(state): State<Arc<AppState>>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.answer_runs()?))
}

async fn answer_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.store.answer_run(id).map_err(|_| {
        ApiError(StatusCode::NOT_FOUND, "Answer run not found".into())
    })?))
}

async fn start_answer_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartAnswerRunRequest>,
) -> ApiResult<impl IntoResponse> {
    request
        .validate()
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let config = state.nebula.clone().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Configure a Nebula backend before generating answers".into(),
        )
    })?;
    let benchmark = state
        .store
        .benchmark(request.benchmark_id)
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Benchmark not found".into()))?;
    let fingerprint = state.store.benchmark_info(benchmark.id)?.fingerprint;
    let permit = state.run_slot.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A benchmark run is already active".into(),
        )
    })?;
    let module = module_for_snapshot(&benchmark)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let run = module
        .prepare_answers(&config, request, &benchmark, fingerprint)
        .await
        .map_err(|error| match error {
            StartFailure::Unavailable(message) => {
                ApiError(StatusCode::SERVICE_UNAVAILABLE, message)
            }
            StartFailure::Invalid(message) => ApiError(StatusCode::BAD_REQUEST, message),
        })?;
    state.store.create_answer_run(&run)?;
    let summary = run.summary.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let id = run.summary.id;
        let task = tokio::spawn(module.run_answers(state.store.clone(), config, benchmark, run));
        if task.await.is_err()
            && let Ok(mut failed) = state.store.answer_run(id)
        {
            failed.summary.status = RunStatus::Failed;
            failed.summary.error = Some("Answer generation worker stopped unexpectedly".into());
            failed.summary.finished_at_ms = Some(crate::now_ms());
            let _ = state.store.save_answer_run(&failed);
        }
    });
    Ok((StatusCode::ACCEPTED, Json(summary)))
}

async fn review_answer(
    State(state): State<Arc<AppState>>,
    Path((id, case_id)): Path<(Uuid, String)>,
    Json(review): Json<AnswerReviewRequest>,
) -> ApiResult<impl IntoResponse> {
    let run = state
        .store
        .review_answer(id, &case_id, review)
        .map_err(|error| match error {
            ReviewFailure::NotFound => {
                ApiError(StatusCode::NOT_FOUND, "Answer run or case not found".into())
            }
            ReviewFailure::Running => ApiError(
                StatusCode::CONFLICT,
                "Reviews are available after the run stops".into(),
            ),
            ReviewFailure::Invalid(message) => ApiError(StatusCode::BAD_REQUEST, message),
            ReviewFailure::Storage(error) => ApiError::from(error),
        })?;
    Ok(Json(run))
}

async fn answer_scores(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let run = state
        .store
        .answer_run(id)
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Answer run not found".into()))?;
    if run.summary.status == RunStatus::Running {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "CSV is available after the run stops".into(),
        ));
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"answer-{id}.csv\""),
            ),
        ],
        state.store.answer_csv(&run)?,
    ))
}

mod persistence {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs::{self, File, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use serde::{Deserialize, Serialize, de::DeserializeOwned};
    use uuid::Uuid;

    use crate::{
        AnswerCase, AnswerReviewRequest, AnswerRun, AnswerRunSummary, Benchmark, BenchmarkModule,
        Error, MetricValues, Result, ReviewFailure, Run, RunStatus,
        config::ResolvedBenchmark,
        digest, module_for_answer_run, module_for_snapshot,
        suite::{SuiteItemStatus, SuiteMode, SuiteRun},
    };

    pub struct Store {
        root: PathBuf,
        // Prevent two servers from racing recovery/progress writes in the same root.
        _lock: File,
        answer_review_lock: Mutex<()>,
        // The process owns this directory exclusively. Polling summaries must not reread every
        // generated answer and evidence history; run.json retains the full answer record.
        answer_summaries: Mutex<BTreeMap<Uuid, AnswerRunSummary>>,
        results: Mutex<ResultTables>,
    }

    #[derive(Deserialize, Serialize)]
    pub struct BenchmarkInfo {
        pub id: Uuid,
        pub source: String,
        pub split: String,
        pub metric_kind: String,
        #[serde(default)]
        pub module_key: Option<String>,
        pub case_count: usize,
        pub document_count: usize,
        pub corpus_path: PathBuf,
        pub fingerprint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub configuration: Option<ResolvedBenchmark>,
    }

    impl Store {
        pub fn open(root: &Path) -> Result<Self> {
            fs::create_dir_all(root)?;
            let root = root.canonicalize()?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join(".server.lock"))?;
            lock.try_lock()
                .map_err(|_| Error("Benchmark data directory is already in use".into()))?;
            fs::create_dir_all(root.join("benchmarks"))?;
            fs::create_dir_all(root.join("runs"))?;
            fs::create_dir_all(root.join("answer-runs"))?;
            fs::create_dir_all(root.join("results"))?;
            let results = ResultTables::open(&root.join("results"))?;
            let store = Self {
                root,
                _lock: lock,
                answer_review_lock: Mutex::new(()),
                answer_summaries: Mutex::new(BTreeMap::new()),
                results: Mutex::new(results),
            };
            let mut rows = Vec::new();
            for mut run in store.runs()? {
                if run.status == RunStatus::Running {
                    run.status = RunStatus::Interrupted;
                    run.error = Some(
                        "Server stopped before the run completed; partial CSV is retained".into(),
                    );
                    run.finished_at_ms = Some(crate::now_ms());
                    atomic_json(&store.run_path(run.id).join("run.json"), &run)?;
                }
                rows.push(ResultRow::retrieval(
                    &run,
                    store.result_benchmark(
                        run.request.benchmark_id,
                        &run.metric_kind,
                        RunMode::Retrieval,
                    )?,
                )?);
            }
            for id in ids_in(&store.root.join("answer-runs"))? {
                let mut run = store.answer_run(id)?;
                if run.summary.status == RunStatus::Running {
                    run.summary.status = RunStatus::Interrupted;
                    run.summary.error = Some("Server stopped before answer generation completed; partial answers are retained".into());
                    run.summary.finished_at_ms = Some(crate::now_ms());
                    atomic_json(&store.answer_run_path(id), &run)?;
                }
                rows.push(ResultRow::generation(
                    &run.summary,
                    store.result_benchmark(
                        run.summary.request.benchmark_id,
                        &run.summary.evaluation,
                        RunMode::Generation,
                    )?,
                )?);
                store
                    .answer_summaries
                    .lock()
                    .map_err(|_| Error("Answer summaries are unavailable".into()))?
                    .insert(id, run.summary);
            }
            rows.sort_by_key(|row| (row.started_at_ms, row.reference.run_id));
            let mut results = store
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            for row in rows {
                results.upsert(row)?;
            }
            // Reconcile queued and active suites after child executions have been recovered.
            for suite in results.registry.suite_runs.values_mut() {
                if suite.status == RunStatus::Running {
                    for preparation in &mut suite.preparations {
                        if matches!(
                            preparation.status,
                            SuiteItemStatus::Queued | SuiteItemStatus::Running
                        ) {
                            preparation.status = SuiteItemStatus::Interrupted;
                            preparation.reason =
                                Some("Server stopped during benchmark preparation".into());
                        }
                    }
                    for item in &mut suite.items {
                        if matches!(
                            item.status,
                            SuiteItemStatus::Queued | SuiteItemStatus::Running
                        ) {
                            let child = item.run_id.and_then(|id| match item.mode {
                                Some(SuiteMode::Retrieval) => {
                                    store.run(id).ok().map(|run| (run.status, run.error))
                                }
                                Some(SuiteMode::Generation) => store
                                    .answer_run(id)
                                    .ok()
                                    .map(|run| (run.summary.status, run.summary.error)),
                                None => None,
                            });
                            let (status, reason) = child.unwrap_or((
                                RunStatus::Interrupted,
                                Some("Server stopped before this execution began".into()),
                            ));
                            item.status = status.into();
                            item.reason = reason;
                        }
                    }
                    suite.status = RunStatus::Interrupted;
                    suite.phase = crate::suite::SuitePhase::Finished;
                    suite.finished_at_ms = Some(crate::now_ms());
                    suite.error = Some(
                        "Server stopped before the suite completed; saved results are retained"
                            .into(),
                    );
                }
            }
            results.save_all(&store.root.join("results"))?;
            drop(results);
            Ok(store)
        }

        pub fn save_benchmark(&self, benchmark: &Benchmark) -> Result<BenchmarkInfo> {
            let parent = self.root.join("benchmarks");
            let stage = tempfile::tempdir_in(&parent)?;
            let corpus = stage.path().join("corpus");
            fs::create_dir(&corpus)?;
            for document in &benchmark.documents {
                fs::write(corpus.join(&document.filename), &document.text)?;
            }
            atomic_json(&stage.path().join("benchmark.json"), benchmark)?;
            fs::rename(stage.path(), parent.join(benchmark.id.to_string()))?;
            self.benchmark_info(benchmark.id)
        }

        pub fn benchmark(&self, id: Uuid) -> Result<Benchmark> {
            read_json(&self.benchmark_path(id).join("benchmark.json"))
        }

        pub fn benchmark_info(&self, id: Uuid) -> Result<BenchmarkInfo> {
            let benchmark = self.benchmark(id)?;
            Ok(BenchmarkInfo {
                id,
                module_key: module_for_snapshot(&benchmark)
                    .ok()
                    .map(|module| module.key().to_owned()),
                fingerprint: digest(&serde_json::to_vec(&(
                    &benchmark.metric_kind,
                    &benchmark.cases,
                    &benchmark.documents,
                ))?),
                case_count: benchmark.cases.len(),
                document_count: benchmark.documents.len(),
                corpus_path: self.benchmark_path(id).join("corpus"),
                source: benchmark.source,
                split: benchmark.split,
                metric_kind: benchmark.metric_kind,
                configuration: benchmark.configuration,
            })
        }

        pub fn benchmarks(&self) -> Result<Vec<BenchmarkInfo>> {
            ids_in(&self.root.join("benchmarks"))?
                .into_iter()
                .map(|id| self.benchmark_info(id))
                .collect()
        }

        /// Oldest first; filesystem save time also supports snapshots written before this API.
        pub fn benchmarks_by_saved_time(&self) -> Result<Vec<Benchmark>> {
            let mut snapshots = Vec::new();
            for id in ids_in(&self.root.join("benchmarks"))? {
                let saved =
                    fs::metadata(self.benchmark_path(id).join("benchmark.json"))?.modified()?;
                snapshots.push((saved, id, self.benchmark(id)?));
            }
            snapshots.sort_by_key(|(saved, id, _)| (*saved, *id));
            Ok(snapshots
                .into_iter()
                .map(|(_, _, benchmark)| benchmark)
                .collect())
        }

        pub fn create_suite(&self, suite: &mut SuiteRun) -> Result<()> {
            let mut results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            if suite.run_number != 0
                || results
                    .registry
                    .suite_runs
                    .values()
                    .any(|existing| existing.id == suite.id)
            {
                return Err(Error("Suite already exists".into()));
            }
            let number = results.registry.next_run_number;
            results.registry.next_run_number = number
                .checked_add(1)
                .ok_or_else(|| Error("Run numbers are exhausted".into()))?;
            suite.run_number = number;
            results.registry.suite_runs.insert(number, suite.clone());
            atomic_json(&self.root.join("results/runs.json"), &results.registry)
        }

        pub fn save_suite(&self, suite: &SuiteRun) -> Result<()> {
            let mut results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            let existing = results
                .registry
                .suite_runs
                .get_mut(&suite.run_number)
                .filter(|existing| existing.id == suite.id)
                .ok_or_else(|| Error("Suite has not been created".into()))?;
            let mut saved = suite.clone();
            saved
                .request
                .description
                .clone_from(&existing.request.description);
            *existing = saved;
            atomic_json(&self.root.join("results/runs.json"), &results.registry)
        }

        pub fn suites(&self) -> Result<Vec<SuiteRun>> {
            let results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            Ok(results
                .registry
                .suite_runs
                .values()
                .rev()
                .cloned()
                .collect())
        }

        pub fn suite(&self, id: Uuid) -> Result<SuiteRun> {
            self.suites()?
                .into_iter()
                .find(|suite| suite.id == id)
                .ok_or_else(|| Error("Suite not found".into()))
        }

        pub fn result_tables(&self) -> Result<ComparisonResults> {
            let results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            let keys: BTreeSet<_> = results
                .registry
                .runs
                .values()
                .map(|reference| reference.benchmark.as_str())
                .collect();
            Ok(ComparisonResults {
                benchmarks: keys.into_iter().map(|key| results.table(key)).collect(),
            })
        }

        pub fn result_csv(&self, benchmark: &str) -> Result<Vec<u8>> {
            validate_result_name(benchmark)?;
            let results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            if !results
                .registry
                .runs
                .values()
                .any(|reference| reference.benchmark == benchmark)
            {
                return Err(Error("Benchmark result table not found".into()));
            }
            results.table(benchmark).csv()
        }

        pub fn create_run(&self, run: &Run) -> Result<()> {
            let mut results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            let row = ResultRow::retrieval(
                run,
                self.result_benchmark(
                    run.request.benchmark_id,
                    &run.metric_kind,
                    RunMode::Retrieval,
                )?,
            )?;
            results.check_new(run.id)?;
            let stage = tempfile::tempdir_in(self.root.join("runs"))?;
            atomic_json(&stage.path().join("run.json"), run)?;
            fs::rename(stage.path(), self.run_path(run.id))?;
            results.save_row(&self.root.join("results"), row)
        }

        pub fn save_run(&self, run: &Run) -> Result<()> {
            let mut results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            let benchmark = results.benchmark(run.id)?;
            atomic_json(&self.run_path(run.id).join("run.json"), run)?;
            results.save_row(
                &self.root.join("results"),
                ResultRow::retrieval(run, benchmark)?,
            )
        }

        pub fn run(&self, id: Uuid) -> Result<Run> {
            read_json(&self.run_path(id).join("run.json"))
        }

        pub fn runs(&self) -> Result<Vec<Run>> {
            ids_in(&self.root.join("runs"))?
                .into_iter()
                .map(|id| self.run(id))
                .collect()
        }

        pub fn create_scores(&self, id: Uuid) -> Result<File> {
            Ok(OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.scores_path(id))?)
        }

        pub fn scores(&self, id: Uuid) -> Result<Vec<u8>> {
            Ok(fs::read(self.scores_path(id))?)
        }

        pub fn create_answer_run(&self, run: &AnswerRun) -> Result<()> {
            let mut summaries = self
                .answer_summaries
                .lock()
                .map_err(|_| Error("Answer summaries are unavailable".into()))?;
            let mut results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            let row = ResultRow::generation(
                &run.summary,
                self.result_benchmark(
                    run.summary.request.benchmark_id,
                    &run.summary.evaluation,
                    RunMode::Generation,
                )?,
            )?;
            results.check_new(run.summary.id)?;
            let parent = self.root.join("answer-runs");
            let stage = tempfile::tempdir_in(&parent)?;
            atomic_json(&stage.path().join("run.json"), run)?;
            fs::rename(stage.path(), parent.join(run.summary.id.to_string()))?;
            summaries.insert(run.summary.id, run.summary.clone());
            results.save_row(&self.root.join("results"), row)
        }

        pub fn save_answer_run(&self, run: &AnswerRun) -> Result<()> {
            let mut summaries = self
                .answer_summaries
                .lock()
                .map_err(|_| Error("Answer summaries are unavailable".into()))?;
            let mut results = self
                .results
                .lock()
                .map_err(|_| Error("Result tables are unavailable".into()))?;
            let benchmark = results.benchmark(run.summary.id)?;
            atomic_json(&self.answer_run_path(run.summary.id), run)?;
            summaries.insert(run.summary.id, run.summary.clone());
            results.save_row(
                &self.root.join("results"),
                ResultRow::generation(&run.summary, benchmark)?,
            )
        }

        fn result_benchmark(&self, id: Uuid, evaluation: &str, mode: RunMode) -> Result<String> {
            let matches = |module: &&dyn BenchmarkModule| match mode {
                RunMode::Retrieval => module.metric_kind() == evaluation,
                RunMode::Generation => module
                    .answer_evaluation()
                    .is_some_and(|value| value.id() == evaluation),
            };
            let name = match fs::symlink_metadata(self.benchmark_path(id)) {
                Ok(_) => {
                    let benchmark = self.benchmark(id)?;
                    if benchmark.id != id {
                        return Err(Error("Result snapshot has a different benchmark ID".into()));
                    }
                    if benchmark.configuration.is_some()
                        || crate::BENCHMARKS
                            .iter()
                            .any(|module| module.metric_kind() == benchmark.metric_kind)
                    {
                        let module = module_for_snapshot(&benchmark)?;
                        if !matches(&module) {
                            return Err(Error(
                                "Result evaluation does not match its benchmark snapshot".into(),
                            ));
                        }
                        module.key().to_owned()
                    } else {
                        // Direct users of the shared executor can supply an independent evaluator.
                        benchmark.metric_kind
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut modules = crate::BENCHMARKS.iter().copied().filter(matches);
                    match (modules.next(), modules.next()) {
                        (Some(module), None) => module.key().to_owned(),
                        (None, _) => evaluation.to_owned(),
                        _ => {
                            return Err(Error(
                                "Result needs its snapshot to identify the benchmark".into(),
                            ));
                        }
                    }
                }
                Err(error) => return Err(error.into()),
            };
            validate_result_name(&name)?;
            Ok(name)
        }

        pub fn append_answer_failure(&self, id: Uuid, case: &AnswerCase) -> Result<()> {
            let Some(failure) = &case.failure else {
                return Ok(());
            };
            let mut event = serde_json::to_value(failure)?;
            let fields = event
                .as_object_mut()
                .expect("failure serializes as an object");
            fields.insert("run_id".into(), serde_json::to_value(id)?);
            fields.insert("case_id".into(), serde_json::to_value(&case.case_id)?);
            fields.insert("latency_ms".into(), serde_json::to_value(case.latency_ms)?);
            let path = self.answer_run_path(id).with_file_name("failures.jsonl");
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, &event)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(())
        }

        pub fn answer_run(&self, id: Uuid) -> Result<AnswerRun> {
            read_json(&self.answer_run_path(id))
        }

        pub fn answer_runs(&self) -> Result<Vec<AnswerRunSummary>> {
            Ok(self
                .answer_summaries
                .lock()
                .map_err(|_| Error("Answer summaries are unavailable".into()))?
                .values()
                .cloned()
                .collect())
        }

        fn answer_module(&self, run: &AnswerRun) -> Result<&'static dyn BenchmarkModule> {
            let id = run.summary.request.benchmark_id;
            match fs::symlink_metadata(self.benchmark_path(id)) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // Standalone historical exports may omit their snapshot. Never guess
                    // when several benchmark modules reuse the same evaluator.
                    return module_for_answer_run(run);
                }
                Err(error) => return Err(error.into()),
            }
            // An existing but incomplete/corrupt snapshot must not fall back to an
            // evaluator-only match: the persisted benchmark is the run's identity.
            let benchmark = self.benchmark(id)?;
            if benchmark.id != id {
                return Err(Error(
                    "Answer run snapshot has a different benchmark ID".into(),
                ));
            }
            let module = module_for_snapshot(&benchmark)?;
            if !module
                .answer_evaluation()
                .is_some_and(|evaluation| evaluation.id() == run.summary.evaluation)
            {
                return Err(Error(
                    "Answer run evaluation does not match its benchmark snapshot".into(),
                ));
            }
            Ok(module)
        }

        pub fn answer_csv(&self, run: &AnswerRun) -> Result<Vec<u8>> {
            let evaluation = self
                .answer_module(run)?
                .answer_evaluation()
                .ok_or_else(|| Error("Answer run has no registered evaluator".into()))?;
            crate::server::generation::csv(run, evaluation)
        }

        pub fn review_answer(
            &self,
            id: Uuid,
            case_id: &str,
            review: AnswerReviewRequest,
        ) -> std::result::Result<AnswerRun, ReviewFailure> {
            // Terminal-only reviews cannot race generation. Serialize concurrent reviewers so each
            // read/modify/atomic-write retains the other case's latest saved review.
            let _guard = self.answer_review_lock.lock().map_err(|_| {
                ReviewFailure::Storage(Error("Answer review storage is unavailable".into()))
            })?;
            let mut run = self.answer_run(id).map_err(|_| ReviewFailure::NotFound)?;
            self.answer_module(&run)
                .map_err(|error| ReviewFailure::Invalid(error.to_string()))?
                .review_answer(&mut run, case_id, review)?;
            self.save_answer_run(&run).map_err(ReviewFailure::Storage)?;
            Ok(run)
        }

        fn answer_run_path(&self, id: Uuid) -> PathBuf {
            self.root
                .join("answer-runs")
                .join(id.to_string())
                .join("run.json")
        }

        fn benchmark_path(&self, id: Uuid) -> PathBuf {
            self.root.join("benchmarks").join(id.to_string())
        }
        fn run_path(&self, id: Uuid) -> PathBuf {
            self.root.join("runs").join(id.to_string())
        }
        fn scores_path(&self, id: Uuid) -> PathBuf {
            self.run_path(id).join("scores.csv")
        }
    }

    #[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    enum RunMode {
        Retrieval,
        Generation,
    }

    #[derive(Clone, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RunReference {
        run_id: Uuid,
        benchmark: String,
        mode: RunMode,
        architecture_name: String,
        description: String,
    }

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RunRegistry {
        next_run_number: u64,
        #[serde(deserialize_with = "unique_run_numbers")]
        runs: BTreeMap<u64, RunReference>,
        #[serde(
            default,
            skip_serializing_if = "BTreeMap::is_empty",
            deserialize_with = "unique_suite_numbers"
        )]
        suite_runs: BTreeMap<u64, SuiteRun>,
    }

    fn unique_run_numbers<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<BTreeMap<u64, RunReference>, D::Error> {
        struct References;
        impl<'de> serde::de::Visitor<'de> for References {
            type Value = BTreeMap<u64, RunReference>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map of unique run numbers to references")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut references = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, RunReference>()? {
                    let number = key.parse::<u64>().map_err(serde::de::Error::custom)?;
                    if references.insert(number, value).is_some() {
                        return Err(serde::de::Error::custom(
                            "Duplicate run number in results/runs.json",
                        ));
                    }
                }
                Ok(references)
            }
        }
        deserializer.deserialize_map(References)
    }

    fn unique_suite_numbers<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<BTreeMap<u64, SuiteRun>, D::Error> {
        struct Suites;
        impl<'de> serde::de::Visitor<'de> for Suites {
            type Value = BTreeMap<u64, SuiteRun>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map of unique suite run numbers")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut suites = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, SuiteRun>()? {
                    let number = key.parse::<u64>().map_err(serde::de::Error::custom)?;
                    if suites.insert(number, value).is_some() {
                        return Err(serde::de::Error::custom(
                            "Duplicate suite number in results/runs.json",
                        ));
                    }
                }
                Ok(suites)
            }
        }
        deserializer.deserialize_map(Suites)
    }

    #[derive(Serialize)]
    pub struct ComparisonResults {
        pub benchmarks: Vec<ComparisonTable>,
    }

    #[derive(Serialize)]
    pub struct ComparisonTable {
        pub benchmark: String,
        pub columns: Vec<String>,
        pub rows: Vec<BTreeMap<String, String>>,
    }

    impl ComparisonTable {
        fn csv(&self) -> Result<Vec<u8>> {
            let mut csv = csv::Writer::from_writer(Vec::new());
            csv.write_record(&self.columns)?;
            for row in &self.rows {
                csv.write_record(
                    self.columns
                        .iter()
                        .map(|column| row.get(column).map(String::as_str).unwrap_or("")),
                )?;
            }
            csv.into_inner().map_err(|error| Error(error.to_string()))
        }
    }

    // CSVs are rebuildable projections. The registry owns human run numbers/descriptions;
    // the existing per-run JSON owns the scores and detailed evidence.
    struct ResultTables {
        registry: RunRegistry,
        rows: BTreeMap<Uuid, ResultRow>,
    }

    struct ResultRow {
        reference: RunReference,
        started_at_ms: u64,
        values: BTreeMap<String, String>,
    }

    const RESULT_COLUMNS: &[&str] = &[
        "run_number",
        "run_id",
        "architecture_name",
        "mode",
        "status",
        "evaluation",
        "benchmark_id",
        "benchmark_fingerprint",
        "top_k",
        "total",
        "completed",
        "failed",
        "answered",
        "reviewed",
        "scored",
        "started_at_ms",
        "finished_at_ms",
        "embedding_model",
        "generation_model",
        "description",
        "suite_id",
        "suite_run_number",
    ];

    impl ResultRow {
        fn retrieval(run: &Run, benchmark: String) -> Result<Self> {
            let mut row = Self::from_summary(
                RunReference {
                    run_id: run.id,
                    benchmark,
                    mode: RunMode::Retrieval,
                    architecture_name: run.request.label.clone(),
                    description: run.request.description.clone(),
                },
                run,
            )?;
            row.values
                .insert("evaluation".into(), run.metric_kind.clone());
            row.values
                .insert("top_k".into(), run.request.top_k.to_string());
            row.metrics("metric", run.means.as_ref());
            Ok(row)
        }

        fn generation(run: &AnswerRunSummary, benchmark: String) -> Result<Self> {
            let mut row = Self::from_summary(
                RunReference {
                    run_id: run.id,
                    benchmark,
                    mode: RunMode::Generation,
                    architecture_name: run.request.architecture_label.clone(),
                    description: run.request.description.clone(),
                },
                run,
            )?;
            row.values
                .insert("embedding_model".into(), run.embedding_model.id.clone());
            row.values.insert(
                "generation_model".into(),
                run.generation_model.label.clone(),
            );
            // Zero is a known count even though the legacy JSON wire format omits it.
            row.values.insert("scored".into(), run.scored.to_string());
            row.metrics("metric", run.automatic_scores.as_ref());
            row.metrics("review", run.means.as_ref());
            Ok(row)
        }

        fn from_summary(reference: RunReference, summary: &impl Serialize) -> Result<Self> {
            let summary = serde_json::to_value(summary)?;
            // Both persisted run envelopes use these common field names. Only scalar
            // comparison metadata is exported; credentials, scope and evidence are excluded.
            let mut values = BTreeMap::new();
            for column in RESULT_COLUMNS {
                let value = match &summary[column] {
                    serde_json::Value::String(value) => value.clone(),
                    serde_json::Value::Number(value) => value.to_string(),
                    _ => String::new(),
                };
                values.insert((*column).to_owned(), value);
            }
            values.insert("run_id".into(), reference.run_id.to_string());
            values.insert(
                "architecture_name".into(),
                reference.architecture_name.clone(),
            );
            values.insert(
                "mode".into(),
                match reference.mode {
                    RunMode::Retrieval => "retrieval",
                    RunMode::Generation => "generation",
                }
                .into(),
            );
            values.insert(
                "benchmark_id".into(),
                summary["request"]["benchmark_id"]
                    .as_str()
                    .ok_or_else(|| Error("Result is missing its benchmark ID".into()))?
                    .into(),
            );
            let started_at_ms = summary["started_at_ms"]
                .as_u64()
                .ok_or_else(|| Error("Result is missing its start time".into()))?;
            Ok(Self {
                reference,
                started_at_ms,
                values,
            })
        }

        fn metrics(&mut self, prefix: &str, metrics: Option<&MetricValues>) {
            if let Some(metrics) = metrics {
                for (name, value) in metrics {
                    self.values
                        .insert(format!("{prefix}.{name}"), value.to_string());
                }
            }
        }
    }

    impl ResultTables {
        fn open(root: &Path) -> Result<Self> {
            let registry: RunRegistry = match File::open(root.join("runs.json")) {
                Ok(file) => serde_json::from_reader(file)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => RunRegistry {
                    next_run_number: 1,
                    runs: BTreeMap::new(),
                    suite_runs: BTreeMap::new(),
                },
                Err(error) => return Err(error.into()),
            };
            let mut ids = BTreeSet::new();
            for (number, reference) in &registry.runs {
                validate_result_name(&reference.benchmark)?;
                if *number == 0
                    || *number >= registry.next_run_number
                    || !ids.insert(reference.run_id)
                {
                    return Err(Error(
                        "Invalid or duplicate run number/ID in results/runs.json".into(),
                    ));
                }
            }
            let mut suite_children = BTreeSet::new();
            for (number, suite) in &registry.suite_runs {
                if *number == 0
                    || *number >= registry.next_run_number
                    || *number != suite.run_number
                    || registry.runs.contains_key(number)
                    || !ids.insert(suite.id)
                {
                    return Err(Error(
                        "Invalid or duplicate suite number/ID in results/runs.json".into(),
                    ));
                }
                suite.request.validate()?;
                for item in &suite.items {
                    validate_result_name(&item.benchmark)?;
                    if let Some(id) = item.run_id {
                        if !suite_children.insert(id)
                            || item.mode.is_none()
                            || item.benchmark_id.is_none()
                        {
                            return Err(Error(
                                "Invalid or duplicate suite execution in results/runs.json".into(),
                            ));
                        }
                        if let Some(reference) = registry
                            .runs
                            .values()
                            .find(|reference| reference.run_id == id)
                        {
                            let mode = match item.mode {
                                Some(SuiteMode::Retrieval) => RunMode::Retrieval,
                                _ => RunMode::Generation,
                            };
                            if reference.benchmark != item.benchmark || reference.mode != mode {
                                return Err(Error(
                                    "Suite execution identity does not match results/runs.json"
                                        .into(),
                                ));
                            }
                        }
                    }
                }
            }
            if registry.next_run_number == 0 {
                return Err(Error("Invalid next run number in results/runs.json".into()));
            }
            Ok(Self {
                registry,
                rows: BTreeMap::new(),
            })
        }

        fn check_new(&self, id: Uuid) -> Result<()> {
            if self
                .registry
                .runs
                .values()
                .any(|reference| reference.run_id == id)
            {
                return Err(Error("Run already exists in the result registry".into()));
            }
            Ok(())
        }

        fn benchmark(&self, id: Uuid) -> Result<String> {
            self.registry
                .runs
                .values()
                .find(|reference| reference.run_id == id)
                .map(|reference| reference.benchmark.clone())
                .ok_or_else(|| Error("Run has not been created".into()))
        }

        fn upsert(&mut self, mut row: ResultRow) -> Result<()> {
            let id = row.reference.run_id;
            if let Some(reference) = self
                .registry
                .runs
                .values_mut()
                .find(|reference| reference.run_id == id)
            {
                if reference.benchmark != row.reference.benchmark
                    || reference.mode != row.reference.mode
                {
                    return Err(Error(
                        "Run identity does not match results/runs.json".into(),
                    ));
                }
                // A description can be edited in the registry while the server is stopped.
                row.reference.description.clone_from(&reference.description);
                reference
                    .architecture_name
                    .clone_from(&row.reference.architecture_name);
            } else {
                let number = self.registry.next_run_number;
                self.registry.next_run_number = number
                    .checked_add(1)
                    .ok_or_else(|| Error("Run numbers are exhausted".into()))?;
                self.registry.runs.insert(number, row.reference.clone());
            }
            self.rows.insert(id, row);
            Ok(())
        }

        fn save_row(&mut self, root: &Path, row: ResultRow) -> Result<()> {
            let benchmark = row.reference.benchmark.clone();
            self.upsert(row)?;
            // Registry first: a crash before CSV replacement cannot reuse a published number.
            atomic_json(&root.join("runs.json"), &self.registry)?;
            self.save_csv(root, &benchmark)
        }

        fn save_all(&self, root: &Path) -> Result<()> {
            atomic_json(&root.join("runs.json"), &self.registry)?;
            let benchmarks: BTreeSet<_> = self
                .registry
                .runs
                .values()
                .map(|reference| reference.benchmark.as_str())
                .collect();
            for benchmark in benchmarks {
                self.save_csv(root, benchmark)?;
            }
            Ok(())
        }

        fn table(&self, benchmark: &str) -> ComparisonTable {
            let rows: Vec<_> = self
                .registry
                .runs
                .iter()
                .filter(|(_, reference)| reference.benchmark == benchmark)
                .filter_map(|(number, reference)| {
                    self.rows.get(&reference.run_id).map(|row| {
                        let mut values = row.values.clone();
                        values.insert("run_number".into(), number.to_string());
                        values.insert("description".into(), reference.description.clone());
                        if let Some(suite) = self.registry.suite_runs.values().find(|suite| {
                            suite
                                .items
                                .iter()
                                .any(|item| item.run_id == Some(reference.run_id))
                        }) {
                            values.insert("suite_id".into(), suite.id.to_string());
                            values.insert("suite_run_number".into(), suite.run_number.to_string());
                        }
                        values
                    })
                })
                .collect();
            let metrics: BTreeSet<_> = rows
                .iter()
                .flat_map(|row| row.keys())
                .filter(|name| name.starts_with("metric.") || name.starts_with("review."))
                .cloned()
                .collect();
            let columns: Vec<_> = RESULT_COLUMNS
                .iter()
                .map(|column| (*column).to_string())
                .chain(metrics)
                .collect();
            let rows = rows
                .into_iter()
                .map(|mut row| {
                    for column in &columns {
                        row.entry(column.clone()).or_default();
                    }
                    row
                })
                .collect();
            ComparisonTable {
                benchmark: benchmark.into(),
                columns,
                rows,
            }
        }

        fn save_csv(&self, root: &Path, benchmark: &str) -> Result<()> {
            let mut file = tempfile::NamedTempFile::new_in(root)?;
            file.write_all(&self.table(benchmark).csv()?)?;
            file.as_file().sync_all()?;
            file.persist(root.join(format!("{benchmark}.csv")))
                .map_err(|error| Error(error.error.to_string()))?;
            Ok(())
        }
    }

    fn validate_result_name(name: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(Error(
                "Benchmark result name must contain only letters, numbers, '-' or '_'".into(),
            ));
        }
        Ok(())
    }

    fn ids_in(path: &Path) -> Result<Vec<Uuid>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Ok(id) = entry.file_name().to_string_lossy().parse()
            {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
        Ok(serde_json::from_reader(File::open(path)?)?)
    }

    fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().expect("store path has parent"))?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path)
            .map_err(|error| Error(error.error.to_string()))?;
        Ok(())
    }
}

/// Shared Nebula transport and batch executor; benchmark modules supply every evaluation policy.
pub mod generation {

    use crate::server::Store;
    use crate::{
        AnswerCase, AnswerEvaluation, AnswerEvidence, AnswerFailure, AnswerLineage, AnswerProfile,
        AnswerRun, AnswerRunSummary, AnswerRuntime, Benchmark, Case, EmbeddingModel, Error,
        GenerationModel, ModelReceipt, ProviderDiagnostic, Result, RunStatus,
        StartAnswerRunRequest, StartFailure, config::NebulaConfig, now_ms, valid_label,
    };
    use serde::{Deserialize, de::DeserializeOwned};
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeMap, HashMap, HashSet},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use uuid::Uuid;
    const GENERATION_BATCH_SIZE: usize = 4;
    const QUERY_TOP_K: usize = 8;

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
        serde_json::from_slice(&bytes)
            .map_err(|_| Error("Nebula returned an invalid response".into()))
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
            failure.diagnostic.provider_diagnostic =
                provider_diagnostic(&value["error"]["diagnostic"]);
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
        evaluation: &'static dyn AnswerEvaluation,
    ) -> std::result::Result<AnswerRun, StartFailure> {
        request
            .validate()
            .map_err(|error| StartFailure::Invalid(error.to_string()))?;
        request.architecture_label = request.architecture_label.trim().to_owned();
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
            let ids = evaluation
                .candidate_sources(case, &sources)
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
                evaluation: evaluation.id().into(),
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
                automatic_scores: evaluation.initial_scores(),
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
        evaluation: &'static dyn AnswerEvaluation,
    ) {
        match execute_inner(&store, &config, &benchmark, &mut run, evaluation).await {
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
        evaluation: &'static dyn AnswerEvaluation,
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
            evaluation,
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
                evaluation.aggregate(&mut run.summary, &run.cases);
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
        evaluation: &'static dyn AnswerEvaluation,
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
            reference_answer: None,
            automatic_scores: None,
        };
        context.evaluation.initialize_case(case, &mut captured);
        let result: std::result::Result<(), CaseFailure> = async {
            let pinned = |error: Error| {
                CaseFailure::local(
                    "validate_answer",
                    "provenance_changed",
                    error.to_string(),
                    true,
                )
            };
            let source_ids = context
                .evaluation
                .candidate_sources(case, &context.sources)
                .map_err(pinned)?;
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
                context.evaluation.score_case(&mut captured);
                (captured, None)
            }
            Err(failure) => {
                captured.error = Some(failure.diagnostic.message.clone());
                captured.failure = Some(failure.diagnostic.clone());
                (captured, Some(failure))
            }
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

    pub fn csv(run: &AnswerRun, evaluation: &dyn AnswerEvaluation) -> Result<Vec<u8>> {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(vec![]);
        let mut columns = vec![
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
        ];
        columns.extend_from_slice(evaluation.csv_columns());
        columns.extend(["max_in_flight", "failure_json"]);
        writer.write_record(columns)?;
        for case in &run.cases {
            let mut values = vec![
                run.summary.id.to_string(),
                run.summary.request.benchmark_id.to_string(),
                run.summary.benchmark_fingerprint.clone(),
                run.summary.request.architecture_label.clone(),
                run.summary.embedding_model.id.clone(),
                run.summary.embedding_model.revision.clone(),
                run.summary.generation_model.profile_id.clone(),
                run.summary.generation_model.label.clone(),
                run.summary.evaluation.clone(),
                run.summary.top_k.to_string(),
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
            ];
            values.extend(evaluation.csv_values(run, case));
            values.extend([
                run.summary.max_in_flight.to_string(),
                case.failure
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?
                    .unwrap_or_default(),
            ]);
            writer.write_record(values)?;
        }
        writer.flush()?;
        writer
            .into_inner()
            .map_err(|error| Error(error.to_string()))
    }
}
