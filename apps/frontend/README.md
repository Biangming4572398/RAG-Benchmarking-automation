# RAG benchmarking dashboard

The dashboard reads the existing Rust HTTP API in `apps/backend`. All UI, API
contracts, styling, and the Genesis SDK adapter belong to this submodule. Genesis
only discovers the module, displays it, and supplies generic development proxy
support. The results table is the first content in each dashboard tab: Retrieval and Generated answers. Benchmark
definitions live in Git-managed [`apps/backend/benchmarks.yaml`](../backend/benchmarks.yaml),
starting with RAGTruth QA; the frontend does not create or edit them.

## Connect and open

Follow the [backend instructions](../../README.md#start) to start the server,
load the `ragtruth-qa` snapshot using the documented curl command, and connect
Nebula to its exported corpus. The YAML catalog is read at startup; it does not
automatically load snapshots.

The local benchmark server does not require a token. `NEBULA_API_TOKEN` still
connects the benchmark server to Nebula; keep it in backend configuration, outside
Git and `VITE_` variables.

In the shell running the frontend:

```sh
# Optional if the backend uses another origin/port:
export BENCHMARK_API_TARGET='http://127.0.0.1:4319'
```

From the Genesis repository root:

```sh
git submodule update --init genesis/Modules/dev/RAG-Banchmarks
pnpm install
pnpm benchmark:ui
```

Or run `pnpm start` and select **Benchmarking** in the Genesis dock. A customized
dock layout takes precedence over generated defaults. Restart the frontend/Genesis
development server after changing proxy environment variables.

The browser calls same-origin `/api/benchmarks/v1` routes. The Node-side proxy
forwards them to the local backend, using port 4319 by default.
Genesis discovers the submodule's `genesisDevelopment.proxy` declaration. Release
packaging excludes development module registrations, frontend assets, and backend
resources. `pnpm --filter @genesis/benchmarking build` builds the standalone UI;
serving that static build separately requires an equivalent local proxy.

## Workflow

1. Review architecture results in the table at the top. Many saved runs can be
   searched, filtered, and paginated.
2. Select a saved snapshot below the table, add an architecture label, and start a
   run. Snapshot names come from their saved YAML configuration, so newly prepared
   benchmarks appear automatically. Blank top-k
   uses that snapshot's saved default, including 8 for older snapshots; an explicit
   top-k must be 1–100. Labels are limited to 256 UTF-8 bytes by the server.
3. Watch progress in the table. The server permits **one active run at a time**.
4. Open run details for fingerprint, source IDs, scope, index watermark, and errors.
   Download CSV after a run stops. A setup failure may produce no CSV; the server's
   error is displayed without substituting fabricated data.

Manage benchmark definitions and defaults in YAML through Git. Restart the backend
after edits, then prepare a new snapshot using its HTTP API when those changes
should apply. Existing snapshots preserve their original configuration. The
dashboard lists saved snapshots and runs; it does not fetch or manage the catalog,
load datasets, launch Nebula, choose models, or change the architecture.

## Generated answers

The **Generated answers** tab records an architecture label together with the
embedding model ID/revision and generation profile reported by the configured
Nebula runtime. Model choices come from that runtime; the UI does not configure
providers, credentials, or arbitrary models. The current Nebula query path uses
top-k 8. The benchmark backend isolates each question in a fresh conversation and
persists the answer, evidence, citations, model receipt and outcome.

Enable remote generation in your separately configured Nebula process as described
in the backend README, then start a pairing against a prepared snapshot. The tab
shows unavailable runtimes honestly and still displays saved results. Retrieval
and answer generation share the server's single active-run limit. Starting a run
can incur charges through the configured model provider.

Select **Answers & review** after a run stops. Read each question, generated answer,
evidence and citation map, then explicitly assess correctness, groundedness,
hallucination presence and citation accuracy. Supply a reviewer name; notes are
optional. A saved review can be replaced. These are **human review v1** judgements,
not an automatic model-judge evaluation. Correctness, groundedness and citation
accuracy show the fraction of reviewed answers marked as passing; hallucinations
show the fraction marked present, so lower is better.

Unreviewed scores stay blank. Answer coverage and review coverage remain visible;
refusals, evidence-only responses and failures are not assigned fabricated scores.
Only completed runs with every case answered and reviewed can use Compare setup.
Comparisons require the same snapshot fingerprint, top-k, evaluator version and
candidate source set. Architecture and model pairings may vary. Partial runs can
still be inspected, reviewed where answers exist, and exported as CSV.

The selected tab and each tab's filters survive Genesis navigation. Switching tabs
aborts its pending UI requests and stops polling; already accepted backend runs
continue. The dashboard never automatically retries generation or review writes.

## Reading retrieval results

The current evaluator is `paired_context_recovery_v1`. It measures recovery of the
supplied context, **not general answer quality or hallucination detection**. Columns
show mean context Hit@k (also Recall@k for this single positive), mean reciprocal
rank@k, and binary NDCG@k on a 0–1 scale. Larger is better. Means cover successful
queries only; missing values stay blank and zero remains a valid score.

Running, failed, and interrupted runs retain available means but are labelled
partial. Only fully completed runs are eligible for **Compare setup**. A comparison
requires the same metric kind, benchmark fingerprint, top-k, and candidate source
ID set. The highest value in each metric is highlighted, including ties; repeated
runs remain separate rows and different metrics are never averaged.

The panel refreshes every three seconds, with no overlapping polls. Closing it
aborts pending requests and stops polling. Returning preserves filters and fetches
fresh server records. Connection failures retain previously loaded data visibly
as stale, disable mutations, and offer Refresh. Failed mutation requests are never
automatically retried because the server may already have accepted them.

## Verify

From the Genesis workspace:

```sh
pnpm --filter @genesis/benchmarking typecheck
pnpm --filter @genesis/benchmarking test
pnpm --filter @genesis/benchmarking build
```

Tests exercise the Rust response shapes, HTTP errors and request bodies, polling,
partial and incompatible results, both run types, human review, tab restoration, CSV, and Node-only proxy
configuration. The Genesis host has a real registry/loader/navigation test using
backend-shaped responses. No tests require external model calls or dataset downloads.
