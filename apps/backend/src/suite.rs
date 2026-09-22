//! Durable experiment orchestration. Benchmark modules retain all dataset and scoring policy.
use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Benchmark, Error, Result, RunStatus, StartAnswerRunRequest, StartFailure, StartRunRequest,
    config::{Catalog, LoadRequest, NebulaConfig, ResolvedBenchmark},
    initialize_benchmark, module_for_definition, module_for_snapshot, now_ms,
    server::Store,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartSuiteRequest {
    #[serde(default = "crate::default_architecture_label")]
    pub architecture_label: String,
    #[serde(default)]
    pub description: String,
    pub profile_id: Option<String>,
    /// Retrieval only; generation uses its evaluator's existing fixed query settings.
    pub top_k: Option<usize>,
}

impl StartSuiteRequest {
    pub fn validate(&self) -> Result<()> {
        if !crate::valid_label(&self.architecture_label, 256) {
            return Err(Error(
                "architecture_label must contain 1–256 bytes without surrounding whitespace".into(),
            ));
        }
        if self.description.len() > 4000 {
            return Err(Error("description must be at most 4000 bytes".into()));
        }
        if self
            .profile_id
            .as_ref()
            .is_some_and(|id| !crate::valid_label(id, 128))
        {
            return Err(Error(
                "profile_id must contain 1–128 bytes without surrounding whitespace".into(),
            ));
        }
        if self.top_k.is_some_and(|top_k| !(1..=100).contains(&top_k)) {
            return Err(Error("top_k must be 1–100".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SuiteRun {
    pub id: Uuid,
    pub run_number: u64,
    pub request: StartSuiteRequest,
    pub status: RunStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub error: Option<String>,
    #[serde(default)]
    pub phase: SuitePhase,
    #[serde(default)]
    pub preparations: Vec<SuitePreparation>,
    pub items: Vec<SuiteItem>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuitePhase {
    Preparing,
    Indexing,
    #[default]
    Running,
    Finished,
}

/// Definitions are resolved when the suite is accepted, before any download begins.
#[derive(Clone, Deserialize, Serialize)]
pub struct SuitePreparation {
    pub benchmark: String,
    pub benchmark_id: Option<Uuid>,
    pub configuration: Option<ResolvedBenchmark>,
    pub status: SuiteItemStatus,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuiteMode {
    Retrieval,
    Generation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuiteItemStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Interrupted,
    Skipped,
}

impl From<RunStatus> for SuiteItemStatus {
    fn from(status: RunStatus) -> Self {
        match status {
            RunStatus::Running => Self::Running,
            RunStatus::Completed => Self::Completed,
            RunStatus::Failed => Self::Failed,
            RunStatus::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SuiteItem {
    pub benchmark: String,
    pub benchmark_id: Option<Uuid>,
    pub mode: Option<SuiteMode>,
    pub run_id: Option<Uuid>,
    pub status: SuiteItemStatus,
    pub reason: Option<String>,
}

/// Resolve canonical modules identically for startup and suite runs.
fn inventory(
    store: &Store,
    catalog: &Catalog,
) -> Result<Vec<(&'static dyn crate::BenchmarkModule, SuitePreparation)>> {
    let mut snapshots: BTreeMap<String, Benchmark> = BTreeMap::new();
    let mut modules = BTreeMap::new();
    let mut definitions = BTreeMap::new();
    for (key, definition) in &catalog.benchmarks {
        let module = module_for_definition(Some(key), definition)?;
        let canonical = module.key().to_string();
        modules.insert(canonical.clone(), module);
        // Prefer the canonical entry; otherwise the first sorted alias wins.
        if key == module.key() || !definitions.contains_key(&canonical) {
            definitions.insert(
                canonical,
                catalog.resolve(&LoadRequest {
                    benchmark: key.clone(),
                    limit: None,
                })?,
            );
        }
    }
    // Store sorts oldest first by save time, then UUID for deterministic ties.
    // Catalog aliases share the canonical module key used by comparison tables.
    for benchmark in store.benchmarks_by_saved_time()? {
        let module = module_for_snapshot(&benchmark)?;
        modules.insert(module.key().to_string(), module);
        snapshots.insert(module.key().to_string(), benchmark);
    }
    Ok(modules
        .into_iter()
        .map(|(key, module)| {
            let benchmark = snapshots.get(&key);
            let preparation = SuitePreparation {
                benchmark: key.clone(),
                benchmark_id: benchmark.map(|value| value.id),
                configuration: benchmark
                    .and_then(|value| value.configuration.clone())
                    .or_else(|| definitions.get(&key).cloned()),
                status: if benchmark.is_some() {
                    SuiteItemStatus::Completed
                } else {
                    SuiteItemStatus::Queued
                },
                reason: None,
            };
            (module, preparation)
        })
        .collect())
}

/// Prepare all integrated datasets at startup, independent of selected run modes/profiles.
pub fn plan_preparations(store: &Store, catalog: &Catalog) -> Result<Vec<SuitePreparation>> {
    Ok(inventory(store, catalog)?
        .into_iter()
        .filter(|(module, _)| module.supports_retrieval() || module.answer_evaluation().is_some())
        .map(|(_, preparation)| preparation)
        .collect())
}

/// Snapshot choices and missing-dataset definitions are fixed before accepting a suite.
pub fn plan(store: &Store, catalog: &Catalog, request: StartSuiteRequest) -> Result<SuiteRun> {
    request.validate()?;
    let mut items = Vec::new();
    let mut preparations = Vec::new();
    // Include prepared snapshots whose definitions were removed from the current catalog.
    for (module, mut preparation) in inventory(store, catalog)? {
        let base = SuiteItem {
            benchmark: preparation.benchmark.clone(),
            benchmark_id: preparation.benchmark_id,
            mode: None,
            run_id: None,
            status: SuiteItemStatus::Skipped,
            reason: None,
        };
        if !module.supports_retrieval() && module.answer_evaluation().is_none() {
            items.push(SuiteItem {
                reason: Some(
                    "Catalog only: dataset loader and evaluator are not integrated".into(),
                ),
                ..base
            });
        } else {
            let executable = module.supports_retrieval() || request.profile_id.is_some();
            if !executable {
                preparation.status = SuiteItemStatus::Skipped;
                preparation.reason = Some("No generation profile selected".into());
            }
            preparations.push(preparation);
            if module.supports_retrieval() {
                items.push(SuiteItem {
                    mode: Some(SuiteMode::Retrieval),
                    status: SuiteItemStatus::Queued,
                    ..base.clone()
                });
            }
            if module.answer_evaluation().is_some() {
                let (status, reason) = if request.profile_id.is_some() {
                    (SuiteItemStatus::Queued, None)
                } else {
                    (
                        SuiteItemStatus::Skipped,
                        Some("No generation profile selected".into()),
                    )
                };
                items.push(SuiteItem {
                    mode: Some(SuiteMode::Generation),
                    status,
                    reason,
                    ..base
                });
            }
        }
    }
    Ok(SuiteRun {
        id: Uuid::new_v4(),
        run_number: 0,
        request,
        status: RunStatus::Running,
        started_at_ms: now_ms(),
        finished_at_ms: None,
        error: None,
        phase: SuitePhase::Preparing,
        preparations,
        items,
    })
}

/// Load/reuse snapshots without creating experiments or consuming run numbers.
/// Report each outcome before advancing, including individual preparation failures.
pub async fn prepare_snapshots(
    store: Arc<Store>,
    preparations: &mut [SuitePreparation],
    mut progress: impl FnMut(&[SuitePreparation]) -> Result<()>,
) -> Result<Vec<Benchmark>> {
    let mut snapshots = BTreeMap::new();
    for index in 0..preparations.len() {
        let preparation = preparations[index].clone();
        if preparation.status == SuiteItemStatus::Skipped {
            continue;
        }
        preparations[index].status = SuiteItemStatus::Running;
        preparations[index].reason = None;
        progress(preparations)?;
        let result = if let Some(id) = preparation.benchmark_id {
            store.benchmark(id)
        } else if let Some(configuration) = preparation.configuration {
            let worker_store = store.clone();
            match tokio::task::spawn_blocking(move || {
                let benchmark = initialize_benchmark(&configuration)?;
                worker_store.save_benchmark(&benchmark)?;
                Ok::<_, Error>(benchmark)
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(Error(
                    "Benchmark preparation worker stopped unexpectedly".into(),
                )),
            }
        } else {
            Err(Error(
                "No pinned catalog definition is available to prepare this benchmark".into(),
            ))
        };
        let result = result.and_then(|benchmark| {
            if module_for_snapshot(&benchmark)?.key() != preparation.benchmark {
                return Err(Error(
                    "Prepared snapshot does not match its benchmark".into(),
                ));
            }
            Ok(benchmark)
        });
        match result {
            Ok(benchmark) => {
                preparations[index].benchmark_id = Some(benchmark.id);
                preparations[index].status = SuiteItemStatus::Completed;
                snapshots.insert(benchmark.id, benchmark);
            }
            Err(error) => {
                preparations[index].status = SuiteItemStatus::Failed;
                preparations[index].reason = Some(error.to_string());
            }
        }
        progress(preparations)?;
    }
    Ok(snapshots.into_values().collect())
}

/// Prepare missing datasets and return snapshots with queued executions.
/// Publish/index this corpus before calling `execute`.
pub async fn prepare(store: Arc<Store>, suite: &mut SuiteRun) -> Result<Vec<Benchmark>> {
    if suite.run_number == 0 {
        return Err(Error(
            "Persist the accepted suite before preparing its datasets".into(),
        ));
    }
    suite.phase = SuitePhase::Preparing;
    store.save_suite(suite)?;
    let mut preparations = suite.preparations.clone();
    for preparation in &mut preparations {
        if !suite.items.iter().any(|item| {
            item.benchmark == preparation.benchmark && item.status == SuiteItemStatus::Queued
        }) {
            preparation.status = SuiteItemStatus::Skipped;
        }
    }
    let prepared = prepare_snapshots(store.clone(), &mut preparations, |progress| {
        suite.preparations = progress.to_vec();
        for preparation in progress {
            for item in &mut suite.items {
                if item.benchmark != preparation.benchmark {
                    continue;
                }
                if let Some(id) = preparation.benchmark_id {
                    item.benchmark_id = Some(id);
                }
                if preparation.status == SuiteItemStatus::Failed
                    && item.status == SuiteItemStatus::Queued
                {
                    item.status = SuiteItemStatus::Failed;
                    item.reason = Some(format!(
                        "Dataset preparation failed: {}",
                        preparation.reason.as_deref().unwrap_or("Unknown error")
                    ));
                }
            }
        }
        store.save_suite(suite)
    })
    .await?;
    let mut snapshots: BTreeMap<_, _> = prepared
        .into_iter()
        .map(|value| (value.id, value))
        .collect();
    // Old suites predate preparation records but already pin saved snapshots.
    for item in &suite.items {
        if item.status == SuiteItemStatus::Queued
            && let Some(id) = item.benchmark_id
            && !snapshots.contains_key(&id)
        {
            snapshots.insert(id, store.benchmark(id)?);
        }
    }
    Ok(snapshots.into_values().collect())
}

pub async fn execute(store: Arc<Store>, config: NebulaConfig, mut suite: SuiteRun) -> Result<()> {
    suite.phase = SuitePhase::Running;
    store.save_suite(&suite)?;
    for index in 0..suite.items.len() {
        if suite.items[index].status != SuiteItemStatus::Queued {
            continue;
        }
        // A preparation/worker failure in one benchmark must not suppress later benchmarks.
        if let Err(error) = execute_item(&store, &config, &mut suite, index).await {
            suite.items[index].status = SuiteItemStatus::Failed;
            suite.items[index].reason = Some(error.to_string());
        }
        store.save_suite(&suite)?;
    }
    suite.status = if suite.items.iter().any(|item| {
        matches!(
            item.status,
            SuiteItemStatus::Failed | SuiteItemStatus::Interrupted
        )
    }) {
        RunStatus::Failed
    } else if suite
        .items
        .iter()
        .all(|item| item.status == SuiteItemStatus::Skipped)
    {
        suite.error = Some("No benchmark executions could run; inspect the skipped reasons".into());
        RunStatus::Failed
    } else {
        RunStatus::Completed
    };
    suite.finished_at_ms = Some(now_ms());
    suite.phase = SuitePhase::Finished;
    store.save_suite(&suite)
}

async fn execute_item(
    store: &Arc<Store>,
    config: &NebulaConfig,
    suite: &mut SuiteRun,
    index: usize,
) -> Result<()> {
    let item = &suite.items[index];
    let benchmark = store.benchmark(
        item.benchmark_id
            .ok_or_else(|| Error("Suite item has no snapshot".into()))?,
    )?;
    let fingerprint = store.benchmark_info(benchmark.id)?.fingerprint;
    let module = module_for_snapshot(&benchmark)?;
    match item.mode {
        Some(SuiteMode::Retrieval) => {
            let run = module.prepare_retrieval(
                StartRunRequest {
                    benchmark_id: benchmark.id,
                    label: suite.request.architecture_label.clone(),
                    description: suite.request.description.clone(),
                    top_k: suite.request.top_k,
                }
                .resolve(&benchmark),
                &benchmark,
                fingerprint,
            )?;
            let id = run.id;
            suite.items[index].run_id = Some(id);
            suite.items[index].status = SuiteItemStatus::Running;
            // Persist the association before the child so restart recovery can always join it.
            store.save_suite(suite)?;
            store.create_run(&run)?;
            if tokio::spawn(module.run_retrieval(store.clone(), config.clone(), benchmark, run))
                .await
                .is_err()
            {
                let mut failed = store.run(id)?;
                failed.status = RunStatus::Failed;
                failed.error = Some("Benchmark worker stopped unexpectedly".into());
                failed.finished_at_ms = Some(now_ms());
                store.save_run(&failed)?;
            }
            let run = store.run(id)?;
            suite.items[index].status = run.status.into();
            suite.items[index].reason = run.error;
        }
        Some(SuiteMode::Generation) => {
            let request = StartAnswerRunRequest {
                benchmark_id: benchmark.id,
                architecture_label: suite.request.architecture_label.clone(),
                description: suite.request.description.clone(),
                profile_id: suite
                    .request
                    .profile_id
                    .clone()
                    .ok_or_else(|| Error("No generation profile selected".into()))?,
            };
            let run = match module
                .prepare_answers(config, request, &benchmark, fingerprint)
                .await
            {
                Ok(run) => run,
                Err(StartFailure::Unavailable(reason)) => {
                    suite.items[index].status = SuiteItemStatus::Skipped;
                    suite.items[index].reason = Some(reason);
                    return Ok(());
                }
                Err(StartFailure::Invalid(reason)) => return Err(Error(reason)),
            };
            let id = run.summary.id;
            suite.items[index].run_id = Some(id);
            suite.items[index].status = SuiteItemStatus::Running;
            store.save_suite(suite)?;
            store.create_answer_run(&run)?;
            if tokio::spawn(module.run_answers(store.clone(), config.clone(), benchmark, run))
                .await
                .is_err()
            {
                let mut failed = store.answer_run(id)?;
                failed.summary.status = RunStatus::Failed;
                failed.summary.error = Some("Answer generation worker stopped unexpectedly".into());
                failed.summary.finished_at_ms = Some(now_ms());
                store.save_answer_run(&failed)?;
            }
            let run = store.answer_run(id)?;
            suite.items[index].status = run.summary.status.into();
            suite.items[index].reason = run.summary.error;
        }
        None => return Err(Error("Suite item has no execution mode".into())),
    }
    Ok(())
}
