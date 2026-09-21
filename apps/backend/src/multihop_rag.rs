//! MultiHop-RAG registration and preparation boundary.
//!
//! A future loader must import the separate news corpus and preserve publication
//! metadata. Evidence labels stay separate from candidate documents. Retrieval
//! and answer evaluation need their own pinned protocols, preserving null queries
//! for generation even when retrieval scoring excludes them.

use crate::{
    Benchmark, BenchmarkModule, Error, Result,
    config::{BenchmarkDefinition, ResolvedBenchmark, validate_external_definition},
};

pub struct MultiHopRag;
pub static MULTIHOP_RAG: MultiHopRag = MultiHopRag;

impl BenchmarkModule for MultiHopRag {
    fn key(&self) -> &'static str {
        "multihop-rag"
    }

    fn adapter(&self) -> &'static str {
        "external_suite"
    }

    fn metric_kind(&self) -> &'static str {
        "external_evaluation"
    }

    fn validate_definition(&self, definition: &BenchmarkDefinition) -> Result<()> {
        validate_external_definition(definition)
    }

    fn initialize(&self, request: &ResolvedBenchmark) -> Result<Benchmark> {
        initialize(request)
    }
}

/// Refuse preparation until the news corpus and benchmark evaluators are integrated.
pub fn initialize(request: &ResolvedBenchmark) -> Result<Benchmark> {
    request.definition.validate_for(&request.key)?;
    Err(Error(format!(
        "{} is registered in the catalog but its dataset loader and evaluator are not integrated. {}",
        request.definition.name,
        request
            .definition
            .preparation
            .as_deref()
            .unwrap_or_default(),
    )))
}
