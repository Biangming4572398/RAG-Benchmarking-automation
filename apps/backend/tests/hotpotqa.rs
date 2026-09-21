use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
};

use backend::{
    Case,
    config::{Catalog, LoadRequest, ResolvedBenchmark},
    digest, initialize_benchmark,
    server::Store,
};
use serde_json::{Value, json};

fn example(id: &str) -> Value {
    json!({
        "_id": id,
        "question": "Where is Alpha?",
        "answer": "REFERENCE_ANSWER_NOT_A_PASSAGE",
        "context": (0..10).map(|n| json!([format!("Passage {n}"), [format!("Sentence {n} zero."), format!("Sentence {n} one.")]])).collect::<Vec<_>>(),
        "supporting_facts": [["Passage 0", 0], ["Passage 1", 1]],
        "type": "bridge",
        "level": "hard"
    })
}

fn settings(source: &str, limit: usize) -> ResolvedBenchmark {
    let mut catalog = Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap();
    let definition = catalog.benchmarks.get_mut("hotpotqa").unwrap();
    definition.source = source.into();
    definition.source_sha256 = None;
    catalog
        .resolve(&LoadRequest {
            benchmark: "hotpotqa".into(),
            limit: Some(limit),
        })
        .unwrap()
}

fn write_fixture(path: &Path, examples: &[Value]) -> ResolvedBenchmark {
    fs::write(path, serde_json::to_vec(examples).unwrap()).unwrap();
    settings(path.to_str().unwrap(), 100)
}

#[test]
fn preserves_per_question_distractors_deduplicates_passages_and_never_indexes_labels() {
    let dir = tempfile::tempdir().unwrap();
    let request = write_fixture(
        &dir.path().join("hotpot.json"),
        &[example("first"), example("second")],
    );
    let benchmark = initialize_benchmark(&request).unwrap();
    assert_eq!(benchmark.metric_kind, "hotpotqa_answer_v1");
    assert_eq!(benchmark.cases.len(), 2);
    assert_eq!(benchmark.documents.len(), 10);
    assert_eq!(benchmark.cases[0].id, "first");
    assert!(
        benchmark
            .cases
            .iter()
            .all(|case| case.document_id.is_empty() && case.reference_outputs.is_empty())
    );
    let reference = benchmark.cases[0].answer_reference.as_ref().unwrap();
    assert_eq!(reference.candidate_document_ids.len(), 10);
    assert_eq!(reference.supporting_facts[1].sentence_index, 1);
    let store = Store::open(&dir.path().join("data")).unwrap();
    let info = store.save_benchmark(&benchmark).unwrap();
    for document in &benchmark.documents {
        let text = fs::read_to_string(info.corpus_path.join(&document.filename)).unwrap();
        assert_eq!(text, document.text);
        assert!(!text.contains("REFERENCE_ANSWER_NOT_A_PASSAGE"));
        assert!(!text.contains("supporting_facts"));
        assert_eq!(digest(text.as_bytes()), document.revision);
    }
    let repeated = initialize_benchmark(&request).unwrap();
    assert_eq!(
        store.save_benchmark(&repeated).unwrap().fingerprint,
        info.fingerprint
    );
    let mut relabelled = repeated;
    relabelled.id = uuid::Uuid::new_v4();
    relabelled.cases[0]
        .answer_reference
        .as_mut()
        .unwrap()
        .answer = "another gold answer".into();
    assert_ne!(
        store.save_benchmark(&relabelled).unwrap().fingerprint,
        info.fingerprint
    );
    let mut limited_request = request;
    limited_request.definition.defaults.limit = 1;
    let limited = initialize_benchmark(&limited_request).unwrap();
    assert_eq!(limited.cases.len(), 1);
    assert_eq!(limited.cases[0].id, "first");
}

#[test]
fn passage_identity_includes_title_and_exact_sentence_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let original = example("first");
    let mut changed = example("second");
    changed["context"][0][1][0] = json!("A changed sentence under the same title.");
    let request = write_fixture(&dir.path().join("hotpot.json"), &[original, changed]);
    let benchmark = initialize_benchmark(&request).unwrap();
    assert_eq!(benchmark.documents.len(), 11);
    assert_ne!(
        benchmark.cases[0]
            .answer_reference
            .as_ref()
            .unwrap()
            .candidate_document_ids[0],
        benchmark.cases[1]
            .answer_reference
            .as_ref()
            .unwrap()
            .candidate_document_ids[0]
    );
}

#[test]
fn rejects_missing_labels_bad_supports_duplicate_ids_and_wrong_distractor_counts() {
    let dir = tempfile::tempdir().unwrap();
    let mut bad = Vec::new();
    let mut row = example("first");
    row.as_object_mut().unwrap().remove("answer");
    bad.push(vec![row]);
    let mut row = example("first");
    row["answer"] = json!(" ");
    bad.push(vec![row]);
    let mut row = example("first");
    row["context"].as_array_mut().unwrap().truncate(1);
    bad.push(vec![row]);
    let mut row = example("first");
    row["context"][1] = row["context"][0].clone();
    bad.push(vec![row]);
    let mut row = example("first");
    row["supporting_facts"][0][1] = json!(-1);
    bad.push(vec![row]);
    let mut row = example("first");
    row["supporting_facts"][0][0] = json!("");
    bad.push(vec![row]);
    bad.push(vec![example("repeated"), example("repeated")]);
    bad.push(vec![]);
    for rows in bad {
        let request = write_fixture(&dir.path().join("hotpot.json"), &rows);
        assert!(
            initialize_benchmark(&request).is_err(),
            "accepted invalid fixture {rows:?}"
        );
    }
}

#[test]
fn preserves_original_unresolved_support_annotations_without_using_them_as_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let mut row = example("first");
    row["supporting_facts"] = json!([["Missing title", 902], ["Passage 0", 902]]);
    let request = write_fixture(&dir.path().join("hotpot.json"), &[row]);
    let benchmark = initialize_benchmark(&request).unwrap();
    let reference = benchmark.cases[0].answer_reference.as_ref().unwrap();
    assert_eq!(reference.supporting_facts[0].title, "Missing title");
    assert_eq!(reference.supporting_facts[1].sentence_index, 902);
    assert_eq!(reference.candidate_document_ids.len(), 10);
    assert_eq!(benchmark.documents.len(), 10);
}

#[test]
fn verifies_optional_source_checksum_and_bounds_local_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hotpot.json");
    let mut request = write_fixture(&path, &[example("first")]);
    request.definition.source_sha256 = Some(digest(&fs::read(&path).unwrap()));
    assert!(initialize_benchmark(&request).is_ok());
    request.definition.source_sha256 = Some("0".repeat(64));
    assert!(
        initialize_benchmark(&request)
            .err()
            .unwrap()
            .to_string()
            .contains("SHA-256")
    );
    fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        initialize_benchmark(&request)
            .err()
            .unwrap()
            .to_string()
            .contains("64 MiB")
    );
}

#[test]
fn downloads_public_http_and_sanitizes_remote_errors() {
    for success in [true, false] {
        let body = if success {
            serde_json::to_vec(&vec![example("first")]).unwrap()
        } else {
            b"sensitive-server-error".to_vec()
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let bytes_read = stream.read(&mut request).unwrap();
            assert!(bytes_read > 0, "expected an HTTP request");
            write!(
                stream,
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                if success { "200 OK" } else { "403 Forbidden" },
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let result = initialize_benchmark(&settings(
            &format!("http://{address}/private?signature=not-for-errors"),
            1,
        ));
        if success {
            assert_eq!(result.unwrap().cases.len(), 1);
        } else {
            let message = result.err().unwrap().to_string();
            assert!(message.contains("Cannot download"));
            assert!(!message.contains("signature"));
            assert!(!message.contains("sensitive"));
        }
        task.join().unwrap();
    }
}

#[test]
fn legacy_cases_round_trip_without_new_fields_or_fingerprint_changes() {
    let json =
        json!({"id":"old", "query":"Question?", "document_id":"document", "reference_outputs":[]});
    let case: Case = serde_json::from_value(json.clone()).unwrap();
    assert!(case.answer_reference.is_none());
    assert_eq!(serde_json::to_value(case).unwrap(), json);
}

#[test]
#[ignore = "downloads public HotpotQA development JSON; no model calls"]
fn live_hotpotqa_development_snapshot() {
    let catalog = Catalog::load(Path::new("benchmarks.yaml")).unwrap();
    let mut request = catalog
        .resolve(&LoadRequest {
            benchmark: "hotpotqa".into(),
            limit: None,
        })
        .unwrap();
    if let Ok(source) = std::env::var("HOTPOTQA_SOURCE") {
        request.definition.source = source;
    }
    let benchmark = initialize_benchmark(&request).unwrap();
    assert_eq!(benchmark.cases.len(), 100);
    assert!(benchmark.documents.len() >= 10);
    assert!(benchmark.cases.iter().all(|case| {
        (2..=10).contains(
            &case
                .answer_reference
                .as_ref()
                .unwrap()
                .candidate_document_ids
                .len(),
        )
    }));
    println!(
        "Prepared {} HotpotQA cases and {} unique candidate passages",
        benchmark.cases.len(),
        benchmark.documents.len()
    );
}
