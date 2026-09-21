# RAG benchmarking development module

This module is for developers evolving the Genesis storage and Nebula RAG pipeline.
Its frontend and backend live together in this submodule; Genesis is a thin host.

## Frontend implementation

- Build a metrics dashboard inside the internal version of Genesis. Include its
  registration, host integration, and resources only in internal builds; verify
  they are absent from release builds, rather than merely hiding navigation.
- Read `README.md` for the existing HTTP interface and metric interpretation.
  Use `apps/backend/src/lib.rs`, `config.rs`, `server.rs`, and the relevant named
  benchmark module as the source of truth for request shapes and responses.
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
  responsibilities; the current benchmark server does not launch Nebula.
- In the Genesis checkout, inspect `genesis/scripts/generate-registry.ts` and
  `pnpm-workspace.yaml` when wiring the dashboard. They discover `Modules/dev` for development only; keep it excluded from release builds.

## Backend direction and isolation

Keep benchmark policy in its named `apps/backend/src/<benchmark>.rs` module:
definition validation, snapshot initialization/loading, run support, candidate
selection, evaluation, and benchmark-specific CSV columns. `ragtruth.rs` and
`hotpotqa.rs` are the working implementations. LongMemEval, TempRAGEval, QASPER,
AbstentionBench, MultiHop-RAG, and RAGBench have named modules but remain
catalog-only; do not imply their loaders or official evaluators are implemented.

`main.rs`, `lib.rs`, `config.rs`, and `server.rs` assemble common infrastructure.
The inline `server::persistence` and `server::generation` modules own storage and
Nebula execution mechanics, with policy supplied through `BenchmarkModule` and
`AnswerEvaluation`, not benchmark-name branches. Register new modules in
`lib::BENCHMARKS` and add matching YAML entries. Use `MetricValues` for
evaluator-specific numeric metrics; each evaluator owns their interpretation and
CSV output. Keep the existing API, YAML, saved snapshots/runs, and fingerprints
compatible. The existing `Case` fields preserve the current wire format; new
data shapes may require explicit versioned extensions.

Leave `init.rs` to the user. It is unchanged and does not implement a first-run
wizard or CRUD. Benchmark-module `initialize()` functions prepare snapshots;
they are separate from application initialization.

Use the actual Genesis storage and Nebula implementations in experimental
checkouts, with separate storage roots and backend processes. Successful changes
should merge directly into production code. Genesis storage extraction/reuse is
planned, not implemented here: the current Rust backend exports its own Markdown
corpus and calls an already-running standalone Nebula through `/retrieve` or `/query`.
Generation requires a configured remote-enabled Nebula; benchmark code does not
launch it or store provider credentials in frontend state.
