# RAG benchmarking dashboard

The dashboard reads the Rust HTTP API in `apps/backend`. UI, API contracts,
styling, the backend lifecycle script, and the Genesis SDK adapter belong to this
submodule. Genesis discovers the module, displays it, and supplies generic
internal-development proxy and backend hooks.

A macOS-style sidebar switches between benchmarks. Each benchmark has its own
results table, with **Retrieval** and **Generated answers** tabs, stable run
numbers, architecture labels, and evaluator-specific columns. System typography,
grouped surfaces, keyboard focus states, responsive navigation, and system dark
mode support both standalone use and the Genesis panel. Definitions live in
Git-managed [`apps/backend/benchmarks.yaml`](../backend/benchmarks.yaml); the
frontend does not create or edit them.

## Connect and open

From the Genesis repository root:

```sh
git submodule update --init genesis/Modules/dev/RAG-Banchmarks genesis/Modules/native/Nebula
pnpm install
pnpm benchmark:ui
```

The development server validates the local Rust binary, compiles it when missing,
stale, damaged, or not previously validated, and starts the API on loopback port 4319. The first build requires Rust 1.95 or newer and can take several minutes.
Build output and startup errors appear in the terminal. A healthy existing API is
reused. On Genesis's `benchmarking` branch, it also builds and starts Nebula with
the private module's shared Kimi settings. Go and the local embedding bundle are
required for this automatic Nebula path. Closing the development server stops
the backend processes it started.

`pnpm start` supplies the same automatic backend lifecycle through Genesis's
`genesisDevelopment.backend` hook; select **Benchmarking** in the dock. A
customized dock layout takes precedence over generated defaults. Other root
commands are:

```sh
pnpm benchmark:build    # Validate/build the Rust backend without running it
pnpm benchmark:backend  # Validate/build and run only the backend
```

The backend script checks executable identity/version, a source/build-environment
fingerprint, and the binary digest before reusing
`apps/backend/target/debug/backend`. Managed processes run from `apps/backend`,
using `apps/backend/benchmark-data` and `apps/backend/benchmarks.yaml` by default.
Set `BENCHMARK_DATA_DIR` and `BENCHMARK_CATALOG` to absolute paths when isolating an
experiment. `pnpm benchmark:backend` respects `BENCHMARK_ADDR`; automatic UI startup
uses port 4319. See the [backend startup instructions](../../README.md#start) for
manual Cargo commands and environment details.

To use a separately managed backend, set its loopback origin in the shell running
the frontend. Any nonempty explicit target disables automatic compilation and
backend startup/shutdown, including when it points to the default port:

```sh
export BENCHMARK_API_TARGET='http://127.0.0.1:4319'
pnpm benchmark:ui
```

Restart the frontend/Genesis development server after changing proxy variables,
backend configuration, or Rust sources. The browser calls same-origin
`/api/benchmarks/v1` routes through a Node-side proxy. Backend credentials never
enter renderer state or bundles. The benchmark API itself requires no token;
`NEBULA_API_TOKEN` authenticates its calls to Nebula and must stay in backend
configuration, outside Git and `VITE_` variables.

Automatic startup leaves the snapshot store and dedicated corpus empty. Follow
the [backend instructions](../../README.md#automatic-nebula-startup-on-the-benchmarking-branch)
to select model/corpus paths and prepare snapshots. Copy exported passages into
the configured benchmark corpus. To run both RAGTruth and HotpotQA, index the
combined exported corpus while preserving its filenames and bytes. The YAML
catalog is read at startup and does not automatically load snapshots. Restart the
dashboard development server after copying or changing corpus passages; Nebula
indexes them during startup. Wait for indexing before running benchmarks.

Release packaging excludes development module registrations, frontend assets,
backend resources, and lifecycle hooks. `pnpm --filter @genesis/benchmarking build`
builds the standalone UI without starting or compiling a backend; serving that
static build separately requires an equivalent proxy.

## Workflow

1. Select a benchmark in the sidebar. Its saved results appear by run number;
   switch between Retrieval and Generated answers. Search by run number,
   architecture, model, or notes; filter by status and page through saved rows.
2. Choose **Run all benchmarks**, enter an architecture label and optional run
   notes, and select an enabled generation profile from the configured Nebula
   runtime. The UI waits for the initial runtime availability check before
   allowing submission. Labels allow 1–256 UTF-8 bytes and notes up to 4000 bytes.
3. Start the suite. The selected sidebar benchmark does not limit the run. The
   backend pins the latest prepared snapshot per canonical benchmark module and
   executes every supported mode in sequence. RAGTruth supports retrieval and
   generation; HotpotQA supports generation. Retrieval uses saved snapshot top-k
   defaults; the current generation path uses top-k 8.
4. Watch progress and open **Suite run history** for completed, failed, interrupted,
   or skipped outcomes. Unprepared benchmarks, catalog-only integrations, and
   unavailable generation are explicitly recorded as skipped. Selecting a suite
   in history changes its progress/details disclosure, not the results filter.
5. Use **Inspect** on an execution for compatible-run comparison, detailed
   settings/evidence, per-run CSV, and answer review. **Export benchmark CSV**
   downloads the selected benchmark's full comparison table across both modes,
   including rows outside the current search, status filter, and page.

One suite reserves a global run number; each child execution also receives its
own global number. Table rows show the shared suite number and the child's
**Execution #**. Standalone runs created through the existing HTTP endpoints show
their execution number without a suite association. Comparison columns come from
backend `metric.*` and `review.*` fields; unavailable values stay blank and zero
remains a valid score. Skipped items or failures before child creation have no
execution row, but their reason remains available in suite history.

The server permits one active suite or individual run at a time. A failure in one
benchmark does not suppress later suite items unless persistence fails. On server
restart, unfinished work is marked interrupted; accepted work is never
automatically rerun. Data and numbering are retained in backend files, including
`results/runs.json`. See [result storage](../../README.md#result-storage).

Manage definitions and defaults in YAML through Git. Restart the backend after
edits and prepare new snapshots when those changes should apply. Existing
snapshots preserve their original configuration. **Benchmark setup & snapshots**
shows preparation guidance, saved snapshot counts, IDs, and corpus paths.
LongMemEval, TempRAGEval, QASPER, AbstentionBench, MultiHop-RAG, and RAGBench remain
**Integration required**: their loaders and official evaluators are not yet
implemented. The dashboard does not load datasets, launch Nebula, configure
providers, or change the architecture being measured.

## Generated answers

**Generated answers** shows the architecture label and actual embedding and
generation model identities saved by the backend. Generation profiles come from
Nebula; the UI does not configure credentials or arbitrary models. When generation
is unavailable, suite submission omits a profile and answer evaluations are
skipped; prepared retrieval benchmarks can still execute. Configure remote
generation in the separate Nebula process as described in the backend README.
Generation can incur charges through that configured provider.

The backend isolates each question in a fresh conversation and persists its
answer, evidence, citations, model receipt, and outcome. Requests run in batches
of up to four, with all responses received before the next batch starts. Request
failures are recorded while remaining questions continue; runtime/provenance or
storage failures can stop scheduling. Failed runs remain inspectable and are
excluded from completed-run comparisons.

HotpotQA's **Answer EM** and **Answer F1** are automatic full-answer scores with the
denominator fixed to every snapshot case. Each question selects its supplied
candidate passages (typically ten); gold answers never enter the index or model
request. Refused, failed, and unprocessed cases contribute zero. The v1 evaluator
uses HotpotQA normalization without extracting text using the gold answer.
Nebula currently quotes full evidence lines, so exact match can be low when a
passage contains the right short answer. These are development subset results;
supporting-fact and joint metrics are not implemented.

Open **Inspect → Answers & review** after generation stops. HotpotQA details show
the reference answer and per-case automatic scores. Read the question, generated
answer, evidence, and citation map, then explicitly assess correctness,
groundedness, hallucination presence, and citation accuracy. Supply a reviewer
name and optional notes. A saved review can be replaced. These **human review
v1** judgments remain separate from automatic scores. Correctness, groundedness,
and citation accuracy measure fractions of reviewed answers marked passing;
hallucinations measure the fraction marked present, so lower is better. RAGTruth's
historical annotations are never reused as labels for new generations.

Failed questions show their request stage, HTTP status, and safe provider
diagnostics when available, including request size and output-token limit.
**Download failure log** exports recorded failures as JSONL without prompts or
credentials. Historical failures remain visible; metadata absent from older runs
cannot be reconstructed.

Unreviewed human scores stay blank, with answered/reviewed/total coverage visible.
Completed and fully scored HotpotQA runs can use **Compare setup** without human
review; RAGTruth requires every case answered and reviewed. Comparisons require
the same snapshot fingerprint, top-k, evaluator version, and candidate source set.
Architecture and model pairings may vary. Partial runs remain inspectable,
reviewable where answers exist, and exportable.

## Reading retrieval results

The current evaluator is `paired_context_recovery_v1`. It measures recovery of the
supplied context. Columns show mean context Hit@k (also Recall@k for this single
positive), mean reciprocal rank@k, and binary NDCG@k on a 0–1 scale. Larger is
better. Means cover successful queries only and do not measure general answer
quality or hallucination detection.

Running, failed, and interrupted runs retain available means with partial
coverage indicated. In **Inspect**, only fully completed runs can use **Compare
setup**. Comparison requires the same metric kind, snapshot fingerprint, top-k,
and candidate source ID set. The highest value per metric is highlighted,
including ties. Repeated runs remain separate rows and different metrics are
never averaged.

The workspace refreshes every three seconds without overlapping its polls. The
selected benchmark, evaluation tab, search, and status filter survive Genesis
navigation. Closing the panel aborts pending UI requests and stops polling;
accepted backend suites continue. Returning fetches fresh records. Connection
failures retain previously loaded results with a visible error and disable suite
submission. **Refresh** retries reads. Suite creation, generation, and review
writes are never automatically retried because the server may already have
accepted them.

## Verify

From the Genesis repository root:

```sh
pnpm --filter @genesis/benchmarking typecheck
pnpm --filter @genesis/benchmarking test
pnpm --filter @genesis/benchmarking build
```

Tests cover Rust response shapes, dynamic result columns, suite requests,
benchmark scoping, stable numbering, status/search state, keyboard tabs,
availability handling, partial and incompatible results, human review, CSV,
backend compilation/reuse, and Node-only proxy/lifecycle configuration. The Genesis
host also tests registry/loader/navigation and generic development backend hooks.
These frontend and script tests do not require external model calls or dataset
downloads. Backend suite and persistence checks are documented in the
[backend verification section](../../README.md#verification).
