//! Startup uses local fixtures and a scripted Nebula; no datasets or models are downloaded.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use backend::{
    config::{Catalog, NebulaConfig},
    server::{Store, router, router_with_initialization},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};

struct Server {
    base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(app: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server { base, task }
}
fn fixture(root: &Path) -> Catalog {
    let source = root.join("hotpot.json");
    std::fs::write(&source, serde_json::to_vec(&json!([{
        "_id":"startup-example", "question":"Where is the observatory?", "answer":"PRIVATE_GOLD_ANSWER",
        "context":(0..10).map(|index| json!([format!("Passage {index}"),[format!("Passage sentence {index}")]])).collect::<Vec<_>>(),
        "supporting_facts":[["Passage 0",0]]
    }])).unwrap()).unwrap();
    let mut catalog = Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap();
    catalog.benchmarks.remove("ragtruth-qa");
    let definition = catalog.benchmarks.get_mut("hotpotqa").unwrap();
    definition.source = source.to_string_lossy().into_owned();
    definition.source_sha256 = None;
    definition.defaults.limit = 1;
    catalog
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
async fn json_response(request: reqwest::RequestBuilder, expected: StatusCode) -> Value {
    let response = request.send().await.unwrap();
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, expected, "{value}");
    value
}
async fn progress(client: &reqwest::Client, server: &Server) -> Value {
    json_response(
        client.get(format!("{}/api/benchmarks/v1/initialization", server.base)),
        StatusCode::OK,
    )
    .await
}
async fn finished(client: &reqwest::Client, server: &Server) -> Value {
    for _ in 0..500 {
        let value = progress(client, server).await;
        if value["status"] != "initializing" {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Initialization did not finish");
}
struct Script {
    store: Arc<Store>,
    corpus: PathBuf,
    entered: Notify,
    release: Semaphore,
    ready: AtomicBool,
    calls: AtomicUsize,
}
async fn reindex(
    State(script): State<Arc<Script>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    assert_eq!(headers["authorization"], "Bearer startup-test-token");
    assert!(script.store.suites().unwrap().is_empty());
    assert!(script.store.runs().unwrap().is_empty());
    assert!(script.store.answer_runs().unwrap().is_empty());
    let saved = serde_json::to_value(script.store.initialization().unwrap()).unwrap();
    assert_eq!(saved["phase"], "indexing");
    let snapshots = script.store.benchmarks_by_saved_time().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(std::fs::read_dir(&script.corpus).unwrap().count(), 10);
    for document in &snapshots[0].documents {
        let text = std::fs::read_to_string(script.corpus.join(&document.filename)).unwrap();
        assert_eq!(text, document.text);
        assert!(!text.contains("PRIVATE_GOLD_ANSWER"));
    }
    script.calls.fetch_add(1, Ordering::SeqCst);
    script.entered.notify_one();
    script.release.acquire().await.unwrap().forget();
    script.ready.store(true, Ordering::SeqCst);
    (StatusCode::ACCEPTED, Json(json!({"status":"initializing"})))
}
async fn knowledge(State(script): State<Arc<Script>>) -> Json<Value> {
    Json(
        json!({"status":{"phase":if script.ready.load(Ordering::SeqCst) {"ready"} else {"initializing"}}}),
    )
}
async fn workspace(State(script): State<Arc<Script>>) -> Json<Value> {
    let ready = script.ready.load(Ordering::SeqCst);
    let snapshots = script.store.benchmarks_by_saved_time().unwrap();
    Json(
        json!({"status":{"phase":if ready {"ready"} else {"initializing"}},
        "sources":snapshots.iter().flat_map(|snapshot| &snapshot.documents).map(|document| json!({
            "title":document.filename,"revision":document.revision,"indexed":ready
        })).collect::<Vec<_>>(), "profiles":[] }),
    )
}
async fn nebula(script: Arc<Script>) -> Server {
    serve(
        Router::new()
            .route("/api/nebula/v1/knowledge/reindex", post(reindex))
            .route("/api/nebula/v1/knowledge/status", get(knowledge))
            .route("/api/nebula/v1/workspace", get(workspace))
            .with_state(script),
    )
    .await
}
fn config(nebula: &Server) -> NebulaConfig {
    NebulaConfig {
        base_url: format!("{}/api/nebula/v1", nebula.base),
        token: "startup-test-token".into(),
    }
}
fn assert_no_experiments(root: &Path, store: &Store) {
    assert!(store.suites().unwrap().is_empty());
    assert!(store.runs().unwrap().is_empty());
    assert!(store.answer_runs().unwrap().is_empty());
    let registry: Value =
        serde_json::from_slice(&std::fs::read(root.join("results/runs.json")).unwrap()).unwrap();
    assert_eq!(registry["next_run_number"], 1);
    assert!(registry["runs"].as_object().unwrap().is_empty());
    assert!(
        registry
            .get("suite_runs")
            .is_none_or(|value| value.as_object().is_some_and(|runs| runs.is_empty()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_prepares_and_indexes_before_any_click_without_creating_experiments() {
    let root = tempfile::tempdir().unwrap();
    let catalog = fixture(root.path());
    let data = root.path().join("data");
    let store = Arc::new(Store::open(&data).unwrap());
    let corpus = root.path().join("corpus");
    let script = Arc::new(Script {
        store: store.clone(),
        corpus: corpus.clone(),
        entered: Notify::new(),
        release: Semaphore::new(0),
        ready: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let nebula = nebula(script.clone()).await;
    let server = serve(
        router_with_initialization(
            store.clone(),
            Some(config(&nebula)),
            catalog.clone(),
            Some(corpus.clone()),
        )
        .unwrap(),
    )
    .await;
    let client = client();
    tokio::time::timeout(Duration::from_secs(5), script.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        json_response(
            client.get(format!("{}/api/benchmarks/v1/health", server.base)),
            StatusCode::OK
        )
        .await["status"],
        "ok"
    );
    let indexing = progress(&client, &server).await;
    assert_eq!(indexing["status"], "initializing");
    assert_eq!(indexing["phase"], "indexing");
    assert!(
        indexing["preparations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["benchmark"] == "hotpotqa" && item["status"] == "completed")
    );
    let base = format!("{}/api/benchmarks/v1", server.base);
    for (route, body) in [
        ("suite-runs", json!({"profile_id":null})),
        ("runs", json!({"benchmark_id":uuid::Uuid::new_v4()})),
        (
            "answer-runs",
            json!({"benchmark_id":uuid::Uuid::new_v4(),"profile_id":"none"}),
        ),
        ("benchmarks", json!({"benchmark":"hotpotqa"})),
    ] {
        let blocked = json_response(
            client.post(format!("{base}/{route}")).json(&body),
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
        assert!(blocked["error"].as_str().unwrap().contains("indexing"));
    }
    assert_no_experiments(&data, &store);
    script.release.add_permits(1);
    let ready = finished(&client, &server).await;
    assert_eq!(ready["status"], "ready", "{ready}");
    assert_eq!(ready["phase"], "ready");
    assert!(ready["error"].is_null());
    let original_id = store.benchmarks().unwrap()[0].id;
    assert_no_experiments(&data, &store);
    drop(server);
    std::fs::remove_file(root.path().join("hotpot.json")).unwrap();
    let restarted = serve(
        router_with_initialization(store.clone(), Some(config(&nebula)), catalog, Some(corpus))
            .unwrap(),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), script.entered.notified())
        .await
        .unwrap();
    script.release.add_permits(1);
    assert_eq!(finished(&client, &restarted).await["status"], "ready");
    assert_eq!(script.calls.load(Ordering::SeqCst), 2);
    assert_eq!(store.benchmarks().unwrap().len(), 1);
    assert_eq!(store.benchmarks().unwrap()[0].id, original_id);
    assert_no_experiments(&data, &store);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_failure_is_persisted_blocks_runs_and_does_not_retry_until_restart() {
    let root = tempfile::tempdir().unwrap();
    let catalog = fixture(root.path());
    std::fs::remove_file(root.path().join("hotpot.json")).unwrap();
    let data = root.path().join("data");
    let store = Arc::new(Store::open(&data).unwrap());
    let server =
        serve(router_with_initialization(store.clone(), None, catalog, None).unwrap()).await;
    let client = client();
    let failure = finished(&client, &server).await;
    assert_eq!(failure["status"], "failed");
    assert_eq!(failure["phase"], "failed");
    assert!(failure["error"].as_str().unwrap().contains("HotpotQA"));
    assert!(
        failure["preparations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["benchmark"] == "hotpotqa" && item["status"] == "failed")
    );
    assert_eq!(
        serde_json::to_value(store.initialization().unwrap()).unwrap(),
        failure
    );
    let rejected = json_response(
        client
            .post(format!("{}/api/benchmarks/v1/suite-runs", server.base))
            .json(&json!({"profile_id":null})),
        StatusCode::SERVICE_UNAVAILABLE,
    )
    .await;
    assert_eq!(rejected["error"], failure["error"]);
    fixture(root.path());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(progress(&client, &server).await, failure);
    assert!(store.benchmarks().unwrap().is_empty());
    assert_no_experiments(&data, &store);
}

#[tokio::test]
async fn ordinary_router_does_not_start_initialization_or_write_initialization_state() {
    let root = tempfile::tempdir().unwrap();
    let catalog = fixture(root.path());
    let store = Arc::new(Store::open(&root.path().join("data")).unwrap());
    let server = serve(router(store.clone(), None, catalog).unwrap()).await;
    let value = progress(&client(), &server).await;
    assert_eq!(
        value,
        json!({"status":"ready","phase":"ready","preparations":[],"error":null})
    );
    assert!(store.benchmarks().unwrap().is_empty());
    assert!(!root.path().join("data/initialization.json").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_indexes_successful_preparations_but_still_reports_other_dataset_failures() {
    let root = tempfile::tempdir().unwrap();
    let mut catalog = fixture(root.path());
    let original = Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap();
    let mut ragtruth = original.benchmarks["ragtruth-qa"].clone();
    ragtruth.source = root
        .path()
        .join("missing.parquet")
        .to_string_lossy()
        .into_owned();
    catalog.benchmarks.insert("ragtruth-qa".into(), ragtruth);
    let data = root.path().join("data");
    let store = Arc::new(Store::open(&data).unwrap());
    let corpus = root.path().join("corpus");
    let script = Arc::new(Script {
        store: store.clone(),
        corpus: corpus.clone(),
        entered: Notify::new(),
        release: Semaphore::new(1),
        ready: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let nebula = nebula(script.clone()).await;
    let server = serve(
        router_with_initialization(store.clone(), Some(config(&nebula)), catalog, Some(corpus))
            .unwrap(),
    )
    .await;
    let result = finished(&client(), &server).await;
    assert_eq!(result["status"], "failed", "{result}");
    assert!(result["error"].as_str().unwrap().contains("ragtruth-qa"));
    assert_eq!(script.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.benchmarks().unwrap().len(), 1);
    assert_no_experiments(&data, &store);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_can_verify_existing_manually_indexed_snapshots_without_a_corpus_grant() {
    let root = tempfile::tempdir().unwrap();
    let catalog = fixture(root.path());
    let prepared = backend::initialize_benchmark(
        &catalog
            .resolve(&backend::config::LoadRequest {
                benchmark: "hotpotqa".into(),
                limit: None,
            })
            .unwrap(),
    )
    .unwrap();
    let data = root.path().join("data");
    let store = Arc::new(Store::open(&data).unwrap());
    store.save_benchmark(&prepared).unwrap();
    std::fs::remove_file(root.path().join("hotpot.json")).unwrap();
    let script = Arc::new(Script {
        store: store.clone(),
        corpus: root.path().join("unused-corpus"),
        entered: Notify::new(),
        release: Semaphore::new(0),
        ready: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let nebula = nebula(script.clone()).await;
    let server = serve(
        router_with_initialization(store.clone(), Some(config(&nebula)), catalog, None).unwrap(),
    )
    .await;
    let client = client();
    for _ in 0..100 {
        let pending = progress(&client, &server).await;
        assert_eq!(pending["status"], "initializing", "{pending}");
        if pending["phase"] == "indexing" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(progress(&client, &server).await["phase"], "indexing");
    assert_eq!(script.calls.load(Ordering::SeqCst), 0);
    script.ready.store(true, Ordering::SeqCst);
    let result = finished(&client, &server).await;
    assert_eq!(result["status"], "ready", "{result}");
    assert_eq!(script.calls.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("unused-corpus").exists());
    assert_no_experiments(&data, &store);
}

#[test]
fn an_occupied_listener_does_not_create_storage_or_begin_initialization() {
    let root = tempfile::tempdir().unwrap();
    let catalog = fixture(root.path());
    let catalog_path = root.path().join("benchmarks.yaml");
    std::fs::write(&catalog_path, yaml_serde::to_string(&catalog).unwrap()).unwrap();
    let data = root.path().join("must-not-be-created");
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_backend"))
        .env("BENCHMARK_ADDR", occupied.local_addr().unwrap().to_string())
        .env("BENCHMARK_DATA_DIR", &data)
        .env("BENCHMARK_CATALOG", &catalog_path)
        .env_remove("NEBULA_API_BASE")
        .env_remove("NEBULA_API_TOKEN")
        .env_remove("BENCHMARK_NEBULA_CORPUS")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .to_lowercase()
            .contains("address already in use")
    );
    assert!(
        !data.exists(),
        "Failed listener binding must precede storage and initialization side effects"
    );
}
