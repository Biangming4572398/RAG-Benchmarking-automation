use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{
    Error, Result,
    config::{NebulaConfig, validate_token},
    load_benchmarks::{LoadRequest, load_benchmarks},
    storage::Store,
    trials::{Run, RunRequest, RunStatus, execute},
};

struct AppState {
    store: Arc<Store>,
    token: String,
    nebula: Option<NebulaConfig>,
    run_slot: Arc<Semaphore>,
    load_slot: Arc<Semaphore>,
}

pub fn router(store: Arc<Store>, token: String, nebula: Option<NebulaConfig>) -> Result<Router> {
    validate_token(&token)?;
    if let Some(config) = &nebula {
        config.validate()?;
    }
    let state = Arc::new(AppState {
        store,
        token,
        nebula,
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
        .route("/api/benchmarks/v1/runs", get(list_runs).post(start_run))
        .route("/api/benchmarks/v1/runs/{id}", get(run))
        .route("/api/benchmarks/v1/runs/{id}/scores.csv", get(scores))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state))
}

async fn authenticate(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let expected = format!("Bearer {}", state.token);
    if request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(&expected)
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Bearer token required"})),
        )
            .into_response();
    }
    next.run(request).await
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

async fn load(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoadRequest>,
) -> ApiResult<impl IntoResponse> {
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
    Json(request): Json<RunRequest>,
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
    let run = Run::new(request, &benchmark, fingerprint)
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
