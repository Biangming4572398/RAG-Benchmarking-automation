//! LongMemEval S-cleaned registration and preparation boundary.
//!
//! A future loader must isolate each question's complete history and preserve
//! session dates. Answer-session labels stay outside indexed text. Its evaluator
//! needs question-type correctness judging and explicit abstention handling;
//! neither oracle history nor HotpotQA token F1 implements this protocol.

use crate::{
    Benchmark, BenchmarkModule, Error, Result,
    config::{BenchmarkDefinition, ResolvedBenchmark, validate_external_definition},
};

pub struct LongMemEval;
pub static LONGMEMEVAL: LongMemEval = LongMemEval;

impl BenchmarkModule for LongMemEval {
    fn key(&self) -> &'static str {
        "longmemeval-cleaned"
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

/// Refuse preparation until the history loader and evaluator are integrated.
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
