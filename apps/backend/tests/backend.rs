use std::{fs::File, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use backend::{
    Benchmark, Run, RunRequest, RunStatus, StartRunRequest,
    config::{Catalog, LoadRequest, NebulaConfig, ResolvedBenchmark},
    initialize_benchmark,
    server::{Store, router},
};
use polars::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixture(dir: &TempDir) -> String {
    let path = dir.path().join("test.parquet");
    let mut frame = df!(
        "id" => ["1", "2", "3", "4"],
        "query" => ["Where, exactly?\nTell me.", "Where, exactly?\nTell me.", "Which city?", "Summarize"],
        "context" => ["Alpha is in London.", "Alpha is in London.", "Beta is in Paris.", "Not a QA context"],
        "output" => ["London", "Mars", "Paris", "summary"],
        "task_type" => ["QA", "QA", "QA", "Summary"],
        "quality" => ["good", "good", "good", "good"],
        "model" => ["old-a", "old-b", "old-a", "old-a"],
        "hallucination_labels" => ["[]", "[{\"text\":\"Mars\"}]", "[]", "[]"]
    ).unwrap();
    ParquetWriter::new(File::create(&path).unwrap())
        .finish(&mut frame)
        .unwrap();
    path.to_string_lossy().into()
}

fn load_fixture(dir: &TempDir) -> Benchmark {
    initialize_benchmark(&load_settings(&fixture(dir), 10)).unwrap()
}

fn catalog_for(source: &str, limit: usize) -> Catalog {
    let mut catalog = Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap();
    let definition = catalog.benchmarks.get_mut("ragtruth-qa").unwrap();
    definition.source = source.into();
    definition.defaults.limit = limit;
    definition.defaults.top_k = 2;
    catalog
}

fn load_settings(source: &str, limit: usize) -> ResolvedBenchmark {
    catalog_for(source, limit)
        .resolve(&LoadRequest {
            benchmark: "ragtruth-qa".into(),
            limit: None,
        })
        .unwrap()
}

#[test]
fn parquet_loading_preserves_annotations_but_exports_only_deduplicated_qa_contexts() {
    let dir = TempDir::new().unwrap();
    let benchmark = load_fixture(&dir);
    assert_eq!(benchmark.cases.len(), 2);
    assert_eq!(benchmark.documents.len(), 2);
    assert_eq!(benchmark.cases[0].reference_outputs.len(), 2);
    assert_eq!(
        benchmark.cases[0].reference_outputs[1].hallucination_labels[0]["text"],
        "Mars"
    );
    let store = Store::open(&dir.path().join("data")).unwrap();
    let info = store.save_benchmark(&benchmark).unwrap();
    let mut repeated = benchmark.clone();
    repeated.id = uuid::Uuid::new_v4();
    assert_eq!(
        store.save_benchmark(&repeated).unwrap().fingerprint,
        info.fingerprint
    );
    for document in &benchmark.documents {
        let text = std::fs::read_to_string(info.corpus_path.join(&document.filename)).unwrap();
        assert_eq!(text, document.text);
        assert!(!text.contains("Mars"));
        assert_eq!(backend::digest(text.as_bytes()), document.revision);
    }
    let id = benchmark.id;
    drop(store);
    let reopened = Store::open(&dir.path().join("data")).unwrap();
    assert_eq!(reopened.benchmark(id).unwrap().cases.len(), 2);
}

#[test]
fn limit_applies_to_unique_cases_and_invalid_input_is_rejected() {
    let dir = TempDir::new().unwrap();
    let source = fixture(&dir);
    let mut settings = load_settings(&source, 1);
    let benchmark = initialize_benchmark(&settings).unwrap();
    assert_eq!(benchmark.cases.len(), 1);
    assert_eq!(benchmark.cases[0].reference_outputs.len(), 2);
    settings.definition.defaults.limit = 0;
    assert!(initialize_benchmark(&settings).is_err());
    settings.definition.defaults.limit = 1;
    settings.definition.split = "../../bad".into();
    assert!(initialize_benchmark(&settings).is_err());
    assert!(initialize_benchmark(&load_settings("missing.parquet", 1)).is_err());
}

#[test]
#[ignore = "downloads public RAGTruth Parquet from Hugging Face"]
fn live_hugging_face_ragtruth() {
    let catalog = Catalog::load(std::path::Path::new("benchmarks.yaml")).unwrap();
    let settings = catalog
        .resolve(&LoadRequest {
            benchmark: "ragtruth-qa".into(),
            limit: None,
        })
        .unwrap();
    let benchmark = initialize_benchmark(&settings).unwrap();
    assert_eq!(benchmark.cases.len(), 100);
    assert!(!benchmark.documents.is_empty());
    assert!(
        benchmark
            .cases
            .iter()
            .all(|case| !case.reference_outputs.is_empty())
    );
    println!(
        "Loaded {} QA cases, {} contexts, {} historical outputs from {}",
        benchmark.cases.len(),
        benchmark.documents.len(),
        benchmark
            .cases
            .iter()
            .map(|case| case.reference_outputs.len())
            .sum::<usize>(),
        benchmark.source
    );
}

#[test]
fn store_rejects_second_owner_and_recovers_interrupted_runs() {
    let dir = TempDir::new().unwrap();
    let benchmark = load_fixture(&dir);
    let root = dir.path().join("data");
    let store = Store::open(&root).unwrap();
    assert!(Store::open(&root).is_err());
    let run = Run::new(
        RunRequest {
            benchmark_id: benchmark.id,
            top_k: 8,
            label: "test".into(),
        },
        &benchmark,
        "fingerprint".into(),
    )
    .unwrap();
    store.create_run(&run).unwrap();
    assert!(store.create_run(&run).is_err());
    assert_eq!(store.runs().unwrap().len(), 1);
    drop(store);
    let store = Store::open(&root).unwrap();
    let recovered = store.run(run.id).unwrap();
    assert_eq!(recovered.status, RunStatus::Interrupted);
    assert!(recovered.finished_at_ms.is_some());
}

struct TestServer {
    base: String,
    task: tokio::task::JoinHandle<()>,
}

#[test]
fn run_overrides_and_pre_catalog_snapshots_remain_supported() {
    let dir = TempDir::new().unwrap();
    let benchmark = load_fixture(&dir);
    let request = |top_k| StartRunRequest {
        benchmark_id: benchmark.id,
        top_k,
        label: String::new(),
    };
    assert_eq!(request(None).resolve(&benchmark).top_k, 2);
    assert_eq!(request(Some(7)).resolve(&benchmark).top_k, 7);
    assert!(
        Run::new(
            request(Some(0)).resolve(&benchmark),
            &benchmark,
            "fp".into()
        )
        .is_err()
    );
    let mut old = serde_json::to_value(&benchmark).unwrap();
    old.as_object_mut().unwrap().remove("configuration");
    let legacy: Benchmark = serde_json::from_value(old).unwrap();
    assert_eq!(request(None).resolve(&legacy).top_k, 8);
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(app: Router) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    TestServer { base, task }
}

async fn nebula(benchmark: &Benchmark, drift: bool, fail: bool) -> TestServer {
    let documents = benchmark.documents.clone();
    let sources = documents
        .iter()
        .map(|doc| {
            json!({"id": doc.id, "title": doc.filename,
        "revision": doc.revision, "indexed": true})
        })
        .collect::<Vec<_>>();
    let scope = json!({"projectId":"project-test", "corpusId":"corpus-test"});
    let watermark = json!({"generation":"generation-1", "collectionGeneration":"collection-1",
        "overlaySequence":0, "pipelineFingerprint":"pipeline-1"});
    let workspace = json!({"scope":scope, "status":{"phase":"ready", "watermark":watermark}, "sources":sources});
    let expected_sources = sources
        .iter()
        .map(|item| item["id"].clone())
        .collect::<Vec<_>>();
    let app = Router::new()
        .route("/api/nebula/v1/workspace", get(move || { let workspace = workspace.clone(); async move { Json(workspace) } }))
        .route("/api/nebula/v1/conversations", post(move |Json(body): Json<Value>| {
            let expected_sources = expected_sources.clone();
            async move {
                assert_eq!(body["sourceIds"], json!(expected_sources));
                Json(json!({"sourceScope":{"ref":{"conversationId":"c1", "revision":1, "digest":"digest-1"}}}))
            }
        }))
        .route("/api/nebula/v1/retrieve", post(move |Json(body): Json<Value>| {
            let documents = documents.clone(); let watermark = watermark.clone();
            async move {
                // Make the first result wrong and the second correct for each question.
                if fail && body["query"] == "Which city?" { return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"offline"}))); }
                let alpha = documents.iter().find(|doc| doc.text.starts_with("Alpha")).unwrap();
                let beta = documents.iter().find(|doc| doc.text.starts_with("Beta")).unwrap();
                let ordered = if body["query"] == "Which city?" { [alpha, beta] } else { [beta, alpha] };
                let evidence = ordered.iter().enumerate().map(|(i, doc)| json!({"id": format!("e{i}"),
                    "sourceId":doc.id, "sourceRevision":doc.revision, "excerpt":doc.text, "score":0.8})).collect::<Vec<_>>();
                (StatusCode::OK, Json(json!({"scope":body["scope"], "conversationScope":body["conversationScope"],
                    "watermark":if drift { json!({"generation":"changed"}) } else { watermark }, "evidence":evidence})))
            }
        }));
    serve(app).await
}

async fn wait_run(client: &reqwest::Client, base: &str, id: &str) -> Value {
    for _ in 0..100 {
        let run: Value = client
            .get(format!("{base}/runs/{id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if run["status"] != "running" {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("run did not complete");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_loads_runs_scores_and_reopens_without_a_database() {
    let dir = TempDir::new().unwrap();
    let source = fixture(&dir);
    let reference = initialize_benchmark(&load_settings(&source, 10)).unwrap();
    let nebula = nebula(&reference, false, false).await;
    let store = Arc::new(Store::open(&dir.path().join("data")).unwrap());
    let server = serve(
        router(
            store.clone(),
            Some(NebulaConfig {
                base_url: format!("{}/api/nebula/v1", nebula.base),
                token: "nebula-token".into(),
            }),
            catalog_for(&source, 10),
        )
        .unwrap(),
    )
    .await;
    let base = format!("{}/api/benchmarks/v1", server.base);
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/benchmarks"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let loaded = client
        .post(format!("{base}/benchmarks"))
        .json(&json!({"benchmark":"ragtruth-qa", "limit":2}))
        .send()
        .await
        .unwrap();
    assert_eq!(loaded.status(), StatusCode::CREATED);
    let loaded: Value = loaded.json().await.unwrap();
    assert_eq!(loaded["case_count"], 2);
    assert_eq!(
        loaded["configuration"]["definition"]["defaults"]["limit"],
        2
    );
    // A new server/catalog must not change defaults on an existing snapshot.
    let mut edited_catalog = catalog_for(&source, 10);
    edited_catalog
        .benchmarks
        .get_mut("ragtruth-qa")
        .unwrap()
        .defaults
        .top_k = 1;
    let new_server = serve(
        router(
            store,
            Some(NebulaConfig {
                base_url: format!("{}/api/nebula/v1", nebula.base),
                token: "nebula-token".into(),
            }),
            edited_catalog,
        )
        .unwrap(),
    )
    .await;
    let base = format!("{}/api/benchmarks/v1", new_server.base);
    let catalog: Value = client
        .get(format!("{base}/catalog"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(catalog["benchmarks"]["ragtruth-qa"]["defaults"]["top_k"], 1);
    assert_eq!(catalog["benchmarks"].as_object().unwrap().len(), 8);
    assert_eq!(catalog["benchmarks"]["qasper"]["adapter"], "external_suite");
    let external = client
        .post(format!("{base}/benchmarks"))
        .json(&json!({"benchmark":"qasper"}))
        .send()
        .await
        .unwrap();
    assert_eq!(external.status(), StatusCode::BAD_REQUEST);
    let error: Value = external.json().await.unwrap();
    assert!(error["error"].as_str().unwrap().contains("not integrated"));
    let invalid = client
        .post(format!("{base}/benchmarks"))
        .json(&json!({"benchmark":"unknown"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let response = client
        .post(format!("{base}/runs"))
        .json(&json!({"benchmark_id":loaded["id"], "label":"baseline"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run: Value = response.json().await.unwrap();
    assert_eq!(run["request"]["top_k"], 2);
    let id = run["id"].as_str().unwrap();
    let result = wait_run(&client, &base, id).await;
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["completed"], 2);
    assert_eq!(result["means"]["context_hit_at_k"], 1.0);
    assert_eq!(result["means"]["reciprocal_rank_at_k"], 0.5);
    let csv = client
        .get(format!("{base}/runs/{id}/scores.csv"))
        .send()
        .await
        .unwrap();
    assert_eq!(csv.status(), StatusCode::OK);
    let csv = csv.text().await.unwrap();
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let headers = reader.headers().unwrap().clone();
    let query_column = headers.iter().position(|header| header == "query").unwrap();
    let rows = reader.records().map(|row| row.unwrap()).collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_eq!(&rows[0][query_column], "Where, exactly?\nTell me.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_queries_and_index_drift_are_recorded_without_fake_zero_scores() {
    for (drift, failure) in [(true, false), (false, true)] {
        let dir = TempDir::new().unwrap();
        let benchmark = load_fixture(&dir);
        let nebula = nebula(&benchmark, drift, failure).await;
        let store = Arc::new(Store::open(&dir.path().join("data")).unwrap());
        let run = Run::new(
            RunRequest {
                benchmark_id: benchmark.id,
                top_k: 2,
                label: "".into(),
            },
            &benchmark,
            "fp".into(),
        )
        .unwrap();
        store.create_run(&run).unwrap();
        backend::benchmarks::ragtruth::execute_retrieval(
            store.clone(),
            NebulaConfig {
                base_url: format!("{}/api/nebula/v1", nebula.base),
                token: "token".into(),
            },
            benchmark,
            run.clone(),
        )
        .await;
        let result = store.run(run.id).unwrap();
        assert_eq!(result.status, RunStatus::Failed);
        assert_eq!(result.completed, usize::from(failure));
        assert_eq!(result.failed, 1);
        assert_eq!(result.means.is_some(), failure);
        let bytes = store.scores(run.id).unwrap();
        let mut reader = csv::Reader::from_reader(bytes.as_slice());
        let column = reader
            .headers()
            .unwrap()
            .iter()
            .position(|h| h == "context_hit_at_k")
            .unwrap();
        assert_eq!(&reader.records().last().unwrap().unwrap()[column], "");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_corpus_selection_fails_before_querying() {
    let dir = TempDir::new().unwrap();
    let mut benchmark = load_fixture(&dir);
    let original = benchmark.documents[0].clone();
    for index in 0..1000 {
        let mut document = original.clone();
        document.id = backend::digest(index.to_string().as_bytes());
        document.filename = format!("ragtruth-{}.md", document.id);
        benchmark.documents.push(document);
    }
    let nebula = nebula(&benchmark, false, false).await;
    let store = Arc::new(Store::open(&dir.path().join("data")).unwrap());
    let run = Run::new(
        RunRequest {
            benchmark_id: benchmark.id,
            top_k: 8,
            label: "".into(),
        },
        &benchmark,
        "fp".into(),
    )
    .unwrap();
    store.create_run(&run).unwrap();
    backend::benchmarks::ragtruth::execute_retrieval(
        store.clone(),
        NebulaConfig {
            base_url: format!("{}/api/nebula/v1", nebula.base),
            token: "token".into(),
        },
        benchmark,
        run.clone(),
    )
    .await;
    let result = store.run(run.id).unwrap();
    assert_eq!(result.status, RunStatus::Failed);
    assert!(result.error.unwrap().contains("64 KiB"));
    assert_eq!(result.completed, 0);
    assert!(store.scores(run.id).is_err());
}
