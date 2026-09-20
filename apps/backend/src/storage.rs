use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::{
    Error, Result,
    answers::{AnswerReviewRequest, AnswerRun, AnswerRunSummary, ReviewFailure},
    catalog::ResolvedBenchmark,
    load_benchmarks::{Benchmark, digest},
    trials::{Run, RunStatus},
};

pub struct Store {
    root: PathBuf,
    // Prevent two servers from racing recovery/progress writes in the same root.
    _lock: File,
    answer_review_lock: Mutex<()>,
    // The process owns this directory exclusively. Polling summaries must not reread every
    // generated answer and evidence history; run.json remains the single durable record.
    answer_summaries: Mutex<BTreeMap<Uuid, AnswerRunSummary>>,
}

#[derive(Deserialize, Serialize)]
pub struct BenchmarkInfo {
    pub id: Uuid,
    pub source: String,
    pub split: String,
    pub metric_kind: String,
    pub case_count: usize,
    pub document_count: usize,
    pub corpus_path: PathBuf,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<ResolvedBenchmark>,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(".server.lock"))?;
        lock.try_lock()
            .map_err(|_| Error("Benchmark data directory is already in use".into()))?;
        fs::create_dir_all(root.join("benchmarks"))?;
        fs::create_dir_all(root.join("runs"))?;
        fs::create_dir_all(root.join("answer-runs"))?;
        let store = Self {
            root,
            _lock: lock,
            answer_review_lock: Mutex::new(()),
            answer_summaries: Mutex::new(BTreeMap::new()),
        };
        for mut run in store.runs()? {
            if run.status == RunStatus::Running {
                run.status = RunStatus::Interrupted;
                run.error =
                    Some("Server stopped before the run completed; partial CSV is retained".into());
                run.finished_at_ms = Some(crate::trials::now_ms());
                store.save_run(&run)?;
            }
        }
        for id in ids_in(&store.root.join("answer-runs"))? {
            let mut run = store.answer_run(id)?;
            if run.summary.status == RunStatus::Running {
                run.summary.status = RunStatus::Interrupted;
                run.summary.error = Some("Server stopped before answer generation completed; partial answers are retained".into());
                run.summary.finished_at_ms = Some(crate::trials::now_ms());
                store.save_answer_run(&run)?;
            } else {
                store
                    .answer_summaries
                    .lock()
                    .map_err(|_| Error("Answer summaries are unavailable".into()))?
                    .insert(id, run.summary);
            }
        }
        Ok(store)
    }

    pub fn save_benchmark(&self, benchmark: &Benchmark) -> Result<BenchmarkInfo> {
        let parent = self.root.join("benchmarks");
        let stage = tempfile::tempdir_in(&parent)?;
        let corpus = stage.path().join("corpus");
        fs::create_dir(&corpus)?;
        for document in &benchmark.documents {
            fs::write(corpus.join(&document.filename), &document.text)?;
        }
        atomic_json(&stage.path().join("benchmark.json"), benchmark)?;
        fs::rename(stage.path(), parent.join(benchmark.id.to_string()))?;
        self.benchmark_info(benchmark.id)
    }

    pub fn benchmark(&self, id: Uuid) -> Result<Benchmark> {
        read_json(&self.benchmark_path(id).join("benchmark.json"))
    }

    pub fn benchmark_info(&self, id: Uuid) -> Result<BenchmarkInfo> {
        let benchmark = self.benchmark(id)?;
        Ok(BenchmarkInfo {
            id,
            fingerprint: digest(&serde_json::to_vec(&(
                &benchmark.metric_kind,
                &benchmark.cases,
                &benchmark.documents,
            ))?),
            case_count: benchmark.cases.len(),
            document_count: benchmark.documents.len(),
            corpus_path: self.benchmark_path(id).join("corpus"),
            source: benchmark.source,
            split: benchmark.split,
            metric_kind: benchmark.metric_kind,
            configuration: benchmark.configuration,
        })
    }

    pub fn benchmarks(&self) -> Result<Vec<BenchmarkInfo>> {
        ids_in(&self.root.join("benchmarks"))?
            .into_iter()
            .map(|id| self.benchmark_info(id))
            .collect()
    }

    pub fn create_run(&self, run: &Run) -> Result<()> {
        let stage = tempfile::tempdir_in(self.root.join("runs"))?;
        atomic_json(&stage.path().join("run.json"), run)?;
        fs::rename(stage.path(), self.run_path(run.id))?;
        Ok(())
    }

    pub fn save_run(&self, run: &Run) -> Result<()> {
        atomic_json(&self.run_path(run.id).join("run.json"), run)
    }

    pub fn run(&self, id: Uuid) -> Result<Run> {
        read_json(&self.run_path(id).join("run.json"))
    }

    pub fn runs(&self) -> Result<Vec<Run>> {
        ids_in(&self.root.join("runs"))?
            .into_iter()
            .map(|id| self.run(id))
            .collect()
    }

    pub fn create_scores(&self, id: Uuid) -> Result<File> {
        Ok(OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.scores_path(id))?)
    }

    pub fn scores(&self, id: Uuid) -> Result<Vec<u8>> {
        Ok(fs::read(self.scores_path(id))?)
    }

    pub fn create_answer_run(&self, run: &AnswerRun) -> Result<()> {
        let mut summaries = self
            .answer_summaries
            .lock()
            .map_err(|_| Error("Answer summaries are unavailable".into()))?;
        let parent = self.root.join("answer-runs");
        let stage = tempfile::tempdir_in(&parent)?;
        atomic_json(&stage.path().join("run.json"), run)?;
        fs::rename(stage.path(), parent.join(run.summary.id.to_string()))?;
        summaries.insert(run.summary.id, run.summary.clone());
        Ok(())
    }

    pub fn save_answer_run(&self, run: &AnswerRun) -> Result<()> {
        let mut summaries = self
            .answer_summaries
            .lock()
            .map_err(|_| Error("Answer summaries are unavailable".into()))?;
        atomic_json(&self.answer_run_path(run.summary.id), run)?;
        summaries.insert(run.summary.id, run.summary.clone());
        Ok(())
    }

    pub fn answer_run(&self, id: Uuid) -> Result<AnswerRun> {
        read_json(&self.answer_run_path(id))
    }

    pub fn answer_runs(&self) -> Result<Vec<AnswerRunSummary>> {
        Ok(self
            .answer_summaries
            .lock()
            .map_err(|_| Error("Answer summaries are unavailable".into()))?
            .values()
            .cloned()
            .collect())
    }

    pub fn review_answer(
        &self,
        id: Uuid,
        case_id: &str,
        review: AnswerReviewRequest,
    ) -> std::result::Result<AnswerRun, ReviewFailure> {
        // Terminal-only reviews cannot race generation. Serialize concurrent reviewers so each
        // read/modify/atomic-write retains the other case's latest saved review.
        let _guard = self.answer_review_lock.lock().map_err(|_| {
            ReviewFailure::Storage(Error("Answer review storage is unavailable".into()))
        })?;
        let mut run = self.answer_run(id).map_err(|_| ReviewFailure::NotFound)?;
        run.review(case_id, review)?;
        self.save_answer_run(&run).map_err(ReviewFailure::Storage)?;
        Ok(run)
    }

    fn answer_run_path(&self, id: Uuid) -> PathBuf {
        self.root
            .join("answer-runs")
            .join(id.to_string())
            .join("run.json")
    }

    fn benchmark_path(&self, id: Uuid) -> PathBuf {
        self.root.join("benchmarks").join(id.to_string())
    }
    fn run_path(&self, id: Uuid) -> PathBuf {
        self.root.join("runs").join(id.to_string())
    }
    fn scores_path(&self, id: Uuid) -> PathBuf {
        self.run_path(id).join("scores.csv")
    }
}

fn ids_in(path: &Path) -> Result<Vec<Uuid>> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Ok(id) = entry.file_name().to_string_lossy().parse()
        {
            ids.push(id);
        }
    }
    ids.sort();
    Ok(ids)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_reader(File::open(path)?)?)
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("store path has parent"))?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| Error(error.error.to_string()))?;
    Ok(())
}
