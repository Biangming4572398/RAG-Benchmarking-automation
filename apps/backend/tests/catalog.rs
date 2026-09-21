use backend::config::{Catalog, LoadRequest};

const YAML: &str = include_str!("../benchmarks.yaml");

#[test]
fn catalog_resolves_named_benchmarks_and_applies_only_explicit_overrides() {
    let catalog = Catalog::from_yaml(YAML).unwrap();
    let mut request = LoadRequest {
        benchmark: "ragtruth-qa".into(),
        limit: None,
    };
    let settings = catalog.resolve(&request).unwrap();
    assert_eq!(settings.definition.defaults.limit, 100);
    assert_eq!(settings.definition.defaults.top_k, 8);
    assert_eq!(
        settings.definition.source,
        "hf://datasets/wandb/RAGTruth-processed/data/test-*.parquet"
    );
    request.limit = Some(12);
    assert_eq!(
        catalog.resolve(&request).unwrap().definition.defaults.limit,
        12
    );
    request.limit = Some(0);
    assert!(catalog.resolve(&request).is_err());
    request.benchmark = "unknown".into();
    assert!(catalog.resolve(&request).is_err());
    assert_eq!(catalog.benchmarks["ragtruth-qa"].defaults.limit, 100);
}

#[test]
fn invalid_catalogs_fail_before_loading_any_dataset() {
    for yaml in [
        "benchmarks: {}".to_string(),
        YAML.replace("ragtruth_qa", "not_implemented"),
        YAML.replace("paired_context_recovery_v1", "made_up_score"),
        YAML.replace("limit: 100", "limit: 0"),
        YAML.replace("top_k: 8", "top_k: 101"),
        YAML.replace("split: test", "split: other"),
        YAML.replace("split: dev", "split: train"),
        YAML.replace(
            "evaluation: hotpotqa_answer_v1",
            "evaluation: paired_context_recovery_v1",
        ),
        YAML.replace("adapter: ragtruth_qa", "adapter: hotpotqa_distractor"),
        YAML.replace("name: RAGTruth QA", "name: ''"),
        YAML.replace("top_k: 8", "top_k: 8\n      typo: 1"),
        format!("{YAML}\nbenchmarks: {{}}"),
        format!("{YAML}{}", YAML.strip_prefix("benchmarks:\n").unwrap()),
    ] {
        assert!(
            Catalog::from_yaml(&yaml).is_err(),
            "accepted invalid YAML: {yaml}"
        );
    }
}

#[test]
fn hotpotqa_catalog_uses_pinned_distractor_dev_and_validates_sources() {
    let catalog = Catalog::from_yaml(YAML).unwrap();
    let request = LoadRequest {
        benchmark: "hotpotqa".into(),
        limit: None,
    };
    let settings = catalog.resolve(&request).unwrap();
    assert_eq!(settings.definition.split, "dev");
    assert_eq!(settings.definition.defaults.limit, 100);
    assert_eq!(
        settings.definition.evaluation.metric_kind(),
        "hotpotqa_answer_v1"
    );
    assert_eq!(
        settings.definition.source_sha256.as_deref(),
        Some("4e9ecb5c8d3b719f624d66b60f8d56bf227f03914f5f0753d6fa1b359d7104ea")
    );
    for source in [
        "hf://bad/path",
        "https://name:secret@example.org/data",
        "https://example.org/data#fragment",
    ] {
        let mut invalid = settings.clone();
        invalid.definition.source = source.into();
        assert!(invalid.definition.validate().is_err());
    }
    let mut invalid = settings;
    invalid.definition.source_sha256 = Some("wrong".into());
    assert!(invalid.definition.validate().is_err());
}

#[test]
fn local_sources_are_relative_to_the_catalog_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.yaml");
    std::fs::write(
        &path,
        YAML.replace(
            "hf://datasets/wandb/RAGTruth-processed/data/{split}-*.parquet",
            "fixtures/{split}.parquet",
        ),
    )
    .unwrap();
    let settings = Catalog::load(&path)
        .unwrap()
        .resolve(&LoadRequest {
            benchmark: "ragtruth-qa".into(),
            limit: None,
        })
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(settings.definition.source),
        dir.path().join("fixtures/test.parquet")
    );
}

#[test]
fn external_suites_are_visible_once_but_cannot_be_loaded_as_supported_benchmarks() {
    let catalog = Catalog::from_yaml(YAML).unwrap();
    assert_eq!(catalog.benchmarks.len(), 8);
    for key in [
        "longmemeval-cleaned",
        "temprageval",
        "qasper",
        "abstentionbench",
        "multihop-rag",
        "ragbench",
    ] {
        let resolved = catalog
            .resolve(&LoadRequest {
                benchmark: key.into(),
                limit: None,
            })
            .unwrap();
        assert!(resolved.definition.homepage.is_some());
        assert!(resolved.definition.description.is_some());
        assert!(resolved.definition.preparation.is_some());
        assert_eq!(
            resolved.definition.evaluation.metric_kind(),
            "external_evaluation"
        );
        let error = match backend::initialize_benchmark(&resolved) {
            Ok(_) => panic!("External suite became a runnable snapshot"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("not integrated"));
        assert!(
            error
                .to_string()
                .contains(resolved.definition.preparation.as_deref().unwrap())
        );
    }
}

#[test]
fn external_catalog_metadata_and_evaluation_pairs_are_validated() {
    let catalog = Catalog::from_yaml(YAML).unwrap();
    let original = &catalog.benchmarks["qasper"];
    let mut definition = original.clone();
    definition.preparation = None;
    assert!(definition.validate_for("qasper").is_err());
    definition = original.clone();
    definition.evaluation = "hotpotqa_answer_v1".into();
    assert!(definition.validate_for("qasper").is_err());
    for url in [
        "javascript:alert(1)",
        "https://name:secret@example.org",
        "https://example.org?token=secret",
    ] {
        definition = original.clone();
        definition.homepage = Some(url.into());
        assert!(definition.validate_for("qasper").is_err());
        definition = original.clone();
        definition.source = url.into();
        assert!(definition.validate_for("qasper").is_err());
    }
    for description in ["".into(), "x".repeat(2001)] {
        definition = original.clone();
        definition.description = Some(description);
        assert!(definition.validate_for("qasper").is_err());
    }
}
