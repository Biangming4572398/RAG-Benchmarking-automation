use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    time::Duration,
};

use serde::Deserialize;
use uuid::Uuid;

use crate::{
    Error, Result,
    catalog::ResolvedBenchmark,
    load_benchmarks::{AnswerReference, Benchmark, Case, Document, SupportingFact, digest},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct Example {
    #[serde(rename = "_id")]
    id: String,
    question: String,
    answer: String,
    context: Vec<(String, Vec<String>)>,
    supporting_facts: Vec<(String, usize)>,
}

/// Preserve the dataset's order and per-question distractors. Only the passages
/// enter the corpus; reference answers/support annotations remain private labels.
pub(crate) fn load(request: &ResolvedBenchmark) -> Result<Benchmark> {
    let bytes = read_source(&request.definition.source)?;
    if request
        .definition
        .source_sha256
        .as_ref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&digest(&bytes)))
    {
        return Err(Error(
            "HotpotQA source SHA-256 does not match the catalog".into(),
        ));
    }
    let examples: Vec<Example> = serde_json::from_slice(&bytes).map_err(|_| {
        Error("Invalid HotpotQA JSON: expected development distractor examples".into())
    })?;
    if examples.is_empty() || examples.len() > 10_000 {
        return Err(Error("HotpotQA must contain 1–10000 examples".into()));
    }
    let mut example_ids = BTreeSet::new();
    for example in &examples {
        if example.id.is_empty()
            || example.id.len() > 128
            || !example
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            || !example_ids.insert(&example.id)
        {
            return Err(Error(
                "HotpotQA example IDs must be unique nonempty identifiers".into(),
            ));
        }
    }
    let mut cases = Vec::new();
    let mut documents = BTreeMap::new();
    for (row, example) in examples
        .into_iter()
        .take(request.definition.defaults.limit)
        .enumerate()
    {
        if example.question.trim().is_empty()
            || example.question.len() > 16 * 1024
            || example.answer.trim().is_empty()
            || example.answer.len() > 16 * 1024
            // The published dev file has a few examples with fewer distractors.
            || !(2..=10).contains(&example.context.len())
            || example.supporting_facts.is_empty()
            || example.supporting_facts.len() > 1000
        {
            return Err(Error(format!(
                "Invalid question, answer, or distractor context at HotpotQA row {row}"
            )));
        }
        let mut titles = BTreeSet::new();
        let mut candidate_document_ids = Vec::new();
        for (title, sentences) in &example.context {
            if title.trim().is_empty()
                || title.len() > 8 * 1024
                || sentences.is_empty()
                || sentences.len() > 1000
                || sentences.iter().any(|sentence| sentence.len() > 64 * 1024)
                || sentences.iter().all(|sentence| sentence.trim().is_empty())
                || !titles.insert(title.as_str())
            {
                return Err(Error(format!(
                    "Invalid or duplicate passage at HotpotQA row {row}"
                )));
            }
            // Include sentence boundaries in identity so distinct versions of the
            // same Wikipedia title cannot silently reuse another case's passage.
            let id = digest(&serde_json::to_vec(&(title, sentences))?);
            candidate_document_ids.push(id.clone());
            documents.entry(id.clone()).or_insert_with(|| {
                let text = format!("# {title}\n\n{}\n", sentences.join("\n"));
                Document {
                    id: id.clone(),
                    filename: format!("hotpotqa-{id}.md"),
                    revision: digest(text.as_bytes()),
                    text,
                }
            });
        }
        // These original annotations are retained, not scored or used to select
        // retrieval sources. The official file includes unresolved sentence IDs.
        for (title, _) in &example.supporting_facts {
            if title.trim().is_empty() || title.len() > 8 * 1024 {
                return Err(Error(format!(
                    "Invalid supporting fact title at HotpotQA row {row}"
                )));
            }
        }
        cases.push(Case {
            id: example.id,
            query: example.question.trim().into(),
            document_id: String::new(),
            reference_outputs: Vec::new(),
            answer_reference: Some(AnswerReference {
                answer: example.answer,
                candidate_document_ids,
                supporting_facts: example
                    .supporting_facts
                    .into_iter()
                    .map(|(title, sentence_index)| SupportingFact {
                        title,
                        sentence_index,
                    })
                    .collect(),
            }),
        });
    }
    Ok(Benchmark {
        id: Uuid::new_v4(),
        source: request.definition.source.clone(),
        split: request.definition.split.clone(),
        metric_kind: request.definition.evaluation.metric_kind().into(),
        configuration: Some(request.clone()),
        cases,
        documents: documents.into_values().collect(),
    })
}

fn read_source(source: &str) -> Result<Vec<u8>> {
    let bytes = if source.contains("://") {
        // Loading runs on the server's blocking worker, alongside the synchronous
        // Parquet loader. Keep this private runtime outside the request executor.
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Error("Cannot initialize HotpotQA download".into()))?
            .block_on(download(source))?
    } else {
        let mut bytes = Vec::new();
        File::open(source)
            .and_then(|file| file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes))
            .map_err(|_| {
                Error("Cannot read HotpotQA JSON: check the catalog source path".into())
            })?;
        bytes
    };
    if bytes.len() > MAX_BYTES {
        return Err(Error(
            "HotpotQA JSON exceeds the 64 MiB safety limit".into(),
        ));
    }
    Ok(bytes)
}

async fn download(source: &str) -> Result<Vec<u8>> {
    let failure = || {
        Error("Cannot download HotpotQA JSON: check the public source URL and connectivity".into())
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| failure())?;
    let mut response = client
        .get(source)
        .send()
        .await
        .map_err(|_| failure())?
        .error_for_status()
        .map_err(|_| failure())?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BYTES as u64)
    {
        return Err(Error(
            "HotpotQA JSON exceeds the 64 MiB safety limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
        if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
            return Err(Error(
                "HotpotQA JSON exceeds the 64 MiB safety limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
