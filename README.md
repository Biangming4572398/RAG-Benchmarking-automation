# RAG benchmarking backend

The experimental architecture comparison dashboard lives in
[`apps/frontend`](apps/frontend/README.md). Open it with `pnpm benchmark:ui` from
the Genesis workspace, or use the Benchmarking dock item in development Genesis.
Its results table comes first, comparing benchmarks against architecture labels.
It connects through a development proxy, starts runs on saved snapshots, polls
progress, compares compatible retrieval results, and downloads CSV files.
Benchmark definitions are managed through Git in
[`apps/backend/benchmarks.yaml`](apps/backend/benchmarks.yaml), starting with
RAGTruth QA. Prepare snapshots with the HTTP API as described below; their saved
names appear in the dashboard automatically. Its UI, API client, and standalone
proxy live inside this submodule. The backend and HTTP interface below remain
unchanged.

Developer-only Rust HTTP server for loading benchmark data with Polars, running
Nebula retrieval, and retaining CSV scores. It does not register with the Genesis
release build, select models, launch Nebula, or modify your normal Genesis storage.
The benchmark backend uses files only; Nebula manages its own index separately.

## First dataset and what is measured

The included `ragtruth-qa` catalog entry uses
[wandb/RAGTruth-processed](https://huggingface.co/datasets/wandb/RAGTruth-processed).
Its source template selects `data/test-*.parquet` (or the train split).
The adapter filters `task_type == "QA"` and snapshots the first 100 unique
question/context pairs by default. Repeated model outputs do not multiply the
retrieval trials. `limit` controls unique cases, from 1 to 10000. A local Parquet
file or a direct `hf://`/HTTPS Parquet path/glob with the same schema also works.
Hugging Face credentials, when needed, come from Polars' `HF_TOKEN` environment
variable, never the load request or catalog.

## Benchmark catalog

Edit [`apps/backend/benchmarks.yaml`](apps/backend/benchmarks.yaml):

```yaml
benchmarks:
  ragtruth-qa:
    name: RAGTruth QA
    source: "hf://datasets/wandb/RAGTruth-processed/data/{split}-*.parquet"
    split: test
    adapter: ragtruth_qa
    evaluation: paired_context_recovery_v1
    defaults:
      limit: 100
      top_k: 8
```

The server reads this catalog at startup. `BENCHMARK_CATALOG` selects another
file; otherwise it reads `benchmarks.yaml` in the working directory. Restart to
apply edits. Missing/invalid catalogs fail startup, including duplicate names,
unknown fields/adapters/evaluators, unsupported splits, and out-of-range defaults.
Relative local source paths are resolved against the catalog's directory;
`{split}` is replaced with the configured `train` or `test` value.

Add named entries for other splits, pinned revisions, local fixtures, or default
settings. `ragtruth_qa` and `paired_context_recovery_v1` are the only implemented
adapter/evaluator pair. A dataset with another schema needs an adapter in code.
Credentials, server addresses, and storage paths remain environment settings.

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

## Start

`BENCHMARK_API_TOKEN` is a local shared password you choose, not a provider API
key or an existing secret you need to recover. The Rust backend required it before
the dashboard was added. Use the same value for the backend and frontend's Node
proxy; it is independent of `NEBULA_API_TOKEN`, which authenticates requests to
Nebula. Keep these credentials outside Git.

Run in `apps/backend` using Rust 1.95 or newer:

```sh
export BENCHMARK_API_TOKEN='a-local-development-token'
export BENCHMARK_DATA_DIR='/absolute/path/to/experiment-data'
export BENCHMARK_ADDR='127.0.0.1:4319' # optional; this is the default
# Optional: export BENCHMARK_CATALOG='/absolute/path/to/benchmarks.yaml'
cargo run --locked
```

The server prints `BENCHMARK_BACKEND_PORT=<port>`; port 0 selects an available
port. All endpoints require `Authorization: Bearer $BENCHMARK_API_TOKEN`.
There is no permissive browser CORS configuration; the internal dashboard uses a
server-side development proxy. Export the same `BENCHMARK_API_TOKEN` in the shell
running `pnpm benchmark:ui` or `pnpm start`; optionally set `BENCHMARK_API_TARGET`
to the backend origin when it differs from `http://127.0.0.1:4319`. Credentials
never enter the renderer. Only one server may own a data directory.

Load a named benchmark using its YAML defaults:

```sh
curl -sS http://127.0.0.1:4319/api/benchmarks/v1/benchmarks \
  -H "Authorization: Bearer $BENCHMARK_API_TOKEN" \
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
Then restart this benchmark server with:

```sh
export NEBULA_API_BASE='http://127.0.0.1:NEBULA_PORT/api/nebula/v1'
export NEBULA_API_TOKEN='the-token-used-to-start-nebula'
cargo run --locked
```

Previously loaded benchmarks survive restart. Both Nebula settings are optional
for loading/browsing, but both are required for running. Remote generation is not
needed: this version calls `/retrieve`, not `/query`.

```sh
curl -sS http://127.0.0.1:4319/api/benchmarks/v1/runs \
  -H "Authorization: Bearer $BENCHMARK_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"benchmark_id":"UUID-FROM-LOAD","label":"baseline"}'
```

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
Nebula supports larger selections.

## HTTP interface

All paths have prefix `/api/benchmarks/v1`.

| Method | Path | Result |
| --- | --- | --- |
| GET | `/health` | Server liveness |
| GET | `/catalog` | Configured benchmark names, definitions and defaults |
| POST | `/benchmarks` | Load/snapshot dataset and export corpus; HTTP 201 |
| GET | `/benchmarks` | Dataset summaries and corpus paths |
| GET | `/benchmarks/{id}` | Full saved cases, contexts, and historical annotations |
| POST | `/runs` | Start retrieval run; HTTP 202 |
| GET | `/runs` | Saved run summaries, including progress |
| GET | `/runs/{id}` | Run status, settings, source IDs, watermark, means, error |
| GET | `/runs/{id}/scores.csv` | Download completed/partial CSV after run stops |

Run states are `running`, `completed`, `failed`, and `interrupted`. Startup marks
unfinished runs interrupted rather than silently resuming against a new index.
Network/protocol failures stop the run and retain earlier results. A run that
fails during setup has a summary but may have no CSV. Requests use snake_case;
Nebula protocol objects preserved in summaries retain their original camelCase.

```text
experiment-data/
  .server.lock
  benchmarks/<uuid>/
    benchmark.json
    corpus/ragtruth-<sha256>.md
  runs/<uuid>/
    run.json
    scores.csv
```

CSV includes run/case/source identifiers, query, top_k, status, latency,
individual metrics, retrieved evidence as a JSON cell, and error. JSON metadata
is replaced atomically; CSV is flushed/synced after each question. A hard kill
during a row write can leave the final row incomplete; interrupted results are
partial artifacts. There is no database dependency or schema migration.

## Verification

```sh
cargo fmt --check
cargo check --locked --tests
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests use real local Parquet and HTTP with a scripted Nebula peer, checking
deduplication, CSV quoting, source selection, known ranks, index drift, failures,
authentication, persistence, and interrupted-run recovery. The optional live
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
