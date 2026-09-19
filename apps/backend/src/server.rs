use std::sync::Arc;

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

use crate::{
    Error, Result,
    catalog::{Catalog, LoadRequest},
    config::NebulaConfig,
    load_benchmarks::load_benchmarks,
    storage::Store,
    trials::{Run, RunStatus, StartRunRequest, execute},
};

struct AppState {
    store: Arc<Store>,
    nebula: Option<NebulaConfig>,
    catalog: Catalog,
    run_slot: Arc<Semaphore>,
    load_slot: Arc<Semaphore>,
}

pub fn router(store: Arc<Store>, nebula: Option<NebulaConfig>, catalog: Catalog) -> Result<Router> {
    if let Some(config) = &nebula {
        config.validate()?;
    }
    let state = Arc::new(AppState {
        store,
        nebula,
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
        .route("/api/benchmarks/v1/runs", get(list_runs).post(start_run))
        .route("/api/benchmarks/v1/runs/{id}", get(run))
        .route("/api/benchmarks/v1/runs/{id}/scores.csv", get(scores))
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
    Json(state.catalog.clone())
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
        let benchmark = load_benchmarks(&request)
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
    let run = Run::new(request.resolve(&benchmark), &benchmark, fingerprint)
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
        let task = tokio::spawn(execute(state.store.clone(), config, benchmark, job_run));
        if task.await.is_err()
            && let Ok(mut failed) = state.store.run(id)
        {
            failed.status = RunStatus::Failed;
            failed.error = Some("Benchmark worker stopped unexpectedly".into());
            failed.finished_at_ms = Some(crate::trials::now_ms());
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
