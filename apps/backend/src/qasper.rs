//! QASPER registration and preparation boundary.
//!
//! A future loader must scope each question to its complete paper and retain all
//! reference annotations outside indexed text. Evaluation needs multiple-reference
//! answer and paragraph-evidence F1, including yes/no and unanswerable responses.
//! Any exclusion of figure or table evidence must be recorded explicitly.

use crate::{
    Benchmark, BenchmarkModule, Error, Result,
    config::{BenchmarkDefinition, ResolvedBenchmark, validate_external_definition},
};

pub struct Qasper;
pub static QASPER: Qasper = Qasper;

impl BenchmarkModule for Qasper {
    fn key(&self) -> &'static str {
        "qasper"
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

/// Refuse preparation until paper loading and answer/evidence scoring are integrated.
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
