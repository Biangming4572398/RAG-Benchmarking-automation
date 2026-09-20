# RAG benchmarking backend

The experimental architecture comparison dashboard lives in
[`apps/frontend`](apps/frontend/README.md). Open it with `pnpm benchmark:ui` from
the Genesis workspace, or use the Benchmarking dock item in development Genesis.
Its results table comes first, comparing benchmarks against architecture labels.
It connects through a development proxy, starts runs on saved snapshots, polls
progress, compares compatible retrieval results, and downloads CSV files. Its
Generated answers tab runs the configured Nebula generation profile, scores
HotpotQA answers automatically, and records explicit human reviews of answer quality.
Benchmark definitions are managed through Git in
[`apps/backend/benchmarks.yaml`](apps/backend/benchmarks.yaml), including
RAGTruth QA and HotpotQA. Prepare snapshots with the HTTP API as described below; their saved
names appear in the dashboard automatically. Its UI, API client, and standalone
proxy live inside this submodule.

Developer-only Rust HTTP server for loading benchmark data with Polars, running
Nebula retrieval and answer generation, and retaining results and human reviews.
It does not register with the Genesis release build, install models, launch
Nebula, or modify your normal Genesis storage.
The benchmark backend uses files only; Nebula manages its own index separately.

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
  -d '{"benchmark_id":"UUID-FROM-LOAD","architecture_label":"Nebula baseline","profile_id":"moonshot-kimi-k3"}'
```

The runner creates a fresh conversation for every question, calls Nebula's
strict `/query` endpoint, and retains the answer, evidence, claim lineage, model
receipt, outcome and latency. Nebula currently fixes this endpoint's retrieval
top-k to 8. The benchmark verifies source revisions, scope, watermark and model
receipt throughout the run. Retrieval and answer generation share one active-run
slot. A hard failure stops the run and retains earlier answers. Refusals and
`evidence-only` responses are retained as separate outcomes.

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

Run in `apps/backend` using Rust 1.95 or newer:

```sh
export BENCHMARK_DATA_DIR='/absolute/path/to/experiment-data'
export BENCHMARK_ADDR='127.0.0.1:4319' # optional; this is the default
# Optional: export BENCHMARK_CATALOG='/absolute/path/to/benchmarks.yaml'
cargo run --locked
```

The server prints `BENCHMARK_BACKEND_PORT=<port>`; port 0 selects an available
port.
There is no permissive browser CORS configuration; the internal dashboard uses a
server-side development proxy. When running `pnpm benchmark:ui`, optionally set
`BENCHMARK_API_TARGET` to the backend origin when it differs from
`http://127.0.0.1:4319`. Only one server may own a data directory.

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
Then restart this benchmark server with:

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
Nebula supports larger selections. HotpotQA validates each question's smaller
candidate selection separately.

## HTTP interface

All paths have prefix `/api/benchmarks/v1`.

| Method | Path                                       | Result                                                        |
| ------ | ------------------------------------------ | ------------------------------------------------------------- |
| GET    | `/health`                                  | Server liveness                                               |
| GET    | `/catalog`                                 | Configured benchmark names, definitions and defaults          |
| POST   | `/benchmarks`                              | Load/snapshot dataset and export corpus; HTTP 201             |
| GET    | `/benchmarks`                              | Dataset summaries and corpus paths                            |
| GET    | `/benchmarks/{id}`                         | Full saved cases, contexts, and historical annotations        |
| POST   | `/runs`                                    | Start retrieval run; HTTP 202                                 |
| GET    | `/runs`                                    | Saved run summaries, including progress                       |
| GET    | `/runs/{id}`                               | Run status, settings, source IDs, watermark, means, error     |
| GET    | `/runs/{id}/scores.csv`                    | Download completed/partial CSV after run stops                |
| GET    | `/answer-runtime`                          | Actual embedding identity and generation profile availability |
| POST   | `/answer-runs`                             | Start generated-answer run; HTTP 202                          |
| GET    | `/answer-runs`                             | Saved answer-run summaries and review coverage                |
| GET    | `/answer-runs/{id}`                        | Summary plus per-case answers, evidence, outcomes and reviews |
| POST   | `/answer-runs/{id}/cases/{case_id}/review` | Save human judgments after run stops                          |
| GET    | `/answer-runs/{id}/scores.csv`             | Download answers and current human reviews after run stops    |

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
  answer-runs/<uuid>/
    run.json
```

CSV includes run/case/source identifiers, query, top_k, status, latency,
individual metrics, retrieved evidence as a JSON cell, and error. JSON metadata
is replaced atomically; CSV is flushed/synced after each question. A hard kill
during a row write can leave the final row incomplete; interrupted results are
partial artifacts. There is no database dependency or schema migration.

Answer runs persist their summary and all finished cases in one atomically
replaced `run.json`. Their CSV is generated on download from the current saved
answers/reviews, so it reflects the latest human judgments. Pending review cells
remain blank. Startup also marks unfinished answer runs interrupted; previously
generated answers remain available for review and export. Concurrent review
updates are serialized to preserve other cases' reviews.

## Verification

```sh
cargo fmt --check
cargo check --locked --tests
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests use real local Parquet and HTTP with a scripted Nebula peer, checking
deduplication, CSV quoting, source selection, known ranks, index drift, failures,
token-free startup, loopback binding, persistence, and interrupted-run recovery. The optional live
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
