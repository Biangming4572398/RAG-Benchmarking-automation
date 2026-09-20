//! Answer evaluation uses only a scripted loopback Nebula, never a model provider.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use backend::{
    answers::AnswerRun,
    catalog::Catalog,
    config::NebulaConfig,
    load_benchmarks::{
        AnswerReference, Benchmark, Case, Document, ReferenceOutput, SupportingFact, digest,
    },
    server::router,
    storage::Store,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

const PROFILE: &str = "moonshot-kimi-k3";
const TOKEN: &str = "scripted-nebula-test-token";

fn benchmark() -> Benchmark {
    let documents = ["Alpha is in London.", "Beta is in Paris."]
        .into_iter()
        .map(|text| {
            let id = digest(text.as_bytes());
            Document {
                filename: format!("ragtruth-{id}.md"),
                revision: id.clone(),
                id,
                text: text.into(),
            }
        })
        .collect::<Vec<_>>();
    let cases = ["Where is Alpha?", "Where is Beta?"]
        .into_iter()
        .enumerate()
        .map(|(index, query)| Case {
            id: format!("case-{index}"),
            query: query.into(),
            document_id: documents[index].id.clone(),
            reference_outputs: vec![ReferenceOutput {
                id: format!("historical-{index}"),
                output: "HISTORICAL OUTPUT MUST NEVER BECOME A GENERATED ANSWER".into(),
                model: "historical-model".into(),
                quality: "bad".into(),
                hallucination_labels: json!([{"text":"historical hallucination"}]),
            }],
            answer_reference: None,
        })
        .collect();
    Benchmark {
        id: Uuid::new_v4(),
        source: "local-scripted-ragtruth.parquet".into(),
        split: "test".into(),
        metric_kind: "paired_context_recovery_v1".into(),
        configuration: None,
        cases,
        documents,
    }
}

fn hotpot_benchmark() -> Benchmark {
    let mut benchmark = benchmark();
    benchmark.source = "local-scripted-hotpotqa.json".into();
    benchmark.split = "dev".into();
    benchmark.metric_kind = "hotpotqa_answer_v1".into();
    benchmark.documents = (0..11)
        .map(|index| {
            let text = match index {
                0 => "London".to_owned(),
                1 => "Paris".to_owned(),
                _ => format!("Distractor {index}"),
            };
            let id = digest(text.as_bytes());
            Document {
                filename: format!("hotpotqa-{id}.md"),
                revision: id.clone(),
                id,
                text,
            }
        })
        .collect();
    for (index, case) in benchmark.cases.iter_mut().enumerate() {
        case.document_id.clear();
        case.reference_outputs.clear();
        case.answer_reference = Some(AnswerReference {
            answer: benchmark.documents[index].text.clone(),
            candidate_document_ids: benchmark.documents[index..index + 10]
                .iter()
                .map(|doc| doc.id.clone())
                .collect(),
            supporting_facts: vec![SupportingFact {
                title: "GOLD SUPPORT LABEL MUST NOT ENTER REQUESTS".into(),
                sentence_index: 7,
            }],
        });
    }
    benchmark
}

struct TestServer {
    base: String,
    task: tokio::task::JoinHandle<()>,
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

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn response(request: reqwest::RequestBuilder, expected: StatusCode) -> Value {
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, expected, "{body}");
    serde_json::from_str(&body).unwrap()
}

#[derive(Clone, Copy, Default)]
enum Outcome {
    #[default]
    Answered,
    FailSecond,
    UnsafeFailure,
    MalformedDiagnostic(usize),
    ScoringOrder,
    EvidenceOnly,
    Refused,
    MissingReceipt,
    ChangedWatermark,
    ChangedModel,
    ForeignEvidence,
    DuplicateEvidence,
    ReusedConversation,
    UnreadyEmbedding,
    DisabledProfile,
    ConciseAnswer,
    OtherCaseEvidence,
}

struct Script {
    benchmark: Benchmark,
    outcome: Outcome,
    queries: Mutex<Vec<Value>>,
    conversations: Mutex<Vec<Value>>,
    query_started: Notify,
    release_query: Option<Arc<Semaphore>>,
    release_cases: Vec<Semaphore>,
}

fn scope() -> Value {
    json!({"projectId":"scripted-project", "checkoutId":"scripted-checkout", "projectName":"Scripted project",
        "corpusId":"scripted-corpus", "corpusName":"Scripted corpus", "access":"private", "routing":"remote-allowed"})
}

fn watermark() -> Value {
    json!({"generation":"generation-1", "collectionGeneration":"collection-1",
        "overlaySequence":0, "pipelineFingerprint":"pipeline-1"})
}

fn assert_auth(headers: &HeaderMap) {
    assert_eq!(
        headers.get("authorization").unwrap(),
        &format!("Bearer {TOKEN}")
    );
}

async fn workspace(State(script): State<Arc<Script>>, headers: HeaderMap) -> Json<Value> {
    assert_auth(&headers);
    let mut workspace = json!({
        "scope":scope(),
        // The public Nebula HTTP protocol flattens the portable engine's embedding status.
        "status":{"phase":"ready", "stage":"complete", "summary":"Ready", "progress":1.0,
            "sourceCount":2,"indexedSourceCount":2,"pendingSourceCount":0,"unsearchableSourceCount":0,
            "watermark":watermark(),"model":{"id":"scripted-embedding","revision":"embedding-revision-1","phase":"ready","summary":"Ready"}},
        "selectedProfileId": PROFILE,
        "profiles":[{"id":PROFILE,"label":"Kimi K3","description":"Scripted reasoning profile","execution":"direct-session","enabled":true}],
        "conversations":[],"collections":[],
        "sources":script.benchmark.documents.iter().map(|doc|json!({
            "id":doc.id,"title":doc.filename,"kind":"markdown","detail":"Scripted document",
            "revision":doc.revision,"indexed":true,"collectionIds":[]
        })).collect::<Vec<_>>()
    });
    match script.outcome {
        Outcome::UnreadyEmbedding => {
            workspace["status"]["model"]["phase"] = json!("loading");
        }
        Outcome::DisabledProfile => {
            workspace["profiles"][0]["enabled"] = json!(false);
            workspace["profiles"][0]["disabledReason"] = json!("Remote model is not configured");
        }
        _ => {}
    }
    Json(workspace)
}

async fn conversation(
    State(script): State<Arc<Script>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_auth(&headers);
    assert_eq!(body["scope"], scope());
    let mut actual = body["sourceIds"].as_array().unwrap().clone();
    actual.sort_by_key(Value::to_string);
    let mut conversations = script.conversations.lock().unwrap();
    let mut candidates = script.benchmark.cases.iter().map(|case| {
        let mut ids = if let Some(reference) = &case.answer_reference {
            reference
                .candidate_document_ids
                .iter()
                .map(|id| json!(id))
                .collect::<Vec<_>>()
        } else {
            script
                .benchmark
                .documents
                .iter()
                .map(|doc| json!(doc.id))
                .collect::<Vec<_>>()
        };
        ids.sort_by_key(Value::to_string);
        ids
    });
    assert!(
        candidates.any(|expected| actual == expected),
        "each conversation uses one case's complete candidate corpus"
    );
    let expected = actual;
    conversations.push(body);
    let id = if matches!(script.outcome, Outcome::ReusedConversation) {
        "conversation-reused".into()
    } else {
        format!("conversation-{}", conversations.len())
    };
    Json(json!({"id":id,"title":"Untitled conversation","turns":[],
        "sourceScope":{"ref":{"conversationId":id,"revision":1,"digest":format!("digest-{id}")},
        "sourceIds":expected,"collectionGeneration":"collection-1"}}))
}

async fn query(
    State(script): State<Arc<Script>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert_auth(&headers);
    assert_eq!(body["scope"], scope());
    assert_eq!(body["profileId"], PROFILE);
    assert_eq!(body["strict"], true);
    assert!(
        body.get("topK").is_none(),
        "/query does not accept retrieve-only topK"
    );
    assert_eq!(
        body["conversationId"],
        body["conversationScope"]["conversationId"]
    );
    script.queries.lock().unwrap().push(body.clone());
    script.query_started.notify_one();
    if let Some(release) = &script.release_query {
        release.acquire().await.unwrap().forget();
    }
    let index = script
        .benchmark
        .cases
        .iter()
        .position(|case| body["query"] == case.query)
        .unwrap();
    if matches!(script.outcome, Outcome::ScoringOrder) {
        script.release_cases[index]
            .acquire()
            .await
            .unwrap()
            .forget();
    }
    if matches!(
        script.outcome,
        Outcome::FailSecond | Outcome::MalformedDiagnostic(_)
    ) && index == 1
    {
        let mut body = json!({"error":{
            "code":"reasoning_rejected",
            "message":"DO-NOT-LOG upstream echoed prompt and credential",
            "diagnostic":{
                "provider":"moonshot", "category":"rejected", "upstreamStatus":400,
                "upstreamCode":"context_length_exceeded", "upstreamType":"invalid_request_error",
                "requestBytes":12456, "maxOutputTokens":2048, "attempt":1, "elapsedMs":32,
                "requestId":"DO-NOT-LOG request identifier", "model":"DO-NOT-LOG model",
                "extra":"DO-NOT-LOG secret"
            }
        }});
        if let Outcome::MalformedDiagnostic(index) = script.outcome {
            let (field, value) = [
                ("maxOutputTokens", 0),
                ("attempt", 0),
                ("requestBytes", u64::MAX),
                ("elapsedMs", u64::MAX),
                ("maxOutputTokens", u64::MAX),
                ("attempt", u64::MAX),
            ][index];
            body["error"]["diagnostic"][field] = json!(value);
        }
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(body));
    }
    if matches!(script.outcome, Outcome::UnsafeFailure) && index == 1 {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error":{
                "code":"reasoning_rejected", "message":"DO-NOT-LOG".repeat(2000),
                "diagnostic":{"provider":"moonshot", "category":"rejected", "requestBytes":42}
            }})),
        );
    }
    let case = &script.benchmark.cases[index];
    let document = script
        .benchmark
        .documents
        .iter()
        .find(|document| {
            case.answer_reference
                .as_ref()
                .map_or(document.id == case.document_id, |reference| {
                    document.text == reference.answer
                })
        })
        .unwrap();
    if !matches!(script.outcome, Outcome::ReusedConversation) {
        let conversation_index = body["conversationId"]
            .as_str()
            .unwrap()
            .strip_prefix("conversation-")
            .unwrap()
            .parse::<usize>()
            .unwrap()
            - 1;
        let mut selected = script.conversations.lock().unwrap()[conversation_index]["sourceIds"]
            .as_array()
            .unwrap()
            .clone();
        let mut expected = case
            .answer_reference
            .as_ref()
            .map(|reference| {
                reference
                    .candidate_document_ids
                    .iter()
                    .map(|id| json!(id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                script
                    .benchmark
                    .documents
                    .iter()
                    .map(|doc| json!(doc.id))
                    .collect()
            });
        selected.sort_by_key(Value::to_string);
        expected.sort_by_key(Value::to_string);
        assert_eq!(
            selected, expected,
            "each query retains its own selected candidates"
        );
    }
    let mut result = json!({
        "scope":scope(), "conversationScope":body["conversationScope"],
        "watermark":watermark(), "outcome":"answered", "answer":format!("Generated answer: {}", document.text),
        "evidence":[{"id":format!("evidence-{index}"),"ordinal":index+1,"sourceId":document.id,"sourceTitle":document.filename,"sourceRevision":document.revision,"location":"paragraph 1","excerpt":document.text}],
        "lineage":[{"id":format!("claim-{index}"),"claim":document.text,"evidenceIds":[format!("evidence-{index}")]}],
        "modelReceipt":{"profileId":PROFILE,"route":"remote","modelLabel":"Kimi K3"},
        "route":{"collectionIds":[],"fallback":true}
    });
    match script.outcome {
        Outcome::ConciseAnswer => {
            result["answer"] = json!(document.text);
        }
        Outcome::ScoringOrder => {
            let suffix = ["", " x", " x y", " x y z q r"][index];
            result["answer"] = json!(format!("{}{suffix}", document.text));
        }
        Outcome::OtherCaseEvidence => {
            let foreign = script.benchmark.documents.last().unwrap();
            result["evidence"][0]["sourceId"] = json!(foreign.id);
            result["evidence"][0]["sourceRevision"] = json!(foreign.revision);
        }
        Outcome::EvidenceOnly | Outcome::Refused => {
            result["outcome"] = json!(if matches!(script.outcome, Outcome::Refused) {
                "refused"
            } else {
                "evidence-only"
            });
            result["reason"] = json!("No supported answer");
            result["answer"] = json!("");
            result["lineage"] = json!([]);
        }
        Outcome::MissingReceipt => {
            result.as_object_mut().unwrap().remove("modelReceipt");
        }
        Outcome::ChangedWatermark => {
            result["watermark"]["generation"] = json!("changed");
        }
        Outcome::ChangedModel => {
            result["modelReceipt"]["modelLabel"] = json!("Unexpected model");
        }
        Outcome::ForeignEvidence => {
            result["evidence"][0]["sourceId"] = json!("outside-the-benchmark");
        }
        Outcome::DuplicateEvidence => {
            let duplicate = result["evidence"][0].clone();
            result["evidence"].as_array_mut().unwrap().push(duplicate);
        }
        _ => {}
    }
    (StatusCode::OK, Json(result))
}

async fn scripted_nebula(
    benchmark: &Benchmark,
    outcome: Outcome,
    block: bool,
) -> (TestServer, Arc<Script>) {
    let script = Arc::new(Script {
        benchmark: benchmark.clone(),
        outcome,
        queries: Mutex::new(vec![]),
        conversations: Mutex::new(vec![]),
        query_started: Notify::new(),
        release_query: block.then(|| Arc::new(Semaphore::new(0))),
        release_cases: benchmark.cases.iter().map(|_| Semaphore::new(0)).collect(),
    });
    let app = Router::new()
        .route("/api/nebula/v1/workspace", get(workspace))
        .route("/api/nebula/v1/conversations", post(conversation))
        .route("/api/nebula/v1/query", post(query))
        .with_state(script.clone());
    (serve(app).await, script)
}

async fn benchmark_server(store: Arc<Store>, nebula: Option<&TestServer>) -> TestServer {
    serve(
        router(
            store,
            nebula.map(|server| NebulaConfig {
                base_url: format!("{}/api/nebula/v1", server.base),
                token: TOKEN.into(),
            }),
            Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap(),
        )
        .unwrap(),
    )
    .await
}

fn csv_rows(csv: &str) -> Vec<BTreeMap<String, String>> {
    csv::Reader::from_reader(csv.as_bytes())
        .deserialize()
        .map(|row| row.unwrap())
        .collect()
}

fn base(server: &TestServer) -> String {
    format!("{}/api/benchmarks/v1", server.base)
}

fn review(correct: bool) -> Value {
    json!({"reviewer":"Test reviewer", "correctness":correct, "groundedness":correct,
        "hallucination":!correct,"citation_accuracy":correct,"notes":"Independent review of this new answer"})
}

async fn start(client: &reqwest::Client, server: &TestServer, benchmark: &Benchmark) -> Value {
    response(client.post(format!("{}/answer-runs", base(server))).json(&json!({
        "benchmark_id":benchmark.id,"architecture_label":"Scripted architecture", "profile_id":PROFILE
    })), StatusCode::ACCEPTED).await
}

async fn wait_run(client: &reqwest::Client, server: &TestServer, id: &str) -> Value {
    for _ in 0..1000 {
        let run = response(
            client.get(format!("{}/answer-runs/{id}", base(server))),
            StatusCode::OK,
        )
        .await;
        if run["status"] != "running" {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("answer run did not stop");
}

async fn wait_queries(script: &Script, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while script.queries.lock().unwrap().len() < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_progress(
    client: &reqwest::Client,
    server: &TestServer,
    id: &str,
    processed: u64,
) -> Value {
    for _ in 0..100 {
        let run = response(
            client.get(format!("{}/answer-runs/{id}", base(server))),
            StatusCode::OK,
        )
        .await;
        if run["completed"].as_u64().unwrap() + run["failed"].as_u64().unwrap() >= processed {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("answer run did not persist progress");
}

async fn csv(client: &reqwest::Client, server: &TestServer, id: &str) -> String {
    let response = client
        .get(format!("{}/answer-runs/{id}/scores.csv", base(server)))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/csv")
    );
    response.text().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_answers_retain_provenance_and_only_explicit_reviews_produce_metrics() {
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let info = store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::Answered, false).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let runtime = response(
        client.get(format!("{}/answer-runtime", base(&server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(runtime["available"], true, "{runtime}");
    assert_eq!(runtime["embedding_model"]["id"], "scripted-embedding");
    assert_eq!(runtime["profiles"][0]["id"], PROFILE);
    assert!(!runtime.to_string().contains(TOKEN));

    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    let result = wait_run(&client, &server, id).await;
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["evaluation"], "manual_review_v1");
    assert_eq!(result["benchmark_fingerprint"], info.fingerprint);
    assert_eq!(result["top_k"], 8);
    assert_eq!(
        result["generation_model"],
        json!({"profile_id":PROFILE,"label":"Kimi K3"})
    );
    assert_eq!(
        result["embedding_model"],
        json!({"id":"scripted-embedding","revision":"embedding-revision-1"})
    );
    assert_eq!(result["completed"], 2);
    assert_eq!(result["answered"], 2);
    assert_eq!(result["reviewed"], 0);
    assert!(
        result["means"].is_null(),
        "historical labels must not grade new answers"
    );
    assert_eq!(result["source_ids"].as_array().unwrap().len(), 2);
    for (index, case) in result["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            case["answer"],
            format!("Generated answer: {}", benchmark.documents[index].text)
        );
        assert_eq!(case["model_receipt"]["profileId"], PROFILE);
        assert_eq!(case["model_receipt"]["modelLabel"], "Kimi K3");
        assert_eq!(case["model_receipt"]["route"], "remote");
        assert_eq!(
            case["evidence"][0]["sourceRevision"],
            benchmark.documents[index].revision
        );
        assert!(case["review"].is_null());
    }
    assert!(!result.to_string().contains("HISTORICAL OUTPUT"));
    let queries = script.queries.lock().unwrap().clone();
    assert_eq!(queries.len(), 2);
    assert_ne!(
        queries[0]["conversationId"], queries[1]["conversationId"],
        "cases must not share conversation history"
    );
    assert_eq!(script.conversations.lock().unwrap().len(), 2);

    let unreviewed = csv_rows(&csv(&client, &server, id).await);
    assert_eq!(unreviewed.len(), 2);
    for row in &unreviewed {
        for field in [
            "correctness",
            "groundedness",
            "hallucination",
            "citation_accuracy",
        ] {
            assert_eq!(
                row.get(field).map(String::as_str),
                Some(""),
                "unreviewed {field}: {row:?}"
            );
        }
    }
    let first_url = format!("{}/answer-runs/{id}/cases/case-0/review", base(&server));
    let reviewed = response(client.post(&first_url).json(&review(true)), StatusCode::OK).await;
    assert_eq!(reviewed["reviewed"], 1);
    assert_eq!(
        reviewed["means"],
        json!({"correctness":1.0,"groundedness":1.0,"hallucination_rate":0.0,"citation_accuracy":1.0}),
        "the unreviewed second answer must not enter any denominator"
    );
    let second_url = format!("{}/answer-runs/{id}/cases/case-1/review", base(&server));
    let reviewed = response(client.post(second_url).json(&review(false)), StatusCode::OK).await;
    assert_eq!(reviewed["reviewed"], 2);
    assert_eq!(
        reviewed["means"],
        json!({"correctness":0.5,"groundedness":0.5,"hallucination_rate":0.5,"citation_accuracy":0.5})
    );
    let replaced = response(client.post(first_url).json(&review(false)), StatusCode::OK).await;
    assert_eq!(
        replaced["reviewed"], 2,
        "replacement must not increase review count"
    );
    assert_eq!(replaced["means"]["correctness"], 0.0);
    let saved: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("answer-runs").join(id).join("run.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["cases"][0]["review"]["correctness"], false);
    assert_eq!(saved["cases"][1]["answer"], result["cases"][1]["answer"]);
    let listed = response(
        client.get(format!("{}/answer-runs", base(&server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(listed[0]["reviewed"], 2);
    assert!(
        listed[0].get("cases").is_none(),
        "list endpoint should return summaries"
    );
    let reviewed_csv = csv_rows(&csv(&client, &server, id).await);
    assert_ne!(reviewed_csv[0]["correctness"], "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_without_nebula_is_unavailable_and_never_creates_a_fake_run() {
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let server = benchmark_server(store, None).await;
    let client = client();
    let runtime = response(
        client.get(format!("{}/answer-runtime", base(&server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(runtime["available"], false);
    assert!(
        runtime["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert!(runtime["embedding_model"].is_null());
    response(
        client
            .post(format!("{}/answer-runs", base(&server)))
            .json(&json!({
                "benchmark_id":benchmark.id,"architecture_label":"No runtime", "profile_id":PROFILE
            })),
        StatusCode::SERVICE_UNAVAILABLE,
    )
    .await;
    assert_eq!(
        response(
            client.get(format!("{}/answer-runs", base(&server))),
            StatusCode::OK
        )
        .await,
        json!([])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_reads_the_model_from_the_real_nebula_http_workspace_shape() {
    // Frozen, sanitized shape from the live Nebula workspace captured 2026-09-20.
    // internal/protocol.KnowledgeStatus exposes `model` directly; the portable
    // rag/contract KnowledgeStatus `embedding.model` is not the HTTP contract.
    let captured: Value = serde_json::from_str(r#"{
      "scope":{"projectId":"fixture","checkoutId":"fixture","projectName":"Fixture",
        "corpusId":"fixture","corpusName":"Fixture","access":"private","routing":"local-only"},
      "status":{"phase":"ready","stage":"complete","summary":"Ready",
        "sourceCount":0,"indexedSourceCount":0,"pendingSourceCount":0,"unsearchableSourceCount":0,
        "embeddedChunkCount":0,"totalChunkCount":0,"pendingEmbeddingCount":0,"progress":1,
        "model":{"id":"intfloat/multilingual-e5-small","revision":"614241f622f53c4eeff9890bdc4f31cfecc418b3",
          "phase":"ready","summary":"The pinned local embedding model completed the active corpus."},
        "watermark":{"generation":"fixture","collectionGeneration":"fixture","overlaySequence":0,"pipelineFingerprint":"fixture"}},
      "profiles":[{"id":"moonshot-kimi-k3","label":"Kimi K3",
        "description":"Moonshot China reasoning over selected, citation-bound project evidence.",
        "execution":"direct-session","enabled":false,
        "disabledReason":"Start an explicitly remote-enabled reasoning session to use this profile."}],
      "conversations":[],"sources":[],"collections":[]
    }"#).unwrap();
    let workspace = Arc::new(Mutex::new(captured));
    let captured = workspace.clone();
    let nebula = serve(Router::new().route(
        "/api/nebula/v1/workspace",
        get(move |headers: HeaderMap| {
            let captured = captured.lock().unwrap().clone();
            async move {
                assert_auth(&headers);
                Json(captured)
            }
        }),
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let server = benchmark_server(Arc::new(Store::open(dir.path()).unwrap()), Some(&nebula)).await;
    let runtime = response(
        client().get(format!("{}/answer-runtime", base(&server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        runtime["embedding_model"],
        json!({
            "id":"intfloat/multilingual-e5-small","revision":"614241f622f53c4eeff9890bdc4f31cfecc418b3"
        })
    );
    assert_eq!(
        runtime["available"], false,
        "an installed embedding model is not a configured generator"
    );
    assert_eq!(runtime["profiles"][0]["enabled"], false);
    assert_eq!(
        runtime["profiles"][0]["disabled_reason"],
        "Start an explicitly remote-enabled reasoning session to use this profile."
    );

    // A portable-engine-shaped fixture must not accidentally make this HTTP
    // integration pass: without the public `status.model`, identity is unknown.
    {
        let mut nested = workspace.lock().unwrap();
        let model = nested["status"]
            .as_object_mut()
            .unwrap()
            .remove("model")
            .unwrap();
        nested["status"]["embedding"] = json!({"model":model});
        nested["profiles"][0]["enabled"] = json!(true);
    }
    let nested = response(
        client().get(format!("{}/answer-runtime", base(&server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(nested["available"], false);
    assert!(nested["embedding_model"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_requires_ready_embedding_and_an_enabled_generation_profile() {
    for outcome in [Outcome::UnreadyEmbedding, Outcome::DisabledProfile] {
        let dir = TempDir::new().unwrap();
        let benchmark = benchmark();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.save_benchmark(&benchmark).unwrap();
        let (nebula, script) = scripted_nebula(&benchmark, outcome, false).await;
        let server = benchmark_server(store, Some(&nebula)).await;
        let client = client();
        let runtime = response(
            client.get(format!("{}/answer-runtime", base(&server))),
            StatusCode::OK,
        )
        .await;
        assert_eq!(runtime["available"], false, "{runtime}");
        assert!(
            runtime["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
        let response = client.post(format!("{}/answer-runs", base(&server))).json(&json!({
            "benchmark_id":benchmark.id,"architecture_label":"Missing runtime configuration", "profile_id":PROFILE
        })).send().await.unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_REQUEST
        ));
        assert!(script.queries.lock().unwrap().is_empty());
        assert!(script.conversations.lock().unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_generation_failures_retain_answers_but_do_not_invent_evaluation_scores() {
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, _) = scripted_nebula(&benchmark, Outcome::FailSecond, false).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    let result = wait_run(&client, &server, id).await;
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["completed"], 1);
    assert_eq!(result["answered"], 1);
    assert_eq!(result["failed"], 1);
    assert_eq!(result["cases"].as_array().unwrap().len(), 2);
    assert_eq!(result["cases"][0]["status"], "ok");
    assert_eq!(result["cases"][1]["status"], "error");
    assert!(result["cases"][1]["answer"].is_null());
    assert!(result["means"].is_null());
    response(
        client
            .post(format!(
                "{}/answer-runs/{id}/cases/case-1/review",
                base(&server)
            ))
            .json(&review(true)),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let reviewed = response(
        client
            .post(format!(
                "{}/answer-runs/{id}/cases/case-0/review",
                base(&server)
            ))
            .json(&review(true)),
        StatusCode::OK,
    )
    .await;
    assert_eq!(reviewed["reviewed"], 1);
    assert_eq!(reviewed["means"]["correctness"], 1.0);
    let rows = csv_rows(&csv(&client, &server, id).await);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["correctness"], "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_answers_are_neither_reviewable_nor_scored_as_wrong_answers() {
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, _) = scripted_nebula(&benchmark, Outcome::EvidenceOnly, false).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    let result = wait_run(&client, &server, id).await;
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["completed"], 2);
    assert_eq!(result["answered"], 0);
    assert_eq!(result["failed"], 0);
    assert!(result["means"].is_null());
    response(
        client
            .post(format!(
                "{}/answer-runs/{id}/cases/case-0/review",
                base(&server)
            ))
            .json(&review(false)),
        StatusCode::BAD_REQUEST,
    )
    .await;
    for row in csv_rows(&csv(&client, &server, id).await) {
        assert_eq!(row["correctness"], "");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generation_rejects_missing_or_changed_provenance_and_unpinned_evidence() {
    for outcome in [
        Outcome::MissingReceipt,
        Outcome::ChangedWatermark,
        Outcome::ChangedModel,
        Outcome::ForeignEvidence,
        Outcome::DuplicateEvidence,
    ] {
        let dir = TempDir::new().unwrap();
        let benchmark = benchmark();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.save_benchmark(&benchmark).unwrap();
        let (nebula, script) = scripted_nebula(&benchmark, outcome, false).await;
        let server = benchmark_server(store, Some(&nebula)).await;
        let client = client();
        let created = start(&client, &server, &benchmark).await;
        let result = wait_run(&client, &server, created["id"].as_str().unwrap()).await;
        assert_eq!(result["status"], "failed", "{result}");
        assert_eq!(result["answered"], 0);
        assert_eq!(result["failed"], 2);
        assert!(result["means"].is_null());
        assert_eq!(
            script.queries.lock().unwrap().len(),
            2,
            "drain already started cases after invalid provenance"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reused_conversations_stop_before_a_second_question_can_leak_history() {
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::ReusedConversation, false).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let result = wait_run(&client, &server, created["id"].as_str().unwrap()).await;
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["answered"], 1);
    assert_eq!(result["failed"], 1);
    assert_eq!(
        result["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["status"] == "error")
            .count(),
        1
    );
    assert_eq!(script.conversations.lock().unwrap().len(), 2);
    assert_eq!(script.queries.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_answer_runs_share_retrieval_slot_and_reject_early_reviews_and_csv() {
    let dir = TempDir::new().unwrap();
    let mut benchmark = benchmark();
    benchmark.cases.truncate(1);
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::Answered, true).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(3), script.query_started.notified())
        .await
        .unwrap();
    response(
        client
            .post(format!("{}/runs", base(&server)))
            .json(&json!({"benchmark_id":benchmark.id,"label":"retrieval"})),
        StatusCode::CONFLICT,
    )
    .await;
    response(client.post(format!("{}/answer-runs", base(&server))).json(&json!({"benchmark_id":benchmark.id,"architecture_label":"Second answer run","profile_id":PROFILE})), StatusCode::CONFLICT).await;
    response(
        client
            .post(format!(
                "{}/answer-runs/{id}/cases/case-0/review",
                base(&server)
            ))
            .json(&review(true)),
        StatusCode::CONFLICT,
    )
    .await;
    response(
        client.get(format!("{}/answer-runs/{id}/scores.csv", base(&server))),
        StatusCode::CONFLICT,
    )
    .await;
    script.release_query.as_ref().unwrap().add_permits(1);
    assert_eq!(wait_run(&client, &server, id).await["status"], "completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_validation_and_interrupted_recovery_preserve_successful_answers() {
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, _) = scripted_nebula(&benchmark, Outcome::Answered, false).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    let result = wait_run(&client, &server, id).await;
    let review_url = format!("{}/answer-runs/{id}/cases/case-0/review", base(&server));
    for (field, value) in [
        ("reviewer", json!(" ")),
        ("reviewer", json!("é".repeat(65))),
        ("notes", json!("é".repeat(2001))),
    ] {
        let mut invalid = review(true);
        invalid[field] = value;
        response(
            client.post(&review_url).json(&invalid),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let mut unknown = review(true);
    unknown["automatic_judge"] = json!(true);
    let status = client
        .post(&review_url)
        .json(&unknown)
        .send()
        .await
        .unwrap()
        .status();
    assert!(
        status.is_client_error(),
        "unknown judgement fields must be rejected"
    );
    response(
        client
            .post(format!(
                "{}/answer-runs/{id}/cases/missing/review",
                base(&server)
            ))
            .json(&review(true)),
        StatusCode::NOT_FOUND,
    )
    .await;
    let mut valid = review(true);
    valid["reviewer"] = json!("  Test reviewer  ");
    let reviewed = response(client.post(&review_url).json(&valid), StatusCode::OK).await;
    assert_eq!(reviewed["cases"][0]["review"]["reviewer"], "Test reviewer");

    // Reproduce a process exit after one durable answer, before final status was saved.
    let recovery_dir = TempDir::new().unwrap();
    let mut interrupted = result;
    interrupted["status"] = json!("running");
    interrupted["finished_at_ms"] = Value::Null;
    interrupted["completed"] = json!(1);
    interrupted["answered"] = json!(1);
    interrupted["cases"].as_array_mut().unwrap().truncate(1);
    let run_dir = recovery_dir.path().join("answer-runs").join(id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("run.json"),
        serde_json::to_vec(&interrupted).unwrap(),
    )
    .unwrap();
    let recovered_store = Arc::new(Store::open(recovery_dir.path()).unwrap());
    let recovered_server = benchmark_server(recovered_store, None).await;
    let recovered = response(
        client.get(format!("{}/answer-runs/{id}", base(&recovered_server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(recovered["status"], "interrupted", "{recovered}");
    assert_eq!(
        recovered["cases"][0]["answer"],
        interrupted["cases"][0]["answer"]
    );
    assert_eq!(recovered["answered"], 1);
    assert!(recovered["finished_at_ms"].is_number());
    assert!(recovered["means"].is_null());
    let recovered_list = response(
        client.get(format!("{}/answer-runs", base(&recovered_server))),
        StatusCode::OK,
    )
    .await;
    assert_eq!(recovered_list[0]["status"], "interrupted");
    assert_eq!(recovered_list[0]["answered"], 1);
    assert_eq!(
        csv_rows(&csv(&client, &recovered_server, id).await).len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summary_polling_uses_cache_updated_only_by_successful_persistence_and_reloads_on_startup()
{
    let dir = TempDir::new().unwrap();
    let benchmark = benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, _) = scripted_nebula(&benchmark, Outcome::Answered, false).await;
    let server = benchmark_server(store.clone(), Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    let mut detail = wait_run(&client, &server, id).await;
    assert_eq!(detail["status"], "completed");
    let listing_url = format!("{}/answer-runs", base(&server));
    let detail_url = format!("{listing_url}/{id}");
    let original = response(client.get(&listing_url), StatusCode::OK).await;
    let saved_path = dir.path().join("answer-runs").join(id).join("run.json");

    // Removing and then editing the full detail proves the poll endpoint does
    // not deserialize generated answers/evidence on every request.
    std::fs::remove_file(&saved_path).unwrap();
    assert_eq!(
        response(client.get(&listing_url), StatusCode::OK).await,
        original
    );
    response(client.get(&detail_url), StatusCode::NOT_FOUND).await;
    detail["request"]["architecture_label"] = json!("Restored disk record");
    std::fs::write(&saved_path, serde_json::to_vec(&detail).unwrap()).unwrap();
    assert_eq!(
        response(client.get(&listing_url), StatusCode::OK).await,
        original
    );
    assert_eq!(
        response(client.get(&detail_url), StatusCode::OK).await["request"]["architecture_label"],
        "Restored disk record"
    );

    let reviewed = response(
        client
            .post(format!("{detail_url}/cases/case-0/review"))
            .json(&review(true)),
        StatusCode::OK,
    )
    .await;
    let current = response(client.get(&listing_url), StatusCode::OK).await;
    assert_eq!(
        current[0]["request"]["architecture_label"],
        "Restored disk record"
    );
    assert_eq!(current[0]["reviewed"], 1);
    assert_eq!(current[0]["means"]["correctness"], 1.0);

    // Force an atomic-save failure without platform-specific permission bits.
    let mut unsaved: AnswerRun = serde_json::from_value(reviewed.clone()).unwrap();
    unsaved.summary.request.architecture_label = "Rejected write".into();
    std::fs::remove_file(&saved_path).unwrap();
    std::fs::create_dir(&saved_path).unwrap();
    assert!(store.save_answer_run(&unsaved).is_err());
    assert_eq!(
        response(client.get(&listing_url), StatusCode::OK).await,
        current
    );
    std::fs::remove_dir(&saved_path).unwrap();
    std::fs::write(&saved_path, serde_json::to_vec(&reviewed).unwrap()).unwrap();
    assert!(
        store.create_answer_run(&unsaved).is_err(),
        "duplicate creation must fail"
    );
    assert_eq!(
        response(client.get(&listing_url), StatusCode::OK).await,
        current
    );

    // A new process reconstructs its cache from durable reviewed records only.
    let restarted_dir = TempDir::new().unwrap();
    let restarted_run = restarted_dir.path().join("answer-runs").join(id);
    std::fs::create_dir_all(&restarted_run).unwrap();
    std::fs::copy(&saved_path, restarted_run.join("run.json")).unwrap();
    let restarted = Store::open(restarted_dir.path()).unwrap();
    assert_eq!(
        serde_json::to_value(restarted.answer_runs().unwrap()).unwrap(),
        current
    );
    assert_eq!(
        restarted.answer_run(id.parse().unwrap()).unwrap().cases[0]
            .review
            .as_ref()
            .unwrap()
            .review
            .reviewer,
        "Test reviewer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hotpot_answers_use_per_question_candidates_keep_gold_private_and_score_full_answers() {
    let dir = TempDir::new().unwrap();
    let benchmark = hotpot_benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::Answered, false).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    assert_eq!(created["evaluation"], "hotpotqa_answer_v1");
    assert_eq!(
        created["automatic_scores"],
        json!({"exact_match":0.0,"f1":0.0})
    );
    let id = created["id"].as_str().unwrap();
    let run = wait_run(&client, &server, id).await;
    assert_eq!(run["status"], "completed");
    assert_eq!(run["scored"], 2);
    assert_eq!(run["automatic_scores"], json!({"exact_match":0.0,"f1":0.5}));
    assert!(run["means"].is_null());
    assert_eq!(run["reviewed"], 0);
    assert_eq!(run["cases"][0]["reference_answer"], "London");
    assert_eq!(run["cases"][0]["answer"], "Generated answer: London");
    for body in script
        .queries
        .lock()
        .unwrap()
        .iter()
        .chain(script.conversations.lock().unwrap().iter())
    {
        let encoded = body.to_string();
        for private in [
            "London",
            "Paris",
            "GOLD SUPPORT",
            "supporting_facts",
            "answer_reference",
        ] {
            assert!(
                !encoded.contains(private),
                "Gold data leaked to generation: {private}"
            );
        }
    }
    let rows = csv_rows(&csv(&client, &server, id).await);
    assert_eq!(rows[0]["evaluation"], "hotpotqa_answer_v1");
    assert_eq!(rows[0]["reference_answer"], "London");
    assert_eq!(rows[0]["exact_match"], "0");
    assert_eq!(rows[0]["f1"], "0.5");
    let reviewed = response(
        client
            .post(format!(
                "{}/answer-runs/{id}/cases/case-0/review",
                base(&server)
            ))
            .json(&review(true)),
        StatusCode::OK,
    )
    .await;
    assert_eq!(reviewed["automatic_scores"], run["automatic_scores"]);
    assert_eq!(reviewed["means"]["correctness"], 1.0);
    response(
        client
            .post(format!("{}/runs", base(&server)))
            .json(&json!({"benchmark_id":benchmark.id})),
        StatusCode::BAD_REQUEST,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hotpot_scores_use_all_selected_cases_as_denominator_while_running() {
    let dir = TempDir::new().unwrap();
    let benchmark = hotpot_benchmark();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::ConciseAnswer, true).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    wait_queries(&script, 2).await;
    script.release_query.as_ref().unwrap().add_permits(1);
    let partial = wait_progress(&client, &server, id, 1).await;
    assert_eq!(partial["status"], "running");
    assert_eq!(partial["scored"], 1);
    assert_eq!(
        partial["automatic_scores"],
        json!({"exact_match":0.5,"f1":0.5})
    );
    script.release_query.as_ref().unwrap().add_permits(1);
    let run = wait_run(&client, &server, id).await;
    assert_eq!(run["scored"], 2);
    assert_eq!(run["automatic_scores"], json!({"exact_match":1.0,"f1":1.0}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hotpot_nonanswers_and_errors_score_zero_without_inflating_partial_results() {
    for (outcome, status, completed, failed, scored, f1) in [
        (Outcome::EvidenceOnly, "completed", 2, 0, 2, 0.0),
        (Outcome::Refused, "completed", 2, 0, 2, 0.0),
        (Outcome::FailSecond, "failed", 99, 1, 100, 0.495),
        (Outcome::OtherCaseEvidence, "failed", 1, 1, 2, 0.25),
    ] {
        let dir = TempDir::new().unwrap();
        let mut benchmark = hotpot_benchmark();
        if matches!(outcome, Outcome::FailSecond) {
            for index in 2..100 {
                let mut unprocessed = benchmark.cases[0].clone();
                unprocessed.id = format!("unprocessed-{index}");
                unprocessed.query = format!("Unprocessed question {index}?");
                benchmark.cases.push(unprocessed);
            }
        }
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.save_benchmark(&benchmark).unwrap();
        let (nebula, _) = scripted_nebula(&benchmark, outcome, false).await;
        let server = benchmark_server(store, Some(&nebula)).await;
        let client = client();
        let created = start(&client, &server, &benchmark).await;
        let run = wait_run(&client, &server, created["id"].as_str().unwrap()).await;
        assert_eq!(run["status"], status);
        assert_eq!(run["completed"], completed);
        assert_eq!(run["failed"], failed);
        assert_eq!(run["scored"], scored);
        assert_eq!(run["automatic_scores"]["exact_match"], 0.0);
        assert!((run["automatic_scores"]["f1"].as_f64().unwrap() - f1).abs() < 1e-9);
        let rows = csv_rows(&csv(&client, &server, created["id"].as_str().unwrap()).await);
        for row in &rows {
            assert_eq!(row["run_status"], status);
            assert_eq!(row["run_total"], benchmark.cases.len().to_string());
            assert_eq!(row["run_scored"], scored.to_string());
            assert_eq!(row["run_exact_match"], "0");
            assert!((row["run_f1"].parse::<f64>().unwrap() - f1).abs() < 1e-9);
        }
        if matches!(outcome, Outcome::FailSecond) {
            assert_eq!(rows.len(), 100);
            assert_eq!(rows[0]["f1"], "0.5");
            assert_eq!(rows[0]["run_total"], "100");
            assert_eq!(rows[1]["f1"], "0");
        }
        for case in run["cases"].as_array().unwrap() {
            if case["status"] == "error" || case["outcome"] != "answered" {
                assert_eq!(
                    case["automatic_scores"],
                    json!({"exact_match":0.0,"f1":0.0})
                );
            }
        }
        if matches!(outcome, Outcome::OtherCaseEvidence) {
            assert!(
                run["error"]
                    .as_str()
                    .unwrap()
                    .contains("outside the pinned")
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hotpot_validates_candidate_selections_individually_instead_of_the_large_union() {
    let mut benchmark = hotpot_benchmark();
    // A realistic large distractor union exceeds Nebula's per-request 64 KiB cap.
    // Each question still sends only its ten candidates.
    for index in 11..1100 {
        let id = digest(format!("large-union-document-{index}").as_bytes());
        benchmark.documents.push(Document {
            id: id.clone(),
            filename: format!("hotpotqa-{id}.md"),
            revision: id,
            text: format!("Document {index}"),
        });
    }
    let (nebula, _) = scripted_nebula(&benchmark, Outcome::Answered, false).await;
    let config = NebulaConfig {
        base_url: format!("{}/api/nebula/v1", nebula.base),
        token: TOKEN.into(),
    };
    let request = backend::answers::StartAnswerRunRequest {
        benchmark_id: benchmark.id,
        architecture_label: "Large candidate union".into(),
        profile_id: PROFILE.into(),
    };
    let run =
        backend::answers::prepare(&config, request.clone(), &benchmark, "fingerprint".into()).await;
    assert!(
        run.is_ok(),
        "A large union must not reject a small per-case selection"
    );
    assert_eq!(
        run.unwrap_or_else(|_| unreachable!())
            .summary
            .source_ids
            .len(),
        1100
    );
    benchmark.cases[0]
        .answer_reference
        .as_mut()
        .unwrap()
        .candidate_document_ids[0] = "missing-candidate".into();
    assert!(matches!(
        backend::answers::prepare(&config, request, &benchmark, "fingerprint".into()).await,
        Err(backend::answers::StartFailure::Invalid(_))
    ));
}

fn repeated_benchmark(count: usize) -> Benchmark {
    let mut benchmark = benchmark();
    benchmark.cases = (0..count)
        .map(|index| {
            let mut case = benchmark.cases[index % 2].clone();
            case.id = format!("case-{index}");
            case.query = format!("Question {index}: {}", case.query);
            case
        })
        .collect();
    benchmark
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generation_waits_for_the_entire_four_request_batch_and_persists_each_completion() {
    let dir = TempDir::new().unwrap();
    let benchmark = repeated_benchmark(6);
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::Answered, true).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["max_in_flight"], 4);
    wait_queries(&script, 4).await;
    assert_eq!(script.conversations.lock().unwrap().len(), 4);
    script.release_query.as_ref().unwrap().add_permits(3);
    let partial = wait_progress(&client, &server, id, 3).await;
    assert_eq!(partial["status"], "running");
    assert_eq!(partial["cases"].as_array().unwrap().len(), 3);
    let ids = partial["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["case_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        ids.windows(2).all(|pair| pair[0] < pair[1]),
        "partial results retain dataset order"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        script.queries.lock().unwrap().len(),
        4,
        "no fifth query until all four responses arrive"
    );
    assert_eq!(
        script.conversations.lock().unwrap().len(),
        4,
        "no fifth conversation before the batch finishes"
    );
    script.release_query.as_ref().unwrap().add_permits(1);
    wait_queries(&script, 6).await;
    let partial = wait_progress(&client, &server, id, 4).await;
    assert_eq!(partial["completed"], 4);
    script.release_query.as_ref().unwrap().add_permits(2);
    let run = wait_run(&client, &server, id).await;
    assert_eq!(run["status"], "completed");
    assert_eq!(run["completed"], 6);
    assert_eq!(run["failed"], 0);
    for (index, case) in run["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(case["case_id"], format!("case-{index}"));
    }
    let mut conversations = script
        .queries
        .lock()
        .unwrap()
        .iter()
        .map(|query| query["conversationId"].to_string())
        .collect::<Vec<_>>();
    conversations.sort();
    conversations.dedup();
    assert_eq!(conversations.len(), 6);
    assert!(
        !dir.path()
            .join("answer-runs")
            .join(id)
            .join("failures.jsonl")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_failures_continue_remaining_batches_and_are_safely_logged_and_exported() {
    for outcome in [Outcome::FailSecond, Outcome::UnsafeFailure] {
        let dir = TempDir::new().unwrap();
        let benchmark = repeated_benchmark(7);
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.save_benchmark(&benchmark).unwrap();
        let (nebula, script) = scripted_nebula(&benchmark, outcome, false).await;
        let server = benchmark_server(store.clone(), Some(&nebula)).await;
        let client = client();
        let created = start(&client, &server, &benchmark).await;
        let id = created["id"].as_str().unwrap();
        let run = wait_run(&client, &server, id).await;
        assert_eq!(run["status"], "failed");
        assert_eq!(run["total"], 7);
        assert_eq!(run["completed"], 6);
        assert_eq!(run["failed"], 1);
        assert_eq!(run["answered"], 6);
        assert_eq!(script.queries.lock().unwrap().len(), 7);
        assert_eq!(run["cases"].as_array().unwrap().len(), 7);
        assert_eq!(run["cases"][6]["status"], "ok");
        let failure = &run["cases"][1]["failure"];
        assert!(failure["timestamp_ms"].as_u64().unwrap() > 0);
        assert_eq!(failure["operation"], "generate_answer");
        if matches!(outcome, Outcome::FailSecond) {
            assert_eq!(failure["http_status"], 422);
            assert_eq!(failure["code"], "reasoning_rejected");
            assert_eq!(failure["provider_diagnostic"]["upstreamStatus"], 400);
            assert_eq!(
                failure["provider_diagnostic"]["upstreamCode"],
                "context_length_exceeded"
            );
            assert_eq!(failure["provider_diagnostic"]["requestBytes"], 12456);
            assert!(failure["provider_diagnostic"].get("requestId").is_none());
            assert!(failure["provider_diagnostic"].get("model").is_none());
        } else {
            assert_eq!(failure["http_status"], 502);
            assert_eq!(failure["code"], "http_error");
            assert!(failure.get("provider_diagnostic").is_none());
        }
        let log = std::fs::read_to_string(
            dir.path()
                .join("answer-runs")
                .join(id)
                .join("failures.jsonl"),
        )
        .unwrap();
        let events = log
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["run_id"], id);
        assert_eq!(events[0]["case_id"], "case-1");
        assert_eq!(events[0]["latency_ms"], run["cases"][1]["latency_ms"]);
        assert_eq!(events[0]["code"], failure["code"]);
        let exported = csv(&client, &server, id).await;
        let rows = csv_rows(&exported);
        assert_eq!(rows[1]["max_in_flight"], "4");
        assert_eq!(
            serde_json::from_str::<Value>(&rows[1]["failure_json"]).unwrap(),
            *failure
        );
        for output in [run.to_string(), log, exported] {
            assert!(!output.contains("DO-NOT-LOG"));
            assert!(!output.contains(TOKEN));
        }
        let saved = store.answer_run(id.parse().unwrap()).unwrap();
        assert!(saved.cases[1].failure.is_some());
        let mut legacy = serde_json::to_value(saved).unwrap();
        legacy.as_object_mut().unwrap().remove("max_in_flight");
        for case in legacy["cases"].as_array_mut().unwrap() {
            case.as_object_mut().unwrap().remove("failure");
        }
        let legacy: AnswerRun = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.summary.max_in_flight, 1);
        assert!(legacy.cases.iter().all(|case| case.failure.is_none()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fatal_provenance_failure_drains_its_batch_and_never_starts_the_next_batch() {
    let dir = TempDir::new().unwrap();
    let benchmark = repeated_benchmark(8);
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store.save_benchmark(&benchmark).unwrap();
    let (nebula, script) = scripted_nebula(&benchmark, Outcome::ChangedWatermark, true).await;
    let server = benchmark_server(store, Some(&nebula)).await;
    let client = client();
    let created = start(&client, &server, &benchmark).await;
    let id = created["id"].as_str().unwrap();
    wait_queries(&script, 4).await;
    script.release_query.as_ref().unwrap().add_permits(1);
    let partial = wait_progress(&client, &server, id, 1).await;
    assert_eq!(
        partial["status"], "running",
        "drain active requests before marking terminal"
    );
    assert_eq!(partial["failed"], 1);
    script.release_query.as_ref().unwrap().add_permits(3);
    let run = wait_run(&client, &server, id).await;
    assert_eq!(run["status"], "failed");
    assert_eq!(run["failed"], 4);
    assert_eq!(run["completed"], 0);
    assert_eq!(script.queries.lock().unwrap().len(), 4);
    assert_eq!(script.conversations.lock().unwrap().len(), 4);
    assert_eq!(run["cases"][0]["failure"]["operation"], "validate_answer");
    assert_eq!(run["cases"][0]["failure"]["code"], "provenance_changed");
    let log = std::fs::read_to_string(
        dir.path()
            .join("answer-runs")
            .join(id)
            .join("failures.jsonl"),
    )
    .unwrap();
    assert_eq!(log.lines().count(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_provider_metadata_is_dropped_without_losing_the_case_failure() {
    for index in 0..6 {
        let dir = TempDir::new().unwrap();
        let benchmark = benchmark();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.save_benchmark(&benchmark).unwrap();
        let (nebula, _) =
            scripted_nebula(&benchmark, Outcome::MalformedDiagnostic(index), false).await;
        let server = benchmark_server(store, Some(&nebula)).await;
        let client = client();
        let created = start(&client, &server, &benchmark).await;
        let run = wait_run(&client, &server, created["id"].as_str().unwrap()).await;
        assert_eq!(run["status"], "failed");
        assert_eq!(run["completed"], 1);
        assert_eq!(run["failed"], 1);
        let failure = &run["cases"][1]["failure"];
        assert_eq!(failure["http_status"], 422);
        assert_eq!(failure["code"], "reasoning_rejected");
        assert!(
            failure.get("provider_diagnostic").is_none(),
            "invalid field variant {index}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_scores_are_identical_for_forward_and_reverse_completion_order() {
    let mut aggregates = vec![];
    for order in [[0, 1, 2, 3], [3, 2, 1, 0]] {
        let dir = TempDir::new().unwrap();
        let mut benchmark = hotpot_benchmark();
        for index in 2..4 {
            let mut case = benchmark.cases[index % 2].clone();
            case.id = format!("case-{index}");
            case.query = format!("Question {index}: {}", case.query);
            benchmark.cases.push(case);
        }
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.save_benchmark(&benchmark).unwrap();
        let (nebula, script) = scripted_nebula(&benchmark, Outcome::ScoringOrder, false).await;
        let server = benchmark_server(store, Some(&nebula)).await;
        let client = client();
        let created = start(&client, &server, &benchmark).await;
        let id = created["id"].as_str().unwrap();
        wait_queries(&script, 4).await;
        for (processed, index) in order.into_iter().enumerate() {
            script.release_cases[index].add_permits(1);
            wait_progress(&client, &server, id, (processed + 1) as u64).await;
        }
        let run = wait_run(&client, &server, id).await;
        assert_eq!(run["status"], "completed");
        assert_eq!(run["scored"], 4);
        for (index, case) in run["cases"].as_array().unwrap().iter().enumerate() {
            assert_eq!(case["case_id"], format!("case-{index}"));
        }
        aggregates.push(run["automatic_scores"].clone());
    }
    assert_eq!(
        aggregates[0], aggregates[1],
        "completion order must never affect the winner"
    );
    assert_eq!(aggregates[0]["exact_match"], 0.25);
    assert_eq!(
        aggregates[0]["f1"],
        (1.0 + 2.0 / 3.0 + 0.5 + 2.0 / 7.0) / 4.0
    );
}
