//! Registration and snapshot routing remain independent of dataset transport.
use backend::{
    BENCHMARKS, Benchmark, RunRequest,
    config::{Catalog, LoadRequest},
    initialize_benchmark, module_for_definition, module_for_snapshot,
};
use uuid::Uuid;

fn catalog() -> Catalog {
    Catalog::from_yaml(include_str!("../benchmarks.yaml")).unwrap()
}

fn snapshot(metric: &str) -> Benchmark {
    Benchmark {
        id: Uuid::new_v4(),
        source: "fixture".into(),
        split: "test".into(),
        metric_kind: metric.into(),
        configuration: None,
        cases: vec![],
        documents: vec![],
    }
}

#[test]
fn every_catalog_entry_resolves_to_its_named_module() {
    let catalog = catalog();
    assert_eq!(BENCHMARKS.len(), catalog.benchmarks.len());
    for (key, definition) in &catalog.benchmarks {
        definition.validate_for(key).unwrap();
        let module = module_for_definition(Some(key), definition).unwrap();
        assert_eq!(module.key(), key);
        assert_eq!(module.adapter(), definition.adapter);
        assert_eq!(module.metric_kind(), definition.evaluation.metric_kind());
    }
    let ragtruth =
        module_for_definition(Some("ragtruth-qa"), &catalog.benchmarks["ragtruth-qa"]).unwrap();
    assert_eq!(
        ragtruth.answer_evaluation().unwrap().id(),
        "manual_review_v1"
    );
    let hotpot = module_for_definition(Some("hotpotqa"), &catalog.benchmarks["hotpotqa"]).unwrap();
    assert_eq!(
        hotpot.answer_evaluation().unwrap().id(),
        "hotpotqa_answer_v1"
    );
}

#[test]
fn shared_adapters_require_a_registered_catalog_key_for_validation() {
    let catalog = catalog();
    for (key, definition) in &catalog.benchmarks {
        if definition.adapter != "external_suite" {
            definition.validate().unwrap();
            continue;
        }
        let error = definition.validate().unwrap_err();
        assert!(error.to_string().contains("specify its catalog key"));
        assert!(module_for_definition(None, definition).is_err());
        assert!(definition.validate_for("unregistered-suite").is_err());
        definition.validate_for(key).unwrap();

        let mut missing_preparation = definition.clone();
        missing_preparation.preparation = None;
        assert!(
            missing_preparation
                .validate_for(key)
                .unwrap_err()
                .to_string()
                .contains("preparation requirements")
        );
        let mut invalid_defaults = definition.clone();
        invalid_defaults.defaults.limit = 0;
        assert!(
            invalid_defaults
                .validate_for(key)
                .unwrap_err()
                .to_string()
                .contains("limit must be between")
        );
    }
}

#[test]
fn legacy_snapshots_dispatch_by_unique_metric_without_catalog_configuration() {
    for (metric, expected_key) in [
        ("paired_context_recovery_v1", "ragtruth-qa"),
        ("hotpotqa_answer_v1", "hotpotqa"),
    ] {
        let legacy = snapshot(metric);
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert!(encoded.get("configuration").is_none());
        let decoded: Benchmark = serde_json::from_value(encoded).unwrap();
        assert_eq!(module_for_snapshot(&decoded).unwrap().key(), expected_key);
    }
    assert!(module_for_snapshot(&snapshot("unknown_evaluation")).is_err());
    // Several catalog-only modules share this marker; guessing one is unsafe.
    assert!(module_for_snapshot(&snapshot("external_evaluation")).is_err());
}

#[test]
fn configured_snapshots_reject_mismatches_without_falling_back_to_the_metric() {
    let mut benchmark = snapshot("paired_context_recovery_v1");
    benchmark.configuration = Some(
        catalog()
            .resolve(&LoadRequest {
                benchmark: "hotpotqa".into(),
                limit: None,
            })
            .unwrap(),
    );
    assert!(module_for_snapshot(&benchmark).is_err());
    benchmark.metric_kind = "hotpotqa_answer_v1".into();
    assert_eq!(module_for_snapshot(&benchmark).unwrap().key(), "hotpotqa");
    benchmark.configuration.as_mut().unwrap().definition.adapter = "unknown_adapter".into();
    assert!(module_for_snapshot(&benchmark).is_err());
    benchmark.configuration.as_mut().unwrap().definition.adapter = "hotpotqa_distractor".into();
    benchmark
        .configuration
        .as_mut()
        .unwrap()
        .definition
        .evaluation = "made_up_evaluation".into();
    assert!(module_for_snapshot(&benchmark).is_err());
}

#[test]
fn supported_adapters_allow_team_catalog_names_without_changing_the_module() {
    let mut request = catalog()
        .resolve(&LoadRequest {
            benchmark: "ragtruth-qa".into(),
            limit: None,
        })
        .unwrap();
    request.key = "our-ragtruth-test-set".into();
    let mut benchmark = snapshot("paired_context_recovery_v1");
    benchmark.configuration = Some(request);
    assert_eq!(
        module_for_snapshot(&benchmark).unwrap().key(),
        "ragtruth-qa"
    );
}

#[test]
fn each_external_module_rejects_initialization_and_retrieval_explicitly() {
    let catalog = catalog();
    for key in [
        "longmemeval-cleaned",
        "temprageval",
        "qasper",
        "abstentionbench",
        "multihop-rag",
        "ragbench",
    ] {
        let request = catalog
            .resolve(&LoadRequest {
                benchmark: key.into(),
                limit: None,
            })
            .unwrap();
        let module = module_for_definition(Some(key), &request.definition).unwrap();
        assert_eq!(module.key(), key);
        assert!(module.answer_evaluation().is_none());
        let error = initialize_benchmark(&request)
            .err()
            .expect("external suite cannot create a snapshot");
        assert!(error.to_string().contains(&request.definition.name));
        assert!(error.to_string().contains("not integrated"));
        assert!(
            error
                .to_string()
                .contains(request.definition.preparation.as_deref().unwrap())
        );
        let mut benchmark = snapshot("external_evaluation");
        benchmark.configuration = Some(request);
        assert!(
            module
                .prepare_retrieval(
                    RunRequest {
                        description: String::new(),
                        benchmark_id: benchmark.id,
                        top_k: 8,
                        label: String::new()
                    },
                    &benchmark,
                    "fixture".into()
                )
                .is_err()
        );
    }
}
