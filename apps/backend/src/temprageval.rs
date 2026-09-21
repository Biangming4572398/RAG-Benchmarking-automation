//! TempRAGEval registration and preparation boundary.
//!
//! A future loader needs accepted publisher access and the associated Wikipedia
//! corpus, preserving temporal constraints and multiple answer references. Gold
//! evidence must remain evaluation metadata, not replace the candidate corpus.
//! Evaluation must explicitly score temporal retrieval and reference answers.

use crate::{
    Benchmark, BenchmarkModule, Error, Result,
    config::{BenchmarkDefinition, ResolvedBenchmark, validate_external_definition},
};

pub struct TempRAGEval;
pub static TEMPRAGEVAL: TempRAGEval = TempRAGEval;

impl BenchmarkModule for TempRAGEval {
    fn key(&self) -> &'static str {
        "temprageval"
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

/// Refuse preparation until corpus access and temporal evaluation are integrated.
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
