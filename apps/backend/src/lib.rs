pub mod analysis;
pub mod answers;
pub mod catalog;
pub mod config;
pub mod load_benchmarks;
pub mod server;
pub mod storage;
pub mod trials;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<csv::Error> for Error {
    fn from(error: csv::Error) -> Self {
        Self(error.to_string())
    }
}
