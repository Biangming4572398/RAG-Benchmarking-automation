use crate::{Error, Result};
use std::{env, net::SocketAddr, path::PathBuf};

pub struct Config {
    pub address: SocketAddr,
    pub data_dir: PathBuf,
    pub catalog_path: PathBuf,
    pub api_token: String,
    pub nebula: Option<NebulaConfig>,
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
        let api_token = env::var("BENCHMARK_API_TOKEN")
            .map_err(|_| Error("BENCHMARK_API_TOKEN is required".into()))?;
        validate_token(&api_token)?;
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
            api_token,
            catalog_path: env::var_os("BENCHMARK_CATALOG")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("benchmarks.yaml")),
            nebula,
        })
    }
}

pub fn validate_token(token: &str) -> Result<()> {
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
