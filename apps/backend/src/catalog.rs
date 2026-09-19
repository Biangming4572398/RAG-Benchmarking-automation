use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub benchmarks: BTreeMap<String, BenchmarkDefinition>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkDefinition {
    pub name: String,
    pub source: String,
    pub split: String,
    pub adapter: Adapter,
    pub evaluation: Evaluation,
    pub defaults: Defaults,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    RagtruthQa,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evaluation {
    PairedContextRecoveryV1,
}

impl Evaluation {
    pub fn metric_kind(self) -> &'static str {
        match self {
            Self::PairedContextRecoveryV1 => "paired_context_recovery_v1",
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub limit: usize,
    pub top_k: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRequest {
    pub benchmark: String,
    pub limit: Option<usize>,
}

/// Stored with the dataset, so subsequent catalog edits never affect a snapshot.
#[derive(Clone, Deserialize, Serialize)]
pub struct ResolvedBenchmark {
    pub key: String,
    pub definition: BenchmarkDefinition,
}

impl Catalog {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        // Value rejects duplicate YAML mapping keys; deserialize directly into
        // a BTreeMap and duplicate benchmark names would silently overwrite.
        let value: yaml_serde::Value = yaml_serde::from_str(yaml)
            .map_err(|error| Error(format!("Invalid benchmark catalog YAML: {error}")))?;
        let catalog: Self = yaml_serde::from_value(value)
            .map_err(|error| Error(format!("Invalid benchmark catalog: {error}")))?;
        if catalog.benchmarks.is_empty() {
            return Err(Error(
                "Benchmark catalog must contain at least one benchmark".into(),
            ));
        }
        for (key, definition) in &catalog.benchmarks {
            if key.is_empty()
                || key.len() > 64
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(Error(
                    "Benchmark keys must use 1–64 lowercase letters, digits or hyphens".into(),
                ));
            }
            definition
                .validate()
                .map_err(|error| Error(format!("Benchmark {key}: {error}")))?;
        }
        Ok(catalog)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let mut catalog = Self::from_yaml(&fs::read_to_string(&path)?)?;
        for definition in catalog.benchmarks.values_mut() {
            if !definition.source.contains("://") && Path::new(&definition.source).is_relative() {
                definition.source = path
                    .parent()
                    .expect("catalog file has parent")
                    .join(&definition.source)
                    .to_string_lossy()
                    .into_owned();
            }
        }
        Ok(catalog)
    }

    pub fn resolve(&self, request: &LoadRequest) -> Result<ResolvedBenchmark> {
        let mut definition = self
            .benchmarks
            .get(&request.benchmark)
            .ok_or_else(|| Error(format!("Unknown benchmark: {}", request.benchmark)))?
            .clone();
        if let Some(limit) = request.limit {
            definition.defaults.limit = limit;
        }
        definition.validate()?;
        definition.source = definition.source.replace("{split}", &definition.split);
        Ok(ResolvedBenchmark {
            key: request.benchmark.clone(),
            definition,
        })
    }
}

impl BenchmarkDefinition {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.source.trim().is_empty() {
            return Err(Error("name and source must not be empty".into()));
        }
        if !["train", "test"].contains(&self.split.as_str()) {
            return Err(Error("RAGTruth split must be train or test".into()));
        }
        if !(1..=10_000).contains(&self.defaults.limit) {
            return Err(Error(
                "limit must be between 1 and 10000 unique cases".into(),
            ));
        }
        if !(1..=100).contains(&self.defaults.top_k) {
            return Err(Error("top_k must be between 1 and 100".into()));
        }
        Ok(())
    }
}
