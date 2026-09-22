# RAG benchmarking backend

The experimental architecture comparison dashboard lives in
[`apps/frontend`](apps/frontend/README.md). Open it with `pnpm benchmark:ui` from
the Genesis repository root, or use the Benchmarking dock item in development
Genesis. The development server compiles and starts the Rust backend when needed.
With Nebula's `benchmarking` branch installed, it also builds and starts Nebula
using that module's private Kimi configuration.
The dashboard uses a macOS-style sidebar to select one benchmark, with separate
Retrieval and Generated answers tables showing saved executions by run number.
It supports search, status filters, notes, CSV exports, evidence inspection, and
compatible-run comparisons.

The default architecture label is `baseline`, referring to the current Nebula
implementation at commit `a16724265d0142684349b773c5e475e2f79ea818`.
When evaluating later Nebula changes, supply a distinct architecture label and
record that revision in the run description. Runtime embedding and generation
model identities are still captured from Nebula for each run.

**Run all benchmarks** creates one durable suite. It selects the latest prepared
snapshot for each benchmark module, executes every supported mode in sequence,
and associates the resulting executions with one suite run number. Unsupported
benchmarks, missing snapshots, and unavailable generation are recorded with
skipped reasons. Suite history retains these outcomes alongside execution results.
RAGTruth and HotpotQA have working loaders and evaluators; LongMemEval — cleaned,
TempRAGEval, QASPER, AbstentionBench, MultiHop-RAG and RAGBench remain catalog-only
and are marked **Integration required**. Registration alone does not make a
benchmark runnable or produce scores.

Definitions are managed through Git in
[`apps/backend/benchmarks.yaml`](apps/backend/benchmarks.yaml). Prepare supported
snapshots with the HTTP API as described below. The dashboard's **Benchmark setup
& snapshots** section shows preparation guidance and saved corpus paths; it does
not edit definitions or load datasets. Its UI, API clients, styling, standalone
proxy, and backend build/lifecycle script live inside this submodule.

Developer-only Rust HTTP server for loading benchmark data with Polars, running
Nebula retrieval and answer generation, and retaining results and human reviews.
It does not register with the Genesis release build, install models, or modify
your normal Genesis storage. The development launcher can supervise a separate
Nebula process for benchmarks.
The benchmark backend uses files only; Nebula manages its own index separately.

## Backend layout

Each benchmark has one named module in `apps/backend/src/benchmarks` that owns its
definition validation, snapshot initialization/loading, supported run modes,
evaluation policy, and benchmark-specific CSV columns:

| Module                                                                                                  | Responsibility                                                                                                                   |
| ------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `ragtruth.rs`                                                                                           | Parquet QA loading, paired-context retrieval/scoring, generated-answer candidate selection, and human review                     |
| `hotpotqa.rs`                                                                                           | Distractor JSON loading/checksums, per-question candidate selection, answer exact match/token F1, and supplementary human review |
| `longmemeval.rs`, `temprageval.rs`, `qasper.rs`, `abstentionbench.rs`, `multihop_rag.rs`, `ragbench.rs` | Named registration and preparation boundaries; loading and evaluation remain unavailable                                         |

`benchmarks/mod.rs` declares these modules. Use explicit internal paths such as
`crate::benchmarks::ragtruth` when importing benchmark implementations.
The files directly under `apps/backend/src` assemble the backend and provide
shared infrastructure:

- `main.rs` reads configuration, opens storage, and starts the HTTP server.
- `lib.rs` declares the benchmark contracts, shared snapshot/run envelopes, and
  the `BENCHMARKS` registry. Dispatch selects registered implementations.
- `config.rs` reads environment settings and YAML, validates common metadata,
  resolves catalog entries, and delegates benchmark-specific validation.
- `server.rs` assembles HTTP routes. Its inline `persistence` and `generation`
  modules provide atomic file writes, recovery, Nebula transport, provenance checks,
  bounded request batches, and failure logging. Benchmark modules supply the
  selection/scoring/export policy; shared mechanics do not branch on benchmark names.
- `suite.rs` plans durable suites, pins one snapshot per canonical module, and
  executes supported retrieval/generation modes serially through module contracts.
  The same server run slot covers the entire suite.

Application initialization is handled by `main.rs`. The bundled benchmark YAML
must exist at startup. A named module's `initialize()` prepares a dataset snapshot
when requested; startup does not create snapshots.

To add a benchmark, create `apps/backend/src/benchmarks/<name>.rs`, declare it in
`benchmarks/mod.rs`, and implement `BenchmarkModule`, including validation,
snapshot preparation, and its supported run handlers.
For the shared Nebula generation executor, implement `AnswerEvaluation` to
select candidates, capture reference metadata, score cases, aggregate results,
and declare CSV columns/values. Register the instance in `lib::BENCHMARKS` and
add its YAML definition with matching adapter/evaluation identifiers. Unsupported
run modes reject requests by default; YAML alone does not implement a benchmark.

`MetricValues` is a map from metric names to numeric values, so evaluators can
declare different result metrics without extending a shared fixed score struct.
Metric meaning and denominators remain evaluator-owned. The main dashboard
builds its columns from the result table's `metric.*` and `review.*` fields.
Detailed evidence/review interfaces still need appropriate support when a new
evaluator introduces a different case or review format.

This reorganization preserves the existing HTTP API, YAML definitions, saved
snapshot/run formats, and snapshot fingerprint calculation. `Case` deliberately
retains its existing serialized fields; it is not a universal dataset schema.
Benchmarks requiring new data shapes may need versioned snapshot extensions.

## Datasets

The included `ragtruth-qa` catalog entry uses
[wandb/RAGTruth-processed](https://huggingface.co/datasets/wandb/RAGTruth-processed).
Its source template selects `data/test-*.parquet` (or the train split).
The adapter filters `task_type == "QA"` and snapshots the first 100 unique
question/context pairs by default. Repeated model outputs do not multiply the
retrieval trials. `limit` controls unique cases, from 1 to 10000. A local Parquet
file or a direct `hf://`/HTTPS Parquet path/glob with the same schema also works.
Hugging Face credentials, when needed, come from Polars' `HF_TOKEN` environment
variable, never the load request or catalog.

The `hotpotqa` entry uses the authors' **development distractor v1** dataset from
[HotpotQA](https://hotpotqa.github.io/). It selects the first 100 questions in file
order by default. Each question retains all its supplied candidate passages
(normally ten; the published file includes some shorter candidate lists), its
reference answer, and supporting-fact annotations. The exported Markdown corpus
contains only passage titles and sentences. Questions, answer labels, and support
annotations are never added to that corpus. Identical title/sentence passages are
shared across questions; each question still searches only its own candidates.

The catalog pins a [RAGLAB mirror revision](https://huggingface.co/datasets/RAGLAB/data/tree/c33b09b7fa5099b89830e84f9bdff78329268d9b/eval_datasets/HotPotQA)
because the [authors' download](https://curtis.ml.cmu.edu/datasets/hotpot/hotpot_dev_distractor_v1.json)
can be unavailable. Its SHA-256 is byte-identical to the original download checksum
recorded in the [published HotpotQA dataset metadata](https://huggingface.co/datasets/hotpotqa/hotpot_qa/blob/cb440e4e72efe5808fe5df497ffa0b4e8da85c32/dataset_infos.json).
The loader verifies this checksum, bounds JSON downloads to 64 MiB, and accepts
local JSON paths or public HTTP(S) URLs. To use a deliberately modified fixture,
update or remove `source_sha256` in its separate catalog entry. Original support
annotations are retained without repair, including any unresolved sentence indices;
supporting-fact and joint scores are not implemented. HotpotQA is distributed under
CC BY-SA 4.0.

## Benchmark catalog

Edit [`apps/backend/benchmarks.yaml`](apps/backend/benchmarks.yaml):

```yaml
benchmarks:
  ragtruth-qa:
    name: RAGTruth QA
    source: 'hf://datasets/wandb/RAGTruth-processed/data/{split}-*.parquet'
    split: test
    adapter: ragtruth_qa
    evaluation: paired_context_recovery_v1
    defaults:
      limit: 100
      top_k: 8
  hotpotqa:
    name: HotpotQA (distractor dev)
    source: 'https://huggingface.co/datasets/RAGLAB/data/resolve/c33b09b7fa5099b89830e84f9bdff78329268d9b/eval_datasets/HotPotQA/hotpot_dev_distractor_v1.json'
    source_sha256: '4e9ecb5c8d3b719f624d66b60f8d56bf227f03914f5f0753d6fa1b359d7104ea'
    split: dev
    adapter: hotpotqa_distractor
    evaluation: hotpotqa_answer_v1
    defaults:
      limit: 100
      top_k: 8
```

The server reads this catalog at startup. `BENCHMARK_CATALOG` selects another
file; otherwise it reads `benchmarks.yaml` in the working directory. Restart to
apply edits. Missing/invalid catalogs fail startup, including duplicate names,
unknown fields/adapters/evaluators, unsupported splits, and out-of-range defaults.
Relative local source paths are resolved against the catalog's directory;
`{split}` is replaced with the configured split value (`train`/`test` for RAGTruth,
`dev` for HotpotQA).

Add named entries for other splits, pinned revisions, local fixtures, or default
settings. Supported adapter/evaluator pairs are `ragtruth_qa` /
`paired_context_recovery_v1` for retrieval and `hotpotqa_distractor` /
`hotpotqa_answer_v1` for generated-answer exact match and token F1.
A dataset with another schema needs an adapter in code.
Credentials, server addresses, and storage paths remain environment settings.

Catalog-only suites use `adapter: external_suite` and
`evaluation: external_evaluation`. They require a `preparation` description and
an HTTP(S) publisher `source`; optional `description` and `homepage` fields are
also displayed. Loading one returns a clear error before downloading anything or
creating a snapshot. Its defaults are intended preparation settings, not evidence
that an evaluator is implemented. A working integration must replace this pair
with a supported loader/evaluator and preserve the dataset's scoring semantics.

The LongMemEval entry selects the authors' **S-cleaned** variant. TempRAGEval
requires publisher-approved dataset access and its associated Wikipedia corpus.
AbstentionBench needs abstention-aware evaluation; refusals are not automatically
failures. RAGBench's historical response annotations must not be copied as scores
for newly generated answers. Source references, variants, schemas and remaining
integration work are recorded in
[`docs/research/benchmark-catalog-expansion.md`](docs/research/benchmark-catalog-expansion.md).

Loading saves the selected entry and effective `limit` into the immutable
snapshot's `configuration`. A run without `top_k` uses that snapshot's saved
default, not the current catalog. An explicit run `top_k` overrides it. Older
snapshots without a configuration remain readable and use their previous default
of 8. The content fingerprint still describes the cases/contexts and metric;
run settings record the effective `top_k` separately.

## Interpreting scores

RAGTruth annotates hallucinations in **historical model outputs**, not retrieval
relevance. This initial metric is explicitly named `paired_context_recovery_v1`:

- Export each distinct supplied context as a Markdown document, identified by its
  SHA-256. A context may itself contain several passages; this version does not
  split those into separately judged documents.
- Search **all exported contexts** for each question. The paired context is the
  single positive document. This is a proxy, not an exhaustive relevance judgment.
- Record context hit@k, reciprocal rank@k and binary NDCG@k. The rank is the
  original chunk-result rank. Duplicate chunks do not earn additional credit or
  promote later ranks. With one positive, hit@k is also recall@k.
- Keep original outputs, model names, quality labels and raw hallucination spans
  in the benchmark snapshot for later evaluation. They are never indexed, used as
  gold answers, or applied as labels to new outputs.

Changing `limit` changes the candidate corpus and therefore the difficulty.
Compare runs on the same benchmark fingerprint, top_k and candidate source set.
Single-document runs are useful smoke tests, not retrieval-quality evidence.
Failed queries have blank metric cells; means cover successful queries only.
Only `completed` runs should be compared as full benchmark results.

## Generated answers and evaluation

Answer runs use the same immutable benchmark snapshots and candidate corpus.
`GET /api/benchmarks/v1/answer-runtime` reports the embedding model/revision and
generation profiles actually exposed by the configured Nebula backend. An
unavailable runtime includes a reason; the dashboard does not invent model
choices. The current Nebula implementation uses multilingual-e5-small and exposes
its one process-configured generation profile. Architecture labels describe the
experiment; the saved scope and pipeline watermark preserve the runtime identity.

To enable generation, explicitly configure the separate Nebula process with
`NEBULA_REMOTE_REASONING=1`, a backend-only `MOONSHOT_API_KEY`, and optionally
`MOONSHOT_MODEL` (`kimi-k3` or `kimi-k2.6`). These are Nebula settings, not benchmark
request fields. This makes real remote model calls. Keep credentials out of the
dashboard and request bodies. The benchmark server still uses its existing
`NEBULA_API_BASE` and `NEBULA_API_TOKEN` connection settings. Generation cannot run
against a local-only Nebula session.

Start a run with a profile ID returned by `/answer-runtime`:

```sh
curl -sS http://127.0.0.1:4319/api/benchmarks/v1/answer-runs \
  -H 'Content-Type: application/json' \
  -d '{"benchmark_id":"UUID-FROM-LOAD","architecture_label":"Nebula baseline","profile_id":"moonshot-kimi-k3","description":"Kimi K3 with the original local embedding configuration"}'
```

The runner creates a fresh conversation for every question, calls Nebula's
strict `/query` endpoint, and retains the answer, evidence, claim lineage, model
receipt, outcome and latency. Nebula currently fixes this endpoint's retrieval
top-k to 8. The benchmark verifies source revisions, scope, watermark and model
receipt throughout the run. Retrieval and answer generation share one active-run
slot. Questions run in batches of at most four. Every request in the current
batch must finish before the next batch starts. Each response is persisted as it
arrives, with cases retained in benchmark order. Request failures are recorded
per question and the remaining batches continue. A run that attempts all cases
with some failures finishes with `status: failed`; the dashboard labels it
“Failed,” with processed and failed counts; its detailed inspector also labels
fully attempted failed runs “finished with errors.” Runtime/provenance changes and storage failures stop
scheduling new batches. Refusals and `evidence-only` responses are retained as
separate outcomes. Saved `max_in_flight` records the batch limit; older runs
default to one.

Each failed case records `failure` metadata: timestamp, request stage, HTTP
status, stable error code and a safe message. Optional `provider_diagnostic`
records Moonshot's upstream status/category, recognized error codes, request byte
count, output-token limit, attempt and elapsed time. Provider free-text errors,
prompts and credentials are excluded. The backend syncs each failure to
`answer-runs/<run-id>/failures.jsonl`; the run JSON and CSV also retain the
diagnostics. Open **Inspect → Answers & review → Download failure log** for a JSONL export
of recorded case failures. Older runs remain readable but cannot recover
provider details that were never saved. Nebula additionally emits structured
provider failure events to its backend log.

HotpotQA runs use **`hotpotqa_answer_v1`**: exact match and token F1 with the
[official normalization](https://github.com/hotpotqa/hotpot/blob/master/hotpot_evaluate_v1.py).
Each question searches only its supplied candidate passages. Reference answers
remain in benchmark storage and never enter a Nebula request. Scoring uses the
entire returned answer, with no gold-guided answer extraction or judge-model call.
Nebula's strict mode currently quotes evidence lines, so even a relevant long quote
can score low against a short reference. These are development subset scores,
not official leaderboard results or general hallucination scores.

`automatic_scores` holds `exact_match` and `f1` fractions; `scored` counts attempted
cases. The denominator is always the snapshot's total, including zero for refused,
failed and unprocessed cases. Partial runs remain labelled and cannot be compared
as completed runs. CSV includes the reference answer and per-case EM/F1, plus
run status, total/scored counts and aggregate EM/F1 so partial exports retain
their scoring denominator. Completed HotpotQA runs can be compared without
human reviews.

RAGTruth answer quality uses **`manual_review_v1`**. A human reviews a successfully
generated answer after the run stops and records four independent judgments:
correctness, groundedness, hallucination present, and citation accuracy, plus a
reviewer name and optional notes. Scores remain null until reviewed; unanswered
questions and unreviewed answers never become fabricated zero scores. Means are
fractions over reviewed answers only. Always show reviewed/answered/total
coverage alongside means, and compare like-for-like benchmark fingerprints,
candidate sets, top-k and evaluation versions. Human selection of cases can
introduce review bias, especially when review coverage differs between runs.
These same optional human reviews are available on HotpotQA answers, separately
from its automatic scores.

`POST /answer-runs/{run_id}/cases/{case_id}/review` accepts:

```json
{
  "reviewer": "Researcher",
  "correctness": true,
  "groundedness": true,
  "hallucination": false,
  "citation_accuracy": true,
  "notes": "The answer is supported by the cited passage."
}
```

The latest review replaces the previous review for that case and recomputes the
means atomically. Reviewer names are trimmed and limited to 128 UTF-8 bytes;
notes are limited to 4000 bytes. Running cases and non-answer outcomes cannot be
reviewed. RAGTruth's historical outputs and hallucination labels are **not** gold
answers or labels for these new generations. Nebula's deterministic citation
checks are not presented as automatic answer-quality evaluation.

## Start

The benchmark API requires no authentication and only accepts loopback bind
addresses. `NEBULA_API_TOKEN` authenticates the runner's requests to Nebula and
stays in backend configuration.

From the Genesis repository root, after initializing the submodule and installing
workspace dependencies:

```sh
pnpm benchmark:ui       # Validate/build the backend, start it, and open the dashboard
pnpm benchmark:build    # Validate/build the Rust backend without starting a server
pnpm benchmark:backend  # Validate/build and run the backend without the dashboard
```

`pnpm start` also starts the benchmark backend through Genesis's generic
`genesisDevelopment.backend` hook. This hook only runs for the internal development
server; release builds do not load the module's backend script. The standalone UI
uses the same module-owned lifecycle implementation in `scripts/backend.mjs`.

The script checks `apps/backend/target/debug/backend` (`backend.exe` on Windows)
for executable identity/version, a matching source/build-environment fingerprint,
and a matching binary digest. A missing, stale, damaged, or unvalidated binary
triggers `cargo build --locked --bin backend` with the module backend's manifest
and target directory. The first build requires Rust 1.95 or newer, may download
Cargo dependencies, and can take several minutes. Later starts reuse the validated
build. Compilation or readiness failures are reported in the terminal.

Managed UI startup uses `http://127.0.0.1:4319`. It reuses an existing healthy
benchmark API; otherwise it starts a child process and waits for health/catalog
responses. Closing the development server stops a child it started, while an
already-running API remains externally owned. To manage the backend yourself,
set `BENCHMARK_API_TARGET` to its loopback origin before starting the UI—even when
using the default port. An explicit target disables automatic build/start/stop.
For example:

```sh
export BENCHMARK_API_TARGET='http://127.0.0.1:4320'
pnpm benchmark:ui
```

`pnpm benchmark:backend` uses `BENCHMARK_ADDR` (default `127.0.0.1:4319`). Managed
processes run with `apps/backend` as their working directory, so the defaults are
`apps/backend/benchmark-data` for data and `apps/backend/benchmarks.yaml` for the
catalog. Set an absolute `BENCHMARK_DATA_DIR` to isolate an experiment. The process
inherits backend-only Nebula connection settings. Restart the managed server
after changing its configuration or backend sources.

### Automatic Nebula startup on the benchmarking branch

When no `NEBULA_API_BASE` / `NEBULA_API_TOKEN` pair is supplied, the launcher looks
for the sibling `Modules/native/Nebula` checkout and uses its public
`@genesis/nebula/benchmarking` entry point. Genesis's `benchmarking` branch pins a
Nebula revision providing that entry point. For a standalone checkout, set
`BENCHMARK_NEBULA_ROOT` to the absolute path of Nebula's `benchmarking` checkout.
Without any Nebula checkout, the standalone dashboard still supports browsing
and dataset preparation. A checkout lacking the entry point reports an update error.

Nebula builds its Go executable when absent, damaged, or changed since the last
build, then loads the shared Kimi settings from its own private repository.
Go must be installed; the first build can download dependencies. The launcher
waits for authenticated loopback health and passes the generated local connection
token directly to the Rust child. The Kimi key is neither copied into this
repository nor supplied to the dashboard. Closing the development server stops
both owned processes. Reusing an existing benchmark server leaves that server's
Nebula connection under its existing owner's control.

| Setting | Default | Purpose |
| --- | --- | --- |
| `BENCHMARK_NEBULA_CORPUS` | `<BENCHMARK_DATA_DIR>/nebula/corpus` | Dedicated Markdown corpus for benchmark passages |
| `BENCHMARK_NEBULA_STORAGE` | `<BENCHMARK_DATA_DIR>/nebula/storage` | Nebula's separate mutable index and state |
| `BENCHMARK_NEBULA_MODEL_DIR` | `~/.genesis/storage/.genesis/modules/nebula/models/intfloat-multilingual-e5-small` | Existing E5 model bundle, reused without changing Genesis's index |

The corpus and state directories are created if absent; the corpus is initially
empty. Snapshots are still prepared explicitly through the API. Copy their
exported Markdown passages into the configured corpus, preserving filenames and
bytes, then restart the dashboard development server so Nebula indexes them.
Restart after each corpus change and wait for indexing to finish before running benchmarks. Never
point Nebula at the whole benchmark data directory: snapshot JSON includes
answers and evaluation labels. Models are not downloaded by the launcher. Install
the embedding bundle first, or set `BENCHMARK_NEBULA_MODEL_DIR` to an existing
compatible bundle. No paid generation request is made merely by opening the dashboard.

You can also run directly in `apps/backend` using Rust 1.95 or newer:

```sh
export BENCHMARK_DATA_DIR='/absolute/path/to/experiment-data'
export BENCHMARK_ADDR='127.0.0.1:4319' # optional; this is the default
# Optional: export BENCHMARK_CATALOG='/absolute/path/to/benchmarks.yaml'
cargo run --locked
```

The server prints `BENCHMARK_BACKEND_PORT=<port>`; port 0 selects an available
port.
There is no permissive browser CORS configuration; the internal dashboard uses a
server-side development proxy. An explicit `BENCHMARK_API_TARGET` connects the
UI to a separately managed backend. Only one server may own a data directory.

Load a named benchmark using its YAML defaults:

```sh
curl -sS http://127.0.0.1:4319/api/benchmarks/v1/benchmarks \
  -H 'Content-Type: application/json' \
  -d '{"benchmark":"ragtruth-qa"}'
```

Use `{"benchmark":"ragtruth-qa","limit":50}` to override the case limit for
one snapshot. Source, split and adapter now come from the catalog; the previous
source-based load request is replaced by this named request.

The response contains the benchmark `id`, `fingerprint`, counts, and absolute
`corpus_path`. Loading is synchronous and runs on a blocking worker so health
and result reads remain responsive. A concurrent load receives HTTP 409.

Start a standalone Nebula backend with `-corpus <corpus_path>` and its own
`-module-storage` directory containing the embedding model bundle. This uses
Nebula's existing launch contract; the model is not selected by this server.
Wait until Nebula reports knowledge ready. Keep its files unchanged during a run.
To use both benchmarks, load `hotpotqa` with the same `/benchmarks` POST endpoint
and copy the exported Markdown passages from both snapshots into a dedicated
combined corpus. Preserve filenames and bytes, keep the snapshots immutable, and
point Nebula at the combined directory. Each runner selects only its benchmark's
sources; HotpotQA further selects each question's supplied candidates.
Then restart this benchmark server with the following environment (the command
below runs in `apps/backend`; use `pnpm benchmark:backend` from the repository
root for the managed equivalent):

```sh
export NEBULA_API_BASE='http://127.0.0.1:NEBULA_PORT/api/nebula/v1'
export NEBULA_API_TOKEN='the-token-used-to-start-nebula'
cargo run --locked
```

Previously loaded benchmarks survive restart. Both Nebula settings are optional
for loading/browsing, but both are required for running. Retrieval-only RAGTruth
runs call `/retrieve` and do not need remote generation. HotpotQA is available
through `/answer-runs`; `/runs` rejects answer-only benchmarks.

```sh
curl -sS http://127.0.0.1:4319/api/benchmarks/v1/runs \
  -H 'Content-Type: application/json' \
  -d '{"benchmark_id":"UUID-FROM-LOAD","label":"baseline","description":"Original indexing configuration"}'
```

Both retrieval and answer-run requests accept an optional `description` of at
most 4000 UTF-8 bytes, including multiline text. It defaults to an empty string
and records the experiment notes in the run reference registry. Retrieval
`label` and generation `architecture_label` become the result table
`architecture_name`. Omitted `label` or `architecture_label` defaults to
`baseline`; supplying a label names a later experiment. Existing saved labels
are preserved.

This returns HTTP 202 with a run ID. Poll its status, then download `scores.csv`.
Only one run is active per server, keeping latency measurements free from
competing benchmark runs. Every context must be indexed under its exported file
name and exact content revision. The runner selects all benchmark documents,
pins the workspace/conversation/index identities, and fails if they change or
Nebula returns evidence outside that selection. Extra non-benchmark files in the
Nebula workspace are not selected.
The loader permits up to 10000 cases, but the current Nebula protocol limits a
source-selection request to 64 KiB (roughly 880 distinct documents). Oversized
selections fail explicitly before querying; use a smaller load `limit` until
Nebula supports larger selections. HotpotQA validates each question's smaller
candidate selection separately.

## Run every benchmark

The dashboard's **Run all benchmarks** action sends one request to
`POST /api/benchmarks/v1/suite-runs`. The selected sidebar benchmark only affects
which results you view; it does not restrict the suite. For example:

```sh
curl -sS http://127.0.0.1:4319/api/benchmarks/v1/suite-runs \
  -H 'Content-Type: application/json' \
  -d '{"architecture_label":"Dense retrieval v2","description":"New chunking; same corpus","profile_id":"PROFILE-FROM-ANSWER-RUNTIME"}'
```

`architecture_label` defaults to `baseline` when omitted. An explicit value must
contain 1–256 UTF-8 bytes without surrounding whitespace. Optional `description`
defaults to an empty string and allows at most 4000 UTF-8 bytes. Omit `profile_id` to skip generation; otherwise use
an enabled profile reported by `/answer-runtime`. Optional `top_k` accepts 1–100
and overrides retrieval settings only. Without it, retrieval uses the pinned
snapshot's saved default. Generation retains its evaluator's fixed settings.
The dashboard exposes the architecture label, notes, and runtime profile; top-k
can be supplied through the API.

Planning chooses the most recently saved snapshot per canonical benchmark module
(save time, then UUID for ties) and pins its ID before accepting the suite. YAML
aliases for one module do not create duplicate executions. Prepared snapshots
remain eligible even if their catalog entry was removed. RAGTruth runs retrieval
and, when a profile is supplied, generation; HotpotQA runs generation. Unsupported
modules and missing snapshots are recorded as `skipped`, as is generation without
a profile or with an unavailable runtime. A setup or worker failure is recorded
for its item while later items are still attempted, unless persistence itself
fails. The server holds its one active-run slot across the whole suite; competing
suite or individual-run requests receive HTTP 409.

HTTP 202 returns a `SuiteRun` with `id`, global `run_number`, the accepted
`request`, `status`, `started_at_ms`, nullable `finished_at_ms`/`error`, and `items`.
Each item contains its canonical `benchmark` key, nullable `benchmark_id`,
`mode` (`retrieval`, `generation`, or null), `status`, nullable child `run_id`, and
nullable `reason`. Item states are `queued`, `running`, `completed`, `failed`,
`interrupted`, and `skipped`. Poll `/suite-runs/{id}` for one suite or `/suite-runs`
for history. A suite fails if an item fails/is interrupted or if every item was
skipped; a completed suite can still contain skipped items, so inspect its coverage.

Each created execution keeps its own globally unique `run_number` and UUID.
Comparison rows also include `suite_id` and `suite_run_number`; the dashboard
shows the shared suite number prominently and the execution number underneath.
Standalone `/runs` and `/answer-runs` requests remain supported and have no suite
association. Skipped or pre-execution failures may have no child row; their outcome
and reason remain in suite history.

## HTTP interface

All paths have prefix `/api/benchmarks/v1`.

| Method | Path                                       | Result                                                          |
| ------ | ------------------------------------------ | --------------------------------------------------------------- |
| GET    | `/health`                                  | Server liveness                                                 |
| GET    | `/catalog`                                 | Configured benchmark names, definitions and defaults            |
| POST   | `/benchmarks`                              | Load/snapshot dataset and export corpus; HTTP 201               |
| GET    | `/benchmarks`                              | Dataset summaries and corpus paths                              |
| GET    | `/benchmarks/{id}`                         | Full saved cases, contexts, and historical annotations          |
| GET    | `/results`                                 | Per-benchmark comparison tables, columns and string-valued rows |
| GET    | `/results/{benchmark}/scores.csv`          | Download a canonical module's comparison table in both modes    |
| POST   | `/suite-runs`                              | Start one suite across prepared benchmark modules; HTTP 202     |
| GET    | `/suite-runs`                              | Saved suite history, including skipped and failed items         |
| GET    | `/suite-runs/{id}`                         | One suite's request, status and execution associations          |
| POST   | `/runs`                                    | Start retrieval run; HTTP 202                                   |
| GET    | `/runs`                                    | Saved run summaries, including progress                         |
| GET    | `/runs/{id}`                               | Run status, settings, source IDs, watermark, means, error       |
| GET    | `/runs/{id}/scores.csv`                    | Download completed/partial CSV after run stops                  |
| GET    | `/answer-runtime`                          | Actual embedding identity and generation profile availability   |
| POST   | `/answer-runs`                             | Start generated-answer run; HTTP 202                            |
| GET    | `/answer-runs`                             | Saved answer-run summaries and review coverage                  |
| GET    | `/answer-runs/{id}`                        | Summary plus per-case answers, evidence, outcomes and reviews   |
| POST   | `/answer-runs/{id}/cases/{case_id}/review` | Save human judgments after run stops                            |
| GET    | `/answer-runs/{id}/scores.csv`             | Download answers and current human reviews after run stops      |

Run states are `running`, `completed`, `failed`, and `interrupted`. Startup marks
unfinished runs interrupted rather than silently resuming against a new index.
Network/protocol failures stop the run and retain earlier results. A run that
fails during setup has a summary but may have no per-question CSV. Requests use
snake_case;
Nebula protocol objects preserved in summaries retain their original camelCase.

## Result storage

All artifacts live under `BENCHMARK_DATA_DIR`. Benchmark comparison tables have
one row per created execution, updated as progress or human reviews are saved.
Repeating a suite creates new execution rows associated with a new suite
number. Outcomes without a child execution remain in the suite registry.

```text
experiment-data/
  .server.lock
  results/
    ragtruth-qa.csv
    hotpotqa.csv
    runs.json
  benchmarks/<uuid>/
    benchmark.json
    corpus/ragtruth-<sha256>.md
  runs/<uuid>/
    run.json
    scores.csv
  answer-runs/<uuid>/
    run.json
    failures.jsonl          # Created when request failures occur
```

The CSV filename uses the registered benchmark module key, such as
`ragtruth-qa.csv` or `hotpotqa.csv`. A benchmark's retrieval and generation runs
share its table; the `mode` and `evaluation` columns identify how each row was
measured. Columns include `run_number`, `run_id`, `architecture_name`, status,
snapshot ID/fingerprint, top-k, completion counts, timestamps, and available
model identities, description, and optional suite ID/number.
`GET /results` returns `{ "benchmarks": [{ "benchmark": "module-key", "columns": [],
"rows": [] }] }`; each row maps column names to strings, with empty strings for
unavailable values. Retrieval and automatic answer scores use `metric.<name>`
columns; human review scores use `review.<name>`. Each table contains the union
of metrics recorded by its runs. Unavailable metrics stay blank rather than
becoming zero. Check evaluation, snapshot fingerprint and settings before
comparing scores; failed or partial runs remain visible.

`results/runs.json` assigns run numbers starting at 1 from one sequence shared by
suites and individual executions across all benchmarks and both run modes. Its
`runs` map holds execution references, and `suite_runs` holds full suite records
including the pinned plan and child associations. A registry containing only a
legacy standalone execution can look like this (an empty `suite_runs` field may
be omitted):

```json
{
  "next_run_number": 2,
  "runs": {
    "1": {
      "run_id": "4d293026-ea7a-44bf-9446-cd2ec804df37",
      "benchmark": "ragtruth-qa",
      "mode": "retrieval",
      "architecture_name": "baseline",
      "description": "Original indexing configuration"
    }
  },
  "suite_runs": {}
}
```

A subsequent suite reserves number 2 in `suite_runs`; its first child reserves
number 3 in `runs`. That child's comparison row has `run_number: "3"` and
`suite_run_number: "2"`. Further children receive their own numbers while sharing
suite number 2. Older registries without `suite_runs` remain readable.

The registry owns run numbers and descriptions. To amend notes, stop the backend
and edit `runs[execution_number].description` or
`suite_runs[suite_number].request.description` in `results/runs.json`. These are
separate notes; edits are preserved when the backend restarts. Keep the registry with the detailed run artifacts: deleting
it loses the assigned numbering and edited descriptions. The comparison CSVs
are derived files and are rebuilt from saved runs and the registry at startup.
On the first startup with older runs, unregistered runs receive numbers in start
time order, with UUID as the tie-breaker. Existing registered numbers are retained.
Unfinished runs become `interrupted`, and the rebuilt tables reflect that status.
Running suites also become `interrupted`: completed children are retained, active
children are reconciled with their saved runs, and queued items that never started
are marked interrupted. Startup never automatically reruns a suite or child.

The per-run files retain detailed evidence. Retrieval `scores.csv` includes
run/case/source identifiers, query, top-k, status, latency, individual metrics,
retrieved evidence as a JSON cell, and error. It is flushed/synced after each
question; a hard kill during a row write can leave its final row incomplete.
Answer runs persist their summary and finished cases in `run.json`. Their
per-case CSV is generated on download from saved answers and current reviews;
pending review cells stay blank. Concurrent review updates are serialized to
preserve other cases' reviews.

JSON files and benchmark comparison CSVs are each replaced atomically. Updates
across the detailed run, registry, and comparison CSV are not one transaction;
startup rebuilds comparison tables from the saved artifacts after an interruption.
There is no database dependency or first-run initialization wizard.

## Verification

```sh
cargo fmt --check
cargo check --locked --tests
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests use real local Parquet and HTTP with a scripted Nebula peer, checking
deduplication, CSV quoting, source selection, known ranks, index drift, failures,
token-free startup, loopback binding, persistence, interrupted-run recovery, suite
planning/execution, and shared numbering. Frontend and build-script checks are
documented in [`apps/frontend/README.md`](apps/frontend/README.md#verify). The optional live
Hugging Face smoke test requires network access:

```sh
cargo test --locked live_hugging_face_ragtruth -- --ignored --nocapture
```

The separate real-Nebula acceptance test launches both backend executables and
creates a three-question Parquet fixture locally. It uses the current Nebula
retrieval implementation and a real installed embedding bundle, without dataset
downloads, remote reasoning, or a scripted retrieval peer. Build Nebula for the
current platform first, following its backend README, then run from
`apps/backend`:

```sh
NEBULA_E2E_BINARY='/absolute/path/to/nebula-backend' \
NEBULA_E2E_MODEL_DIR='/absolute/path/to/models/intfloat-multilingual-e5-small' \
NEBULA_E2E_ARTIFACT_DIR='/tmp/nebula-benchmark-acceptance' \
cargo test --locked --test nebula_e2e -- --ignored --nocapture
```

`NEBULA_E2E_MODEL_DIR` contains the complete bundle, including the native ONNX
runtime for the current platform. The test copies it to a fresh private storage
root and starts both servers on ephemeral loopback ports. It loads the Parquet
through the Rust HTTP API, waits for actual indexing, and requires a completed run
with all three paired contexts recovered. It also verifies source revisions,
scope/index identity, evidence excerpts against the original documents, and the
downloaded CSV. This small acceptance fixture proves integration, not retrieval
quality on a representative dataset.

The optional artifact directory retains a uniquely named run folder, including
`benchmark.json`, `workspace.json`, `run.json`, `scores.csv`, process logs, the
original Parquet fixture, and both servers' isolated state. It is printed before
startup so failures can be inspected too. Retaining the copied model requires
several hundred MB per run. Omit `NEBULA_E2E_ARTIFACT_DIR` to clean up automatically.
The test always stops its own child processes and never uses the normal Genesis
storage directory. Ordinary `cargo test` leaves this acceptance test ignored.

Polars' [Hugging Face documentation](https://docs.pola.rs/user-guide/io/hugging-face/)
describes `hf://datasets/owner/repo@revision/path` syntax. Use a pinned revision
when reproducibility across future dataset updates matters; loaded snapshots
and their content fingerprint are retained locally.
