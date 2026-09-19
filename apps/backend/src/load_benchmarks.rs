use std::collections::BTreeMap;

use polars::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Error, Result};

pub const RAGTRUTH: &str = "hf://datasets/wandb/RAGTruth-processed/";

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoadRequest {
    pub source: String,
    pub split: String,
    /// Number of unique QA cases, not number of recorded model responses.
    pub limit: usize,
}

impl Default for LoadRequest {
    fn default() -> Self {
        Self {
            source: RAGTRUTH.into(),
            split: "test".into(),
            limit: 100,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ReferenceOutput {
    pub id: String,
    pub output: String,
    pub model: String,
    pub quality: String,
    pub hallucination_labels: serde_json::Value,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub query: String,
    pub document_id: String,
    // Annotations describe these historical outputs, never a new Nebula response.
    pub reference_outputs: Vec<ReferenceOutput>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Document {
    pub id: String,
    pub filename: String,
    pub text: String,
    pub revision: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Benchmark {
    pub id: Uuid,
    pub source: String,
    pub split: String,
    pub metric_kind: String,
    pub cases: Vec<Case>,
    pub documents: Vec<Document>,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Snapshot the QA subset and deduplicate the model repetitions in RAGTruth.
/// Only source contexts are exported; outputs/annotations cannot enter retrieval.
pub fn load_benchmarks(request: &LoadRequest) -> Result<Benchmark> {
    if request.limit == 0 || request.limit > 10_000 {
        return Err(Error(
            "limit must be between 1 and 10000 unique cases".into(),
        ));
    }
    if !["train", "test"].contains(&request.split.as_str()) {
        return Err(Error("split must be train or test".into()));
    }
    if request.source.trim().is_empty() {
        return Err(Error("source is required".into()));
    }
    let source = if request.source.trim_end_matches('/') == RAGTRUTH.trim_end_matches('/') {
        format!("{}data/{}-*.parquet", RAGTRUTH, request.split)
    } else {
        request.source.clone()
    };
    let columns = [
        "id",
        "query",
        "context",
        "output",
        "task_type",
        "quality",
        "model",
        "hallucination_labels",
    ];
    let frame = LazyFrame::scan_parquet(PlRefPath::new(&source), ScanArgsParquet::default())
        .and_then(|frame| {
            frame
                .select(columns.iter().map(|name| col(*name)).collect::<Vec<_>>())
                .filter(col("task_type").eq(lit("QA")))
                .limit(100_001)
                .collect()
        })
        // Cloud errors may contain URLs/credentials. Do not persist the raw error.
        .map_err(|_| {
            Error("Cannot load RAGTruth Parquet: check source, access, and required columns".into())
        })?;
    if frame.height() > 100_000 {
        return Err(Error(
            "Dataset exceeds the 100000 QA-row safety limit".into(),
        ));
    }
    let string_columns = columns
        .iter()
        .map(|name| {
            frame
                .column(name)
                .and_then(|column| column.str())
                .map_err(|_| Error(format!("Column {name} must contain strings")))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut cases = Vec::<Case>::new();
    let mut case_positions = BTreeMap::new();
    let mut documents = BTreeMap::new();
    for row in 0..frame.height() {
        let values = string_columns
            .iter()
            .zip(columns)
            .map(|(column, name)| {
                column
                    .get(row)
                    .ok_or_else(|| Error(format!("Null {name} at QA row {row}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let [
            id,
            raw_query,
            raw_context,
            output,
            _task_type,
            quality,
            model,
            raw_labels,
        ]: [&str; 8] = values.try_into().expect("eight selected RAGTruth columns");
        let query = raw_query.trim();
        let context = raw_context.trim();
        if query.is_empty() || query.len() > 16 * 1024 || context.is_empty() {
            return Err(Error(format!(
                "Empty/oversized query or empty context at QA row {row}"
            )));
        }
        let document_id = digest(context.as_bytes());
        let case_id = digest(&serde_json::to_vec(&(query, &document_id))?);
        let position = if let Some(position) = case_positions.get(&case_id) {
            *position
        } else {
            if cases.len() >= request.limit {
                continue;
            }
            let position = cases.len();
            case_positions.insert(case_id.clone(), position);
            documents
                .entry(document_id.clone())
                .or_insert_with(|| Document {
                    id: document_id.clone(),
                    filename: format!("ragtruth-{document_id}.md"),
                    text: context.into(),
                    revision: document_id.clone(),
                });
            cases.push(Case {
                id: case_id,
                query: query.into(),
                document_id,
                reference_outputs: vec![],
            });
            position
        };
        let labels: serde_json::Value = serde_json::from_str(raw_labels)
            .map_err(|_| Error(format!("Invalid hallucination_labels JSON at QA row {row}")))?;
        if !labels.is_array() {
            return Err(Error(format!(
                "hallucination_labels must be an array at QA row {row}"
            )));
        }
        cases[position].reference_outputs.push(ReferenceOutput {
            id: id.into(),
            output: output.into(),
            model: model.into(),
            quality: quality.into(),
            hallucination_labels: labels,
        });
    }
    if cases.is_empty() {
        return Err(Error("No QA cases found in the dataset".into()));
    }
    Ok(Benchmark {
        id: Uuid::new_v4(),
        source,
        split: request.split.clone(),
        metric_kind: "paired_context_recovery_v1".into(),
        cases,
        documents: documents.into_values().collect(),
    })
}
