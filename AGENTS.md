# RAG benchmarking development module

This module is for developers evolving the Genesis storage and Nebula RAG pipeline.
Its frontend and backend live together in this submodule; Genesis is a thin host.

## Frontend implementation

- Build a metrics dashboard inside the internal version of Genesis. Include its
  registration, host integration, and resources only in internal builds; verify
  they are absent from release builds, rather than merely hiding navigation.
- Read `README.md` for the existing HTTP interface and metric interpretation.
  Use `apps/backend/src/lib.rs`, `config.rs`, `server.rs`, and the relevant named
  module under `apps/backend/src/benchmarks` as the source of truth for request
  shapes and responses.
  Backend changes continue independently;
  coordinate contract changes rather than inventing endpoints.
- Keep result tables first. Benchmark definitions and dataset preparation are
  managed through YAML/Git and backend setup, not frontend catalog editing.
  Support prepared snapshots, run creation, progress/status, saved results and CSV.
- Keep Retrieval and Generated answers separate. Retrieval scores measure paired
  context recovery. RAGTruth answer quality uses explicit `manual_review_v1` human reviews
  with answered/reviewed coverage; pending reviews have no scores. HotpotQA uses
  `hotpotqa_answer_v1` full-answer exact match/token F1 over all snapshot cases,
  with the supplied candidate passages per question (typically ten) and separate optional human review.
  Never pass reference answers into retrieval or generation. Never reuse
  RAGTruth historical annotations as labels for newly generated answers.
- Record actual embedding/generation provenance from Nebula. Compare only complete,
  fully scored HotpotQA runs or fully reviewed RAGTruth answer runs with matching fingerprints, top-k, evaluator version
  and candidate source sets; partial results remain inspectable.
- Connect through a host-owned proxy or bridge to the loopback benchmark API,
  which requires no authentication. Nebula credentials stay in backend/host
  configuration, outside renderer state and bundles.
  Model selection, Nebula launch, and experiment orchestration remain backend/host
  responsibilities. The module-owned development launcher may build and start
  Nebula through its public `@genesis/nebula/benchmarking` entry point; the Rust
  server consumes only its loopback connection.
- In the Genesis checkout, inspect `genesis/scripts/generate-registry.ts` and
  `pnpm-workspace.yaml` when wiring the dashboard. They discover `Modules/dev` for development only; keep it excluded from release builds.

## Backend direction and isolation

Keep benchmark policy in `apps/backend/src/benchmarks/<benchmark>.rs`:
definition validation, snapshot initialization/loading, run support, candidate
selection, evaluation, and benchmark-specific CSV columns. `ragtruth.rs` and
`hotpotqa.rs` are the working implementations. LongMemEval, TempRAGEval, QASPER,
AbstentionBench, MultiHop-RAG, and RAGBench have named modules but remain
catalog-only; do not imply their loaders or official evaluators are implemented.

`main.rs`, `lib.rs`, `config.rs`, and `server.rs` assemble common infrastructure.
The inline `server::persistence` and `server::generation` modules own storage and
Nebula execution mechanics, with policy supplied through `BenchmarkModule` and
`AnswerEvaluation`, not benchmark-name branches. Declare new modules in
`benchmarks/mod.rs`, register their instances in `lib::BENCHMARKS`, and add matching
YAML entries. Internal imports use explicit paths such as
`crate::benchmarks::ragtruth`. Use `MetricValues` for
evaluator-specific numeric metrics; each evaluator owns their interpretation and
CSV output. Keep the existing API, YAML, saved snapshots/runs, and fingerprints
compatible. The existing `Case` fields preserve the current wire format; new
data shapes may require explicit versioned extensions.

Keep benchmark comparison results in `BENCHMARK_DATA_DIR/results/<module-key>.csv`
with one row per execution, using global run numbers, architecture names, mode,
status, snapshot fingerprint and settings. `metric.*` columns hold retrieval or
automatic scores; `review.*` columns hold manual scores. Build each table's metric
union and leave unavailable values blank. Preserve the existing detailed per-run
JSON, retrieval CSV, answer exports and failure logs.

`results/runs.json` owns the next run number and the number-to-run references,
including descriptions. Requests accept an optional description up to 4000 UTF-8
bytes. Preserve registry descriptions edited while the backend is stopped. Keep
run numbers stable across restarts; legacy runs receive numbers in start-time/UUID
order. The comparison CSVs are derived and rebuilt at startup after interrupted
runs are marked. Files are atomic individually, not as a multi-file transaction;
recovery uses saved run details and the registry. Do not discard the registry or
introduce a database stack for this mapping.

Application initialization belongs in `main.rs`. The bundled benchmark YAML must
exist at startup; do not generate a replacement when it is missing.
Server startup prepares all missing supported snapshots from pinned YAML definitions,
including generation-only datasets without requiring a generation profile. Reuse
saved snapshots, publish only exported passage text into the explicitly configured
Nebula corpus, and wait for authenticated reindex completion and exact passage
revisions. Reference answers and snapshot JSON must never be indexed.
Expose responsive liveness and initialization progress while this work runs; block
new loads/runs until ready and retain failures in `initialization.json`. Startup
must not create suite/run records, consume run numbers, or generate paid answers.
Restart retries initialization while reusing completed snapshots. Suite preparation
remains a fallback, with the accepted suite persisted before any work.


Use the actual Genesis storage and Nebula implementations in experimental
checkouts, with separate storage roots and backend processes. Successful changes
should merge directly into production code. Genesis storage extraction/reuse is
planned, not implemented here: the current Rust backend exports its own Markdown
corpus and calls an already-running standalone Nebula through `/retrieve` or `/query`.
Generation requires a configured remote-enabled Nebula. The development launcher
can supervise a separate Nebula using the private module's shared configuration;
provider credentials remain there and never enter frontend state. Reuse only the
model bundle from normal Genesis storage, keeping benchmark corpus/index state
separate. Nebula's managed launcher
provisions its verified model bundle and enables the benchmark reindex capability.
