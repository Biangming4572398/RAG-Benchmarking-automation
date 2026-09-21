//! RAGBench registration and preparation boundary.
//!
//! A future loader must select subsets and index candidate documents separately
//! from historical responses and TRACe labels. Those labels describe recorded
//! outputs, not newly generated answers. New responses need fresh evaluation;
//! evaluator reproduction must preserve the original annotations and provenance.

use crate::{
    Benchmark, BenchmarkModule, Error, Result,
    config::{BenchmarkDefinition, ResolvedBenchmark, validate_external_definition},
};

pub struct RagBench;
pub static RAGBENCH: RagBench = RagBench;

impl BenchmarkModule for RagBench {
    fn key(&self) -> &'static str {
        "ragbench"
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

/// Refuse preparation until subset loading and fresh response evaluation are integrated.
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
