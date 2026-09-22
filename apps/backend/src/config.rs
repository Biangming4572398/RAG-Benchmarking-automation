use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

pub struct Config {
    pub address: SocketAddr,
    pub data_dir: PathBuf,
    pub catalog_path: PathBuf,
    pub nebula: Option<NebulaConfig>,
    /// Explicitly granted corpus for automatic suite publication and indexing.
    pub nebula_corpus: Option<PathBuf>,
}

// Deliberately no Debug/Serialize: credentials never become dashboard data.
#[derive(Clone)]
pub struct NebulaConfig {
    pub base_url: String,
    pub token: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let address: SocketAddr = env::var("BENCHMARK_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:4319".into())
            .parse()
            .map_err(|_| Error("BENCHMARK_ADDR must be a loopback socket address".into()))?;
        if !address.ip().is_loopback() {
            return Err(Error("BENCHMARK_ADDR must use loopback".into()));
        }
        let nebula = match (
            env::var("NEBULA_API_BASE").ok(),
            env::var("NEBULA_API_TOKEN").ok(),
        ) {
            (None, None) => None,
            (Some(base_url), Some(token)) => {
                let config = NebulaConfig { base_url, token };
                config.validate()?;
                Some(config)
            }
            _ => {
                return Err(Error(
                    "Set both NEBULA_API_BASE and NEBULA_API_TOKEN, or neither".into(),
                ));
            }
        };
        Ok(Self {
            address,
            data_dir: env::var_os("BENCHMARK_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("benchmark-data")),
            catalog_path: env::var_os("BENCHMARK_CATALOG")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("benchmarks.yaml")),
            nebula,
            nebula_corpus: env::var_os("BENCHMARK_NEBULA_CORPUS")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        })
    }
}

fn validate_token(token: &str) -> Result<()> {
    if token.is_empty() || token.len() > 1024 || !token.bytes().all(|b| (33..=126).contains(&b)) {
        return Err(Error(
            "API tokens must contain 1–1024 visible ASCII characters without spaces".into(),
        ));
    }
    Ok(())
}

impl NebulaConfig {
    pub fn validate(&self) -> Result<()> {
        validate_token(&self.token)?;
        let url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| Error("Invalid NEBULA_API_BASE".into()))?;
        let loopback = url
            .host_str()
            .and_then(|host| host.parse::<std::net::Ipv4Addr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if !loopback
            || url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path().trim_end_matches('/') != "/api/nebula/v1"
        {
            return Err(Error(
                "NEBULA_API_BASE must be http://127.x.x.x:PORT/api/nebula/v1".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub benchmarks: BTreeMap<String, BenchmarkDefinition>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    pub split: String,
    pub adapter: Adapter,
    pub evaluation: Evaluation,
    pub defaults: Defaults,
}

pub type Adapter = String;

#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Evaluation(pub String);

impl Evaluation {
    pub fn metric_kind(&self) -> &str {
        &self.0
    }
}
impl From<&str> for Evaluation {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
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
                .validate_for(key)
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
        definition.validate_for(&request.benchmark)?;
        definition.source = definition.source.replace("{split}", &definition.split);
        Ok(ResolvedBenchmark {
            key: request.benchmark.clone(),
            definition,
        })
    }
}

impl BenchmarkDefinition {
    /// Validate an adapter with one registered implementation, without a catalog key.
    pub fn validate(&self) -> Result<()> {
        self.validate_fields()?;
        crate::module_for_definition(None, self)?.validate_definition(self)
    }

    /// Keep benchmark-specific validation attached to the selected catalog module.
    pub fn validate_for(&self, key: &str) -> Result<()> {
        self.validate_fields()?;
        crate::module_for_definition(Some(key), self)?.validate_definition(self)
    }

    fn validate_fields(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.source.trim().is_empty() {
            return Err(Error("name and source must not be empty".into()));
        }
        for (name, value) in [
            ("description", &self.description),
            ("preparation", &self.preparation),
        ] {
            if let Some(value) = value
                && (value.trim().is_empty() || value.len() > 2_000)
            {
                return Err(Error(format!("{name} must contain 1–2000 bytes")));
            }
        }
        if let Some(homepage) = &self.homepage {
            public_url(homepage)?;
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

fn public_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| Error("Catalog links must be public HTTP(S) URLs".into()))?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
    {
        return Err(Error(
            "Catalog links must be public HTTP(S) URLs without credentials or query parameters"
                .into(),
        ));
    }
    Ok(())
}

/// Common requirements for catalog-only modules. Their own files decide availability.
pub fn validate_external_definition(definition: &BenchmarkDefinition) -> Result<()> {
    if definition.adapter != "external_suite"
        || definition.evaluation.metric_kind() != "external_evaluation"
    {
        return Err(Error("Adapter and evaluation do not match".into()));
    }
    public_url(&definition.source)?;
    if definition.preparation.is_none() {
        return Err(Error(
            "External suites must explain their preparation requirements".into(),
        ));
    }
    if definition.source_sha256.is_some() {
        return Err(Error(
            "External suite sources identify the publisher, not a loadable dataset file".into(),
        ));
    }
    if definition.split.trim().is_empty() || definition.split.len() > 128 {
        return Err(Error(
            "External suite split must contain 1–128 bytes".into(),
        ));
    }
    Ok(())
}
