//! Publish snapshot passages to an explicitly granted corpus and wait for Nebula to index it.
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::{Benchmark, Error, Result, config::NebulaConfig};

const INDEX_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) async fn prepare(
    directory: PathBuf,
    config: NebulaConfig,
    benchmarks: Vec<Benchmark>,
) -> Result<()> {
    config.validate()?;
    let documents = tokio::task::spawn_blocking(move || publish(&directory, &benchmarks))
        .await
        .map_err(|_| Error("Benchmark corpus publication stopped unexpectedly".into()))??;
    tokio::time::timeout(INDEX_TIMEOUT, index(&config, &documents))
        .await
        .map_err(|_| Error("Nebula indexing did not finish within 30 minutes".into()))?
}

fn publish(directory: &Path, benchmarks: &[Benchmark]) -> Result<BTreeMap<String, String>> {
    // Validate the entire manifest before writing. Only exported passage text enters this
    // directory: case queries, reference answers, and snapshot JSON stay in the store.
    let mut passages = BTreeMap::new();
    for benchmark in benchmarks {
        for document in &benchmark.documents {
            let mut components = Path::new(&document.filename).components();
            if !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
                || document.filename.contains(['/', '\\'])
                || !document.filename.ends_with(".md")
            {
                return Err(Error(
                    "Benchmark passage filename must be a single Markdown filename".into(),
                ));
            }
            if let Some(existing) = passages.insert(document.filename.clone(), document)
                && (existing.text != document.text || existing.revision != document.revision)
            {
                return Err(Error(
                    "Benchmark snapshots contain conflicting passage filenames".into(),
                ));
            }
        }
    }
    fs::create_dir_all(directory)?;
    // Preserve the directory itself: Nebula holds an open directory handle for its lifetime.
    for (name, document) in &passages {
        let destination = directory.join(name);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if !metadata.file_type().is_file()
                    || fs::read(&destination)? != document.text.as_bytes()
                {
                    return Err(Error(format!(
                        "Benchmark corpus already contains different content at {name}; choose a dedicated corpus directory"
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut temporary = tempfile::Builder::new()
                    .prefix(".benchmark-")
                    .tempfile_in(directory)?;
                temporary.write_all(document.text.as_bytes())?;
                temporary.flush()?;
                temporary
                    .persist_noclobber(destination)
                    .map_err(|_| Error(format!("Cannot publish benchmark passage {name}")))?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(passages
        .into_iter()
        .map(|(name, document)| (name, document.revision.clone()))
        .collect())
}

#[derive(Deserialize)]
struct Status {
    phase: String,
    #[serde(default)]
    summary: String,
}

#[derive(Deserialize)]
struct KnowledgeStatus {
    status: Status,
}

#[derive(Deserialize)]
struct Source {
    title: String,
    revision: String,
    indexed: bool,
}

#[derive(Deserialize)]
struct Workspace {
    status: Status,
    sources: Vec<Source>,
}

async fn index(config: &NebulaConfig, documents: &BTreeMap<String, String>) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| Error("Cannot create Nebula indexing client".into()))?;
    let base = config.base_url.trim_end_matches('/');
    loop {
        let response = client
            .post(format!("{base}/knowledge/reindex"))
            .bearer_auth(&config.token)
            .send()
            .await
            .map_err(|_| {
                Error("Cannot request Nebula indexing; check its backend connection".into())
            })?;
        match response.status() {
            StatusCode::ACCEPTED => break,
            // Initial model/index startup may still be running. An explicit conflict means
            // this request did not start another job; a lost response is never retried.
            StatusCode::CONFLICT => tokio::time::sleep(POLL_INTERVAL).await,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => return Err(Error(
                "Nebula does not expose benchmark indexing. Update the benchmarking branch and restart its managed launcher (or enable -benchmark-reindex on an external backend).".into(),
            )),
            status => return Err(Error(format!("Nebula indexing request failed with HTTP {}", status.as_u16()))),
        }
    }
    loop {
        let status: KnowledgeStatus = get(&client, config, "knowledge/status").await?;
        match status.status.phase.as_str() {
            "ready" => break,
            "initializing" => tokio::time::sleep(POLL_INTERVAL).await,
            _ => {
                return Err(Error(format!(
                    "Nebula could not index the benchmark corpus: {}",
                    status.status.summary
                )));
            }
        }
    }
    let workspace: Workspace = get(&client, config, "workspace").await?;
    if workspace.status.phase != "ready"
        || documents.iter().any(|(name, revision)| {
            workspace
                .sources
                .iter()
                .filter(|source| {
                    source.indexed && &source.title == name && &source.revision == revision
                })
                .count()
                != 1
        })
    {
        return Err(Error("Nebula has not indexed the exact benchmark passages; check BENCHMARK_NEBULA_CORPUS matches its configured corpus".into()));
    }
    Ok(())
}

async fn get<T: serde::de::DeserializeOwned>(
    client: &Client,
    config: &NebulaConfig,
    path: &str,
) -> Result<T> {
    client
        .get(format!("{}/{path}", config.base_url.trim_end_matches('/')))
        .bearer_auth(&config.token)
        .send()
        .await
        .map_err(|_| Error("Cannot read Nebula indexing progress".into()))?
        .error_for_status()
        .map_err(|_| Error("Nebula indexing progress is unavailable".into()))?
        .json()
        .await
        .map_err(|_| Error("Nebula returned invalid indexing progress".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Case, Document};

    fn benchmark() -> Benchmark {
        Benchmark {
            id: uuid::Uuid::new_v4(),
            source: "fixture".into(),
            split: "test".into(),
            metric_kind: "paired_context_recovery_v1".into(),
            configuration: None,
            documents: vec![Document {
                id: "doc".into(),
                filename: "passage.md".into(),
                text: "Only passage bytes\r\n".into(),
                revision: "revision".into(),
            }],
            cases: vec![Case {
                id: "case".into(),
                query: "Secret evaluation question".into(),
                document_id: "doc".into(),
                reference_outputs: vec![],
                answer_reference: None,
            }],
        }
    }

    #[test]
    fn publishes_only_passages_and_reuses_exact_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let benchmark = benchmark();
        publish(directory.path(), std::slice::from_ref(&benchmark)).unwrap();
        let file = directory.path().join("passage.md");
        let modified = fs::metadata(&file).unwrap().modified().unwrap();
        publish(directory.path(), &[benchmark]).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"Only passage bytes\r\n");
        assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_escaping_filenames_and_preserves_conflicting_corpus_files() {
        let directory = tempfile::tempdir().unwrap();
        for filename in [
            "../passage.md",
            "/passage.md",
            "folder/passage.md",
            "folder\\passage.md",
            "benchmark.json",
        ] {
            let mut benchmark = benchmark();
            benchmark.documents[0].filename = filename.into();
            assert!(publish(directory.path(), &[benchmark]).is_err());
        }
        let file = directory.path().join("passage.md");
        fs::write(&file, b"Existing user content").unwrap();
        assert!(publish(directory.path(), &[benchmark()]).is_err());
        assert_eq!(fs::read(file).unwrap(), b"Existing user content");
    }
}
