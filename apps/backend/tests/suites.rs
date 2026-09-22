//! Suites use a scripted loopback Nebula and local snapshots; no model providers or downloads.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use backend::{
    Benchmark, Case, Document, Run, RunStatus, StartRunRequest,
    benchmarks::hotpotqa::AnswerReference,
    config::{Catalog, NebulaConfig},
    server::{Store, router},
    suite::{self, StartSuiteRequest, SuiteItemStatus, SuiteMode, SuiteRun},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

fn catalog() -> Catalog {
    Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap()
}
fn request() -> StartSuiteRequest {
    StartSuiteRequest {
        architecture_label: "Suite architecture".into(),
        description: "A shared experiment".into(),
        profile_id: Some("profile".into()),
        top_k: None,
    }
}
fn snapshot(hotpot: bool) -> Benchmark {
    let key = if hotpot { "hotpotqa" } else { "ragtruth-qa" };
    let documents: Vec<_> = (0..if hotpot { 10 } else { 1 })
        .map(|index| Document {
            id: format!("{key}-{index}"),
            filename: format!("{key}-{index}.md"),
            text: format!("{key} passage {index}"),
            revision: format!("rev-{key}-{index}"),
        })
        .collect();
    let case = Case {
        id: format!("{key}-case"),
        query: format!("Where for {key}?"),
        document_id: if hotpot {
            String::new()
        } else {
            documents[0].id.clone()
        },
        reference_outputs: vec![],
        answer_reference: hotpot.then(|| AnswerReference {
            answer: "London".into(),
            candidate_document_ids: documents.iter().map(|doc| doc.id.clone()).collect(),
            supporting_facts: vec![],
        }),
    };
    Benchmark {
        id: Uuid::new_v4(),
        source: "fixture".into(),
        split: "test".into(),
        metric_kind: if hotpot {
            "hotpotqa_answer_v1"
        } else {
            "paired_context_recovery_v1"
        }
        .into(),
        configuration: None,
        cases: vec![case],
        documents,
    }
}

#[test]
fn suite_plan_pins_latest_snapshot_and_distinguishes_catalog_only_and_unprepared() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let old = snapshot(false);
    store.save_benchmark(&old).unwrap();
    // File timestamps have a finite resolution; explicitly make the old save older.
    let file = std::fs::File::open(
        root.path()
            .join("benchmarks")
            .join(old.id.to_string())
            .join("benchmark.json"),
    )
    .unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH)
        .unwrap();
    let latest = snapshot(false);
    store.save_benchmark(&latest).unwrap();
    let plan = suite::plan(&store, &catalog(), request()).unwrap();
    let runnable: Vec<_> = plan
        .items
        .iter()
        .filter(|item| item.status == SuiteItemStatus::Queued)
        .collect();
    assert_eq!(runnable.len(), 2);
    assert!(
        runnable
            .iter()
            .all(|item| item.benchmark_id == Some(latest.id))
    );
    let unprepared = plan
        .items
        .iter()
        .find(|item| item.benchmark == "hotpotqa")
        .unwrap();
    assert!(
        unprepared
            .reason
            .as_ref()
            .unwrap()
            .contains("No prepared snapshot")
    );
    assert_eq!(
        plan.items
            .iter()
            .filter(|item| item
                .reason
                .as_ref()
                .is_some_and(|reason| reason.contains("Catalog only")))
            .count(),
        6
    );
    let mut no_profile = request();
    no_profile.profile_id = None;
    let plan = suite::plan(&store, &catalog(), no_profile).unwrap();
    assert_eq!(
        plan.items
            .iter()
            .filter(|item| item.status == SuiteItemStatus::Queued)
            .count(),
        1
    );
}

#[test]
fn catalog_aliases_share_the_module_table_and_do_not_duplicate_suite_executions() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let mut catalog = catalog();
    let definition = catalog.benchmarks["ragtruth-qa"].clone();
    catalog
        .benchmarks
        .insert("team-ragtruth".into(), definition.clone());
    let mut benchmark = snapshot(false);
    benchmark.configuration = Some(backend::config::ResolvedBenchmark {
        key: "team-ragtruth".into(),
        definition,
    });
    store.save_benchmark(&benchmark).unwrap();
    assert_eq!(
        store
            .benchmark_info(benchmark.id)
            .unwrap()
            .module_key
            .as_deref(),
        Some("ragtruth-qa")
    );
    let suite = suite::plan(&store, &catalog, request()).unwrap();
    let runnable: Vec<_> = suite
        .items
        .iter()
        .filter(|item| item.status == SuiteItemStatus::Queued)
        .collect();
    assert_eq!(runnable.len(), 2);
    assert!(
        runnable
            .iter()
            .all(|item| item.benchmark == "ragtruth-qa" && item.benchmark_id == Some(benchmark.id))
    );
}

#[test]
fn suite_recovery_preserves_global_numbers_descriptions_and_child_results() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let benchmark = snapshot(false);
    store.save_benchmark(&benchmark).unwrap();
    let mut suite = suite::plan(&store, &catalog(), request()).unwrap();
    store.create_suite(&mut suite).unwrap();
    assert_eq!(suite.run_number, 1);
    let mut run = Run::new(
        StartRunRequest {
            benchmark_id: benchmark.id,
            top_k: Some(4),
            label: "architecture".into(),
            description: "child description".into(),
        }
        .resolve(&benchmark),
        &benchmark,
        "fingerprint".into(),
    )
    .unwrap();
    let item = suite
        .items
        .iter_mut()
        .find(|item| item.mode == Some(SuiteMode::Retrieval))
        .unwrap();
    item.run_id = Some(run.id);
    item.status = SuiteItemStatus::Running;
    store.save_suite(&suite).unwrap();
    store.create_run(&run).unwrap();
    run.status = RunStatus::Completed;
    run.finished_at_ms = Some(backend::now_ms());
    run.completed = run.total;
    run.means = Some(BTreeMap::from([("context_hit_at_k".into(), 1.0)]));
    store.save_run(&run).unwrap();
    drop(store);
    let registry = root.path().join("results/runs.json");
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
    saved["suite_runs"]["1"]["request"]["description"] =
        json!("Edited suite description while stopped");
    saved["runs"]["2"]["description"] = json!("Edited child description while stopped");
    std::fs::write(&registry, serde_json::to_vec(&saved).unwrap()).unwrap();
    let store = Store::open(root.path()).unwrap();
    let recovered = store.suite(suite.id).unwrap();
    assert_eq!(recovered.status, RunStatus::Interrupted);
    assert_eq!(
        recovered.request.description,
        "Edited suite description while stopped"
    );
    assert_eq!(
        recovered
            .items
            .iter()
            .find(|item| item.run_id.is_some())
            .unwrap()
            .status,
        SuiteItemStatus::Completed
    );
    assert!(
        recovered
            .items
            .iter()
            .any(|item| item.status == SuiteItemStatus::Interrupted)
    );
    let table = store.result_tables().unwrap();
    let row = &table.benchmarks[0].rows[0];
    assert_eq!(row["suite_run_number"], "1");
    assert_eq!(row["run_number"], "2");
    assert_eq!(row["suite_id"], suite.id.to_string());
    assert_eq!(row["description"], "Edited child description while stopped");
    assert_eq!(row["metric.context_hit_at_k"], "1");
    let mut next = suite::plan(&store, &catalog(), request()).unwrap();
    store.create_suite(&mut next).unwrap();
    assert_eq!(next.run_number, 3);
}

#[tokio::test]
async fn suites_without_runnable_snapshots_fail_with_honest_skipped_reasons() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(root.path()).unwrap());
    let mut suite = suite::plan(&store, &catalog(), request()).unwrap();
    store.create_suite(&mut suite).unwrap();
    let id = suite.id;
    suite::execute(
        store.clone(),
        NebulaConfig {
            base_url: "http://127.0.0.1:1/api/nebula/v1".into(),
            token: "unused".into(),
        },
        suite,
    )
    .await
    .unwrap();
    let finished = store.suite(id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert!(
        finished
            .items
            .iter()
            .all(|item| item.status == SuiteItemStatus::Skipped)
    );
    assert!(finished.error.unwrap().contains("No benchmark executions"));
    assert!(store.runs().unwrap().is_empty());
    assert!(store.answer_runs().unwrap().is_empty());
}

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
struct Script {
    documents: Vec<Document>,
    conversations: Mutex<BTreeMap<String, Vec<String>>>,
    events: Mutex<Vec<String>>,
    started: Notify,
    gate: Semaphore,
    fail_retrieval: bool,
}
async fn workspace(State(script): State<Arc<Script>>) -> Json<Value> {
    Json(json!({
        "scope":{"projectId":"suite-test"}, "status":{"phase":"ready", "watermark":{"generation":"stable"},
            "model":{"id":"embedding", "revision":"v1", "phase":"ready"}},
        "profiles":[{"id":"profile", "label":"Test model", "enabled":true}],
        "sources":script.documents.iter().map(|doc| json!({"id":doc.id,"title":doc.filename,"revision":doc.revision,"indexed":true})).collect::<Vec<_>>()
    }))
}
async fn conversation(State(script): State<Arc<Script>>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(
        body.as_object().unwrap().len(),
        2,
        "Only scope and candidate IDs enter corpus selection"
    );
    let id = Uuid::new_v4().to_string();
    let source_ids: Vec<String> = serde_json::from_value(body["sourceIds"].clone()).unwrap();
    assert!(source_ids.len() == 1 || source_ids.len() == 10);
    script
        .conversations
        .lock()
        .unwrap()
        .insert(id.clone(), source_ids);
    Json(
        json!({"id":id,"sourceScope":{"ref":{"conversationId":id,"revision":1,"digest":"stable"}}}),
    )
}
async fn query(State(script): State<Arc<Script>>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body["profileId"], "profile");
    assert_eq!(body["strict"], true);
    assert!(body.get("answer").is_none());
    assert!(body.get("reference_answer").is_none());
    assert!(!body.to_string().contains("London"));
    let ids =
        script.conversations.lock().unwrap()[body["conversationId"].as_str().unwrap()].clone();
    let event = if ids.len() == 10 {
        "hotpotqa generation"
    } else {
        "ragtruth generation"
    };
    script.events.lock().unwrap().push(event.into());
    script.started.notify_one();
    script.gate.acquire().await.unwrap().forget();
    let doc = script
        .documents
        .iter()
        .find(|doc| doc.id == ids[0])
        .unwrap();
    Json(
        json!({"scope":body["scope"],"conversationScope":body["conversationScope"],"watermark":{"generation":"stable"},
        "outcome":"answered","answer":"London","evidence":[{"id":"e1","ordinal":1,"sourceId":doc.id,
        "sourceTitle":doc.filename,"sourceRevision":doc.revision,"location":"p1","excerpt":doc.text}],"lineage":[],
        "modelReceipt":{"profileId":"profile","route":"remote","modelLabel":"Test model"}}),
    )
}
async fn retrieve(
    State(script): State<Arc<Script>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    script
        .events
        .lock()
        .unwrap()
        .push("ragtruth retrieval".into());
    if script.fail_retrieval {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"offline"})),
        );
    }
    let doc = script
        .documents
        .iter()
        .find(|doc| doc.id == "ragtruth-qa-0")
        .unwrap();
    (
        StatusCode::OK,
        Json(
            json!({"scope":body["scope"],"conversationScope":body["conversationScope"],
        "watermark":{"generation":"stable"},"evidence":[{"id":"e1","sourceId":doc.id,"sourceRevision":doc.revision,"excerpt":doc.text,"score":1.0}]}),
        ),
    )
}
async fn wait_suite(client: &reqwest::Client, base: &str, id: Uuid) -> SuiteRun {
    for _ in 0..200 {
        let suite: SuiteRun = client
            .get(format!("{base}/suite-runs/{id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if suite.status != RunStatus::Running {
            return suite;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("suite did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_suite_runs_every_supported_mode_serially_and_exposes_numbered_results() {
    exercise_api(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_execution_keeps_its_results_and_does_not_cancel_later_benchmarks() {
    exercise_api(true).await;
}
async fn exercise_api(fail_retrieval: bool) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(root.path()).unwrap());
    let ragtruth = snapshot(false);
    let hotpot = snapshot(true);
    store.save_benchmark(&ragtruth).unwrap();
    store.save_benchmark(&hotpot).unwrap();
    let script = Arc::new(Script {
        documents: ragtruth
            .documents
            .iter()
            .chain(&hotpot.documents)
            .cloned()
            .collect(),
        conversations: Mutex::new(BTreeMap::new()),
        events: Mutex::new(vec![]),
        started: Notify::new(),
        gate: Semaphore::new(0),
        fail_retrieval,
    });
    let nebula = serve(
        Router::new()
            .route("/api/nebula/v1/workspace", get(workspace))
            .route("/api/nebula/v1/conversations", post(conversation))
            .route("/api/nebula/v1/query", post(query))
            .route("/api/nebula/v1/retrieve", post(retrieve))
            .with_state(script.clone()),
    )
    .await;
    let server = serve(
        router(
            store.clone(),
            Some(NebulaConfig {
                base_url: format!("{}/api/nebula/v1", nebula.base),
                token: "test-token".into(),
            }),
            catalog(),
        )
        .unwrap(),
    )
    .await;
    let base = format!("{}/api/benchmarks/v1", server.base);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let catalog: Value = client
        .get(format!("{base}/catalog"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(catalog["module_keys"]["ragtruth-qa"], "ragtruth-qa");
    assert_eq!(catalog["module_keys"].as_object().unwrap().len(), 8);
    let mut body = serde_json::to_value(request()).unwrap();
    let expected_label = if fail_retrieval {
        "Suite architecture"
    } else {
        body.as_object_mut().unwrap().remove("architecture_label");
        "baseline"
    };
    let accepted = client
        .post(format!("{base}/suite-runs"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let accepted: SuiteRun = accepted.json().await.unwrap();
    assert_eq!(accepted.request.architecture_label, expected_label);
    tokio::time::timeout(Duration::from_secs(5), script.started.notified())
        .await
        .unwrap();
    // A single permit covers the entire suite, including time between child runs.
    assert_eq!(
        client
            .post(format!("{base}/suite-runs"))
            .json(&request())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .post(format!("{base}/runs"))
            .json(&json!({"benchmark_id":ragtruth.id,"label":"outside suite"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        script.events.lock().unwrap().as_slice(),
        ["hotpotqa generation"]
    );
    script.gate.add_permits(2);
    let finished = wait_suite(&client, &base, accepted.id).await;
    assert_eq!(
        finished.status,
        if fail_retrieval {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        }
    );
    assert_eq!(
        finished
            .items
            .iter()
            .filter(|item| item.status == SuiteItemStatus::Skipped)
            .count(),
        6
    );
    assert_eq!(
        finished
            .items
            .iter()
            .filter(|item| item.run_id.is_some())
            .count(),
        3
    );
    assert_eq!(
        script.events.lock().unwrap().as_slice(),
        [
            "hotpotqa generation",
            "ragtruth retrieval",
            "ragtruth generation"
        ]
    );
    let tables: Value = client
        .get(format!("{base}/results"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tables = tables["benchmarks"].as_array().unwrap();
    assert_eq!(tables.len(), 2);
    for table in tables {
        for row in table["rows"].as_array().unwrap() {
            assert_eq!(row["suite_run_number"], accepted.run_number.to_string());
            assert_eq!(row["suite_id"], accepted.id.to_string());
            assert_eq!(row["architecture_name"], expected_label);
            assert_ne!(row["run_number"], row["suite_run_number"]);
            if row["mode"] == "generation" && table["benchmark"] == "ragtruth-qa" {
                assert_eq!(row["reviewed"], "0");
                assert!(
                    !row.as_object()
                        .unwrap()
                        .iter()
                        .any(|(key, value)| key.starts_with("review.") && value != "")
                );
            }
            if table["benchmark"] == "hotpotqa" {
                assert_eq!(row["metric.exact_match"], "1");
            }
        }
    }
    let csv = client
        .get(format!("{base}/results/ragtruth-qa/scores.csv"))
        .send()
        .await
        .unwrap();
    assert_eq!(csv.status(), StatusCode::OK);
    assert!(csv.text().await.unwrap().contains("suite_run_number"));
    let json: Value = client
        .get(format!("{base}/suite-runs"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    let invalid = client
        .post(format!("{base}/suite-runs"))
        .json(&json!({"architecture_label":"bad","top_k":0}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}
