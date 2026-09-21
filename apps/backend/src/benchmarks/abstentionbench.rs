//! AbstentionBench registration and preparation boundary.
//!
//! A future loader must select explicit component datasets and preserve their
//! prompting protocols. This suite has no single shared retrieval corpus.
//! Evaluation must distinguish abstention detection, expected abstention, and
//! answer correctness; a refusal on an unanswerable question can be correct.

use crate::{
    Benchmark, BenchmarkModule, Error, Result,
    config::{BenchmarkDefinition, ResolvedBenchmark, validate_external_definition},
};

pub struct AbstentionBench;
pub static ABSTENTIONBENCH: AbstentionBench = AbstentionBench;

impl BenchmarkModule for AbstentionBench {
    fn key(&self) -> &'static str {
        "abstentionbench"
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

/// Refuse preparation until component loading and abstention evaluation are integrated.
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
