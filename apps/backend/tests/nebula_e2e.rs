//! Server startup checks and opt-in acceptance through both real executables.
use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use backend::Benchmark;
use polars::prelude::*;
use serde_json::{Value, json};

const NEBULA_TOKEN: &str = "nebula-acceptance-local";
const TIMEOUT: Duration = Duration::from_secs(180);

struct Process {
    child: Child,
    log: PathBuf,
}

impl Process {
    fn start(command: &mut Command, log: PathBuf) -> Self {
        let output = File::create(&log).unwrap();
        let child = command
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap_or_else(|error| panic!("Cannot start {command:?}: {error}"));
        Self { child, log }
    }

    fn assert_running(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "Process stopped; {}:\n{}",
            self.log.display(),
            fs::read_to_string(&self.log).unwrap()
        );
    }

    async fn port(&mut self, prefix: &str) -> u16 {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            self.assert_running();
            if let Some(port) = fs::read_to_string(&self.log)
                .unwrap()
                .lines()
                .find_map(|line| line.strip_prefix(prefix)?.parse().ok())
            {
                return port;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("No {prefix} before timeout; inspect {}", self.log.display());
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn fixture(root: &Path) {
    let mut frame = df!(
        "id" => ["aurora", "rainfall", "bread"],
        "query" => [
            "Where is the Aurora observatory located?",
            "Which instrument measures rainfall?",
            "What temperature should the sourdough bread bake at?",
        ],
        "context" => [
            "The Aurora observatory is located in Tromso, Norway. Scientists at the observatory study the northern lights in the Arctic sky.",
            "A rain gauge is the instrument used to measure rainfall. The gauge collects rainwater in a graduated cylinder and reports precipitation in millimetres.",
            "Bake the sourdough bread at 230 degrees Celsius for forty minutes. Allow the bread to cool on a wire rack before slicing.",
        ],
        "output" => ["HISTORICAL_OUTPUT_NOT_FOR_INDEXING"; 3],
        "task_type" => ["QA"; 3],
        "quality" => ["good"; 3],
        "model" => ["historical-fixture"; 3],
        "hallucination_labels" => ["[]"; 3]
    ).unwrap();
    ParquetWriter::new(File::create(root.join("qa.parquet")).unwrap())
        .finish(&mut frame)
        .unwrap();
    fs::write(
        root.join("benchmarks.yaml"),
        "benchmarks:\n  local-qa:\n    name: Local acceptance QA\n    source: qa.parquet\n    split: test\n    adapter: ragtruth_qa\n    evaluation: paired_context_recovery_v1\n    defaults:\n      limit: 3\n      top_k: 3\n",
    ).unwrap();
}

fn start_benchmark(root: &Path, nebula_port: Option<u16>) -> Process {
    let mut command = Command::new(env!("CARGO_BIN_EXE_backend"));
    command
        .env("BENCHMARK_ADDR", "127.0.0.1:0")
        .env_remove("BENCHMARK_API_TOKEN")
        .env("BENCHMARK_DATA_DIR", root.join("benchmark-data"))
        .env("BENCHMARK_CATALOG", root.join("benchmarks.yaml"))
        .env_remove("NEBULA_API_BASE")
        .env_remove("NEBULA_API_TOKEN");
    if let Some(port) = nebula_port {
        command.env("NEBULA_API_TOKEN", NEBULA_TOKEN).env(
            "NEBULA_API_BASE",
            format!("http://127.0.0.1:{port}/api/nebula/v1"),
        );
    }
    let log = if nebula_port.is_some() {
        "benchmark-run.log"
    } else {
        "benchmark-load.log"
    };
    Process::start(&mut command, root.join(log))
}

async fn response(request: reqwest::RequestBuilder, expected_status: u16) -> Value {
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    assert_eq!(status, expected_status, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn save_json(root: &Path, name: &str, value: &Value) {
    fs::write(root.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[tokio::test]
async fn benchmark_starts_without_credentials_on_loopback() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let mut server = start_benchmark(root.path(), None);
    let port = server.port("BENCHMARK_BACKEND_PORT=").await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let health = response(
        client.get(format!("http://127.0.0.1:{port}/api/benchmarks/v1/health")),
        200,
    )
    .await;
    assert_eq!(health["status"], "ok");
}

#[test]
fn benchmark_rejects_non_loopback_addresses() {
    let output = Command::new(env!("CARGO_BIN_EXE_backend"))
        .env("BENCHMARK_ADDR", "0.0.0.0:0")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("BENCHMARK_ADDR must use loopback"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires NEBULA_E2E_BINARY and a real installed NEBULA_E2E_MODEL_DIR bundle"]
async fn real_nebula_produces_benchmark_results() {
    let binary = fs::canonicalize(
        env::var_os("NEBULA_E2E_BINARY").expect("Set NEBULA_E2E_BINARY to a built nebula-backend"),
    )
    .unwrap();
    let model =
        fs::canonicalize(env::var_os("NEBULA_E2E_MODEL_DIR").expect(
            "Set NEBULA_E2E_MODEL_DIR to the installed intfloat-multilingual-e5-small bundle",
        ))
        .unwrap();
    let temporary;
    let root = if let Some(parent) = env::var_os("NEBULA_E2E_ARTIFACT_DIR") {
        fs::create_dir_all(&parent).unwrap();
        tempfile::Builder::new()
            .prefix("nebula-acceptance-")
            .tempdir_in(parent)
            .unwrap()
            .keep()
    } else {
        temporary = tempfile::tempdir().unwrap();
        temporary.path().to_path_buf()
    };
    println!("Acceptance artifacts: {}", root.display());
    fixture(&root);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let mut benchmark_server = start_benchmark(&root, None);
    let port = benchmark_server.port("BENCHMARK_BACKEND_PORT=").await;
    let base = format!("http://127.0.0.1:{port}/api/benchmarks/v1");
    assert_eq!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    let loaded = response(
        client
            .post(format!("{base}/benchmarks"))
            .json(&json!({"benchmark":"local-qa"})),
        201,
    )
    .await;
    save_json(&root, "loaded.json", &loaded);
    assert_eq!(loaded["case_count"], 3);
    let corpus = Path::new(loaded["corpus_path"].as_str().unwrap());
    let benchmark_id = loaded["id"].as_str().unwrap();
    let snapshot = response(client.get(format!("{base}/benchmarks/{benchmark_id}")), 200).await;
    let benchmark: Benchmark = serde_json::from_value(snapshot.clone()).unwrap();
    assert_eq!(benchmark.documents.len(), 3);
    for document in &benchmark.documents {
        assert_eq!(
            fs::read_to_string(corpus.join(&document.filename)).unwrap(),
            document.text
        );
        assert!(!document.text.contains("HISTORICAL_OUTPUT_NOT_FOR_INDEXING"));
    }
    save_json(&root, "benchmark.json", &snapshot);
    drop(benchmark_server);

    let storage = root.join("nebula-state");
    copy_directory(
        &model,
        &storage.join("models/intfloat-multilingual-e5-small"),
    );
    let mut nebula = Process::start(
        Command::new(binary)
            .args(["-addr", "127.0.0.1:0", "-corpus"])
            .arg(corpus)
            .arg("-module-storage")
            .arg(&storage)
            .env("NEBULA_API_TOKEN", NEBULA_TOKEN)
            .env("NEBULA_REMOTE_REASONING", "0"),
        root.join("nebula.log"),
    );
    let nebula_port = nebula.port("NEBULA_BACKEND_PORT=").await;
    let nebula_base = format!("http://127.0.0.1:{nebula_port}/api/nebula/v1");
    let deadline = Instant::now() + TIMEOUT;
    let workspace = loop {
        nebula.assert_running();
        let workspace = response(
            client
                .get(format!("{nebula_base}/workspace"))
                .bearer_auth(NEBULA_TOKEN),
            200,
        )
        .await;
        save_json(&root, "workspace.json", &workspace);
        if workspace["status"]["phase"] == "ready" {
            break workspace;
        }
        assert!(
            Instant::now() < deadline,
            "Nebula did not finish indexing: {workspace}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let mut benchmark_server = start_benchmark(&root, Some(nebula_port));
    let port = benchmark_server.port("BENCHMARK_BACKEND_PORT=").await;
    let base = format!("http://127.0.0.1:{port}/api/benchmarks/v1");
    let started = response(
        client
            .post(format!("{base}/runs"))
            .json(&json!({"benchmark_id":benchmark_id,"label":"real-nebula-acceptance"})),
        202,
    )
    .await;
    let run_id = started["id"].as_str().unwrap();
    let deadline = Instant::now() + TIMEOUT;
    let run = loop {
        nebula.assert_running();
        benchmark_server.assert_running();
        let run = response(client.get(format!("{base}/runs/{run_id}")), 200).await;
        save_json(&root, "run.json", &run);
        if run["status"] != "running" {
            break run;
        }
        assert!(Instant::now() < deadline, "Benchmark did not finish: {run}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert_eq!(run["status"], "completed", "{run}");
    assert!(run["error"].is_null());
    assert!(run["finished_at_ms"].as_u64().unwrap() >= run["started_at_ms"].as_u64().unwrap());
    assert_eq!(run["completed"], 3);
    assert_eq!(run["failed"], 0);
    assert_eq!(run["source_ids"].as_array().unwrap().len(), 3);
    assert_eq!(run["benchmark_fingerprint"], loaded["fingerprint"]);
    assert_eq!(run["scope"], workspace["scope"]);
    assert_eq!(run["watermark"], workspace["status"]["watermark"]);
    assert_eq!(run["means"]["context_hit_at_k"], 1.0);
    let csv = client
        .get(format!("{base}/runs/{run_id}/scores.csv"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    fs::write(root.join("scores.csv"), &csv).unwrap();
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let headers = reader.headers().unwrap().clone();
    let rows = reader.records().map(|row| row.unwrap()).collect::<Vec<_>>();
    assert_eq!(rows.len(), benchmark.cases.len());
    let column = |name| headers.iter().position(|header| header == name).unwrap();
    for (row, case) in rows.iter().zip(&benchmark.cases) {
        assert_eq!(&row[column("run_id")], run_id);
        assert_eq!(&row[column("case_id")], case.id);
        assert_eq!(&row[column("query")], case.query);
        assert_eq!(&row[column("expected_document_id")], case.document_id);
        assert_eq!(&row[column("status")], "ok");
        assert_eq!(&row[column("error")], "");
        for metric in ["context_hit_at_k", "reciprocal_rank_at_k", "ndcg_at_k"] {
            let score: f64 = row[column(metric)].parse().unwrap();
            assert!(score.is_finite() && score > 0.0 && score <= 1.0);
        }
        let evidence: Vec<Value> =
            serde_json::from_str(&row[column("retrieved_evidence_json")]).unwrap();
        assert!(!evidence.is_empty());
        assert!(
            evidence
                .iter()
                .any(|item| item["sourceId"] == row[column("expected_source_id")])
        );
        for item in evidence {
            let source = workspace["sources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|source| source["id"] == item["sourceId"])
                .unwrap();
            let document = benchmark
                .documents
                .iter()
                .find(|document| source["title"] == document.filename)
                .unwrap();
            assert_eq!(item["sourceRevision"], document.revision);
            assert!(item["score"].as_f64().unwrap().is_finite());
            let excerpt = item["excerpt"].as_str().unwrap();
            assert!(!excerpt.is_empty());
            assert!(
                document.text.contains(excerpt),
                "Evidence must come from its claimed source: {item}"
            );
        }
    }
    println!("Completed real Nebula run {run_id}: {}", run["means"]);
}
