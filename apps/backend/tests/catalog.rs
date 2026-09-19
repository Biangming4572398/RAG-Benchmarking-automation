use backend::catalog::{Catalog, LoadRequest};

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
