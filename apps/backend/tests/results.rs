//! Benchmark-level result tables and run references require no Nebula or network.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

use backend::{AnswerRun, Benchmark, Run, RunStatus, server::Store};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

const SHARED_COLUMNS: &[&str] = &[
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
];

fn benchmark(metric: &str) -> Benchmark {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "source": "fixture", "split": "test",
        "metric_kind": metric, "cases": [], "documents": []
    }))
    .unwrap()
}

fn retrieval(snapshot: &Benchmark, id: Uuid, started_at_ms: u64) -> Run {
    serde_json::from_value(json!({
        "id": id,
        "request": {"benchmark_id": snapshot.id, "top_k": 4, "label": "Vector, then rerank\nrevision 2", "description": "Retrieval baseline"},
        "metric_kind": snapshot.metric_kind, "benchmark_fingerprint": "fixture-fingerprint",
        "status": "completed", "started_at_ms": started_at_ms, "finished_at_ms": started_at_ms + 10,
        "total": 1, "completed": 1, "failed": 0, "means": null,
        "error": null, "scope": null, "watermark": null, "source_ids": []
    }))
    .unwrap()
}

fn generation(snapshot: &Benchmark, id: Uuid, started_at_ms: u64) -> AnswerRun {
    let evaluation = if snapshot.metric_kind == "hotpotqa_answer_v1" {
        "hotpotqa_answer_v1"
    } else {
        "manual_review_v1"
    };
    serde_json::from_value(json!({
        "id": id,
        "request": {"benchmark_id": snapshot.id, "architecture_label": "Vector + Kimi", "profile_id": "kimi", "description": "Generation baseline"},
        "benchmark_fingerprint": "fixture-fingerprint", "evaluation": evaluation,
        "embedding_model": {"id": "local-embedding", "revision": "r1"},
        "generation_model": {"profile_id": "kimi", "label": "Kimi K3"},
        "top_k": 4, "max_in_flight": 4, "status": "completed",
        "started_at_ms": started_at_ms, "finished_at_ms": started_at_ms + 10,
        "total": 1, "completed": 1, "failed": 0, "answered": 1, "reviewed": 0,
        "means": null, "automatic_scores": null, "scored": 0,
        "error": null, "scope": {}, "watermark": {}, "source_ids": [],
        "cases": [{
            "case_id": "case-1", "query": "Where?", "status": "ok", "outcome": "answered",
            "answer": "London", "reason": null, "evidence": [], "lineage": [],
            "model_receipt": null, "latency_ms": 1, "error": null, "review": null
        }]
    }))
    .unwrap()
}

fn registry(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("results/runs.json")).unwrap()).unwrap()
}

fn table(root: &Path, benchmark: &str) -> (Vec<String>, Vec<BTreeMap<String, String>>) {
    let mut reader =
        csv::Reader::from_path(root.join("results").join(format!("{benchmark}.csv"))).unwrap();
    let headers: Vec<_> = reader.headers().unwrap().iter().map(String::from).collect();
    let records = reader
        .records()
        .map(|record| {
            headers
                .iter()
                .cloned()
                .zip(record.unwrap().iter().map(String::from))
                .collect()
        })
        .collect();
    (headers, records)
}

fn write_legacy(root: &Path, directory: &str, id: Uuid, run: &impl serde::Serialize) {
    let path = root.join(directory).join(id.to_string());
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("run.json"), serde_json::to_vec(run).unwrap()).unwrap();
}

#[test]
fn concurrent_retrieval_and_generation_get_unique_global_run_numbers() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(Store::open(root.path()).unwrap());
    let snapshot = Arc::new(benchmark("paired_context_recovery_v1"));
    store.save_benchmark(&snapshot).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|index| {
            let store = Arc::clone(&store);
            let snapshot = Arc::clone(&snapshot);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let id = Uuid::new_v4();
                barrier.wait();
                if index % 2 == 0 {
                    store.create_run(&retrieval(&snapshot, id, 100)).unwrap();
                } else {
                    store
                        .create_answer_run(&generation(&snapshot, id, 100))
                        .unwrap();
                }
                id.to_string()
            })
        })
        .collect();
    let ids: BTreeSet<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let references = registry(root.path());
    let runs = references["runs"].as_object().unwrap();
    assert_eq!(references["next_run_number"], 9);
    assert_eq!(runs.len(), 8);
    assert_eq!(
        runs.values()
            .map(|run| run["run_id"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>(),
        ids
    );
    for number in 1..=8 {
        let run = &runs[&number.to_string()];
        assert_eq!(run["benchmark"], "ragtruth-qa");
        match run["mode"].as_str().unwrap() {
            "retrieval" => assert_eq!(run["description"], "Retrieval baseline"),
            "generation" => assert_eq!(run["description"], "Generation baseline"),
            mode => panic!("Unexpected mode {mode}"),
        }
    }
    let (headers, rows) = table(root.path(), "ragtruth-qa");
    assert_eq!(&headers[..SHARED_COLUMNS.len()], SHARED_COLUMNS);
    assert_eq!(rows.len(), 8);
    assert_eq!(
        rows.iter()
            .map(|row| row["run_number"].parse::<u64>().unwrap())
            .collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
    assert_eq!(
        rows.iter()
            .map(|row| row["run_id"].clone())
            .collect::<BTreeSet<_>>(),
        ids
    );
    let before = references;
    drop(store);
    let reopened = Store::open(root.path()).unwrap();
    assert_eq!(registry(root.path()), before);
    assert_eq!(reopened.runs().unwrap().len(), 4);
    assert_eq!(reopened.answer_runs().unwrap().len(), 4);
}

#[test]
fn saves_replace_rows_and_preserve_missing_metrics_as_blank() {
    let root = TempDir::new().unwrap();
    let store = Store::open(root.path()).unwrap();
    let snapshot = benchmark("paired_context_recovery_v1");
    store.save_benchmark(&snapshot).unwrap();
    let mut first = retrieval(&snapshot, Uuid::new_v4(), 100);
    first.status = RunStatus::Running;
    first.completed = 0;
    first.finished_at_ms = None;
    store.create_run(&first).unwrap();
    let mut second = retrieval(&snapshot, Uuid::new_v4(), 200);
    second.means = Some(BTreeMap::from([("recall".into(), 0.0)]));
    store.create_run(&second).unwrap();
    first.status = RunStatus::Completed;
    first.completed = 1;
    first.finished_at_ms = Some(300);
    first.means = Some(BTreeMap::from([("mrr".into(), 0.5)]));
    store.save_run(&first).unwrap();
    store.save_run(&first).unwrap();

    let (headers, rows) = table(root.path(), "ragtruth-qa");
    assert_eq!(
        &headers[SHARED_COLUMNS.len()..],
        ["metric.mrr", "metric.recall"]
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["run_id"], first.id.to_string());
    assert_eq!(rows[0]["run_number"], "1");
    assert_eq!(rows[0]["architecture_name"], first.request.label);
    assert_eq!(rows[0]["status"], "completed");
    assert_eq!(rows[0]["completed"], "1");
    assert_eq!(rows[0]["metric.mrr"], "0.5");
    assert_eq!(rows[0]["metric.recall"], "");
    assert_eq!(rows[1]["metric.mrr"], "");
    assert_eq!(rows[1]["metric.recall"].parse::<f64>().unwrap(), 0.0);
    assert_eq!(registry(root.path())["next_run_number"], 3);
}

#[test]
fn benchmark_tables_separate_automatic_metrics_from_reviews_and_refresh_on_review() {
    let root = TempDir::new().unwrap();
    let store = Store::open(root.path()).unwrap();
    let ragtruth = benchmark("paired_context_recovery_v1");
    let hotpotqa = benchmark("hotpotqa_answer_v1");
    store.save_benchmark(&ragtruth).unwrap();
    store.save_benchmark(&hotpotqa).unwrap();
    let manual = generation(&ragtruth, Uuid::new_v4(), 100);
    store.create_answer_run(&manual).unwrap();
    let mut automatic = generation(&hotpotqa, Uuid::new_v4(), 200);
    automatic.summary.scored = 1;
    automatic.summary.automatic_scores = Some(BTreeMap::from([
        ("exact_match".into(), 0.0),
        ("f1".into(), 0.5),
    ]));
    store.create_answer_run(&automatic).unwrap();
    for correctness in [false, true] {
        let review = serde_json::from_value(json!({
            "reviewer": "Reviewer", "correctness": correctness, "groundedness": true,
            "hallucination": false, "citation_accuracy": true, "notes": "Checked the evidence"
        }))
        .unwrap();
        assert!(
            store
                .review_answer(manual.summary.id, "case-1", review)
                .is_ok()
        );
    }

    let (manual_headers, manual_rows) = table(root.path(), "ragtruth-qa");
    assert_eq!(manual_rows.len(), 1);
    assert_eq!(manual_rows[0]["run_number"], "1");
    assert_eq!(manual_rows[0]["reviewed"], "1");
    assert_eq!(manual_rows[0]["scored"], "0");
    assert_eq!(
        manual_rows[0]["review.correctness"].parse::<f64>().unwrap(),
        1.0
    );
    assert_eq!(
        manual_rows[0]["review.hallucination_rate"]
            .parse::<f64>()
            .unwrap(),
        0.0
    );
    assert!(
        !manual_headers
            .iter()
            .any(|name| name.starts_with("metric."))
    );
    let (automatic_headers, automatic_rows) = table(root.path(), "hotpotqa");
    assert_eq!(automatic_rows.len(), 1);
    assert_eq!(automatic_rows[0]["run_number"], "2");
    assert_eq!(automatic_rows[0]["scored"], "1");
    assert_eq!(
        automatic_rows[0]["metric.exact_match"]
            .parse::<f64>()
            .unwrap(),
        0.0
    );
    assert_eq!(automatic_rows[0]["metric.f1"], "0.5");
    assert!(
        !automatic_headers
            .iter()
            .any(|name| name.starts_with("review."))
    );
    assert_eq!(registry(root.path())["next_run_number"], 3);
}

#[test]
fn legacy_runs_are_numbered_by_start_and_id_and_recovery_updates_the_table() {
    let root = TempDir::new().unwrap();
    let snapshot = benchmark("paired_context_recovery_v1");
    let first = retrieval(&snapshot, Uuid::from_u128(1), 100);
    let second = generation(&snapshot, Uuid::from_u128(2), 100);
    let mut third = retrieval(&snapshot, Uuid::from_u128(3), 200);
    third.status = RunStatus::Running;
    third.finished_at_ms = None;
    write_legacy(root.path(), "runs", third.id, &third);
    write_legacy(root.path(), "answer-runs", second.summary.id, &second);
    write_legacy(root.path(), "runs", first.id, &first);
    // Historical standalone exports can lack snapshots; these evaluator identities are unique.
    let store = Store::open(root.path()).unwrap();
    let references = registry(root.path());
    assert_eq!(references["runs"]["1"]["run_id"], first.id.to_string());
    assert_eq!(
        references["runs"]["2"]["run_id"],
        second.summary.id.to_string()
    );
    assert_eq!(references["runs"]["3"]["run_id"], third.id.to_string());
    assert_eq!(references["next_run_number"], 4);
    assert_eq!(store.run(third.id).unwrap().status, RunStatus::Interrupted);
    let (_, rows) = table(root.path(), "ragtruth-qa");
    assert_eq!(rows[2]["status"], "interrupted");
    assert!(!rows[2]["finished_at_ms"].is_empty());
    drop(store);

    let mut edited = references;
    edited["runs"]["2"]["description"] =
        json!("Team description edited while the server was stopped");
    fs::write(
        root.path().join("results/runs.json"),
        serde_json::to_vec_pretty(&edited).unwrap(),
    )
    .unwrap();
    // CSV is a derived table and can be rebuilt from the detailed saved runs.
    fs::remove_file(root.path().join("results/ragtruth-qa.csv")).unwrap();
    let reopened = Store::open(root.path()).unwrap();
    reopened.save_answer_run(&second).unwrap();
    let fourth = retrieval(&snapshot, Uuid::from_u128(4), 300);
    reopened.create_run(&fourth).unwrap();
    let references = registry(root.path());
    assert_eq!(
        references["runs"]["2"]["description"],
        edited["runs"]["2"]["description"]
    );
    assert_eq!(references["runs"]["4"]["run_id"], fourth.id.to_string());
    assert_eq!(references["next_run_number"], 5);
    assert_eq!(table(root.path(), "ragtruth-qa").1.len(), 4);
}

#[test]
fn malformed_or_duplicate_run_references_are_rejected_without_overwriting_them() {
    for duplicate in [false, true] {
        let root = TempDir::new().unwrap();
        let store = Store::open(root.path()).unwrap();
        let snapshot = benchmark("paired_context_recovery_v1");
        store
            .create_run(&retrieval(&snapshot, Uuid::new_v4(), 100))
            .unwrap();
        drop(store);
        let path = root.path().join("results/runs.json");
        let invalid = if duplicate {
            let mut references = registry(root.path());
            references["runs"]["2"] = references["runs"]["1"].clone();
            references["next_run_number"] = json!(3);
            serde_json::to_vec_pretty(&references).unwrap()
        } else {
            b"{invalid json".to_vec()
        };
        fs::write(&path, &invalid).unwrap();
        assert!(Store::open(root.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), invalid);
    }
}

#[test]
fn duplicate_numeric_run_keys_are_rejected_before_descriptions_or_numbers_are_lost() {
    for duplicate_key in ["1", "01"] {
        let root = TempDir::new().unwrap();
        let store = Store::open(root.path()).unwrap();
        let snapshot = benchmark("paired_context_recovery_v1");
        for started_at_ms in [100, 200] {
            store
                .create_run(&retrieval(&snapshot, Uuid::new_v4(), started_at_ms))
                .unwrap();
        }
        drop(store);
        let mut references = registry(root.path());
        references["runs"]["1"]["description"] = json!("Team notes only recorded in the registry");
        // Construct raw JSON: serde_json::Value would discard duplicate object keys itself.
        let invalid = format!(
            "{{\"next_run_number\":3,\"runs\":{{\"1\":{},\"{duplicate_key}\":{}}}}}",
            references["runs"]["1"], references["runs"]["2"]
        );
        let path = root.path().join("results/runs.json");
        fs::write(&path, &invalid).unwrap();
        assert!(Store::open(root.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), invalid.as_bytes());
    }
}
