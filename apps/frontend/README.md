# RAG benchmarking dashboard

The dashboard reads the existing Rust HTTP API in `apps/backend`. All UI, API
contracts, styling, and the Genesis SDK adapter belong to this submodule. Genesis
only discovers the module, displays it, and supplies generic development proxy
support. The benchmark server is unchanged.

## Connect and open

Start the benchmark backend using the [backend instructions](../../README.md#start).
Then, in the shell running the frontend, export the **same token** used by that
backend. Do not use a `VITE_` variable for credentials.

```sh
export BENCHMARK_API_TOKEN='the-same-local-development-token'
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

The browser calls same-origin `/api/benchmarks/v1` routes. The Node-side proxy adds
the bearer token; it is never stored in panel state or built into the frontend.
Genesis discovers the submodule's `genesisDevelopment.proxy` declaration. Release
packaging excludes development module registrations, frontend assets, and backend
resources. `pnpm --filter @genesis/benchmarking build` builds the standalone UI;
serving that static build separately requires an equivalent authenticated proxy.

## Workflow

1. Select a catalog benchmark. Optionally override its case limit (1–10000), then
   load a snapshot. The server resolves the source, split, adapter, and metric from
   its YAML catalog. Loading is synchronous and may take time.
2. Point your separately configured Nebula runtime at the displayed exported
   corpus and wait for indexing. The backend README explains its connection
   settings. The dashboard does not launch Nebula, choose models, or change the
   architecture.
3. Select a saved snapshot, add an architecture label, and start a run. Blank top-k
   uses that snapshot's saved default, including 8 for older snapshots; an explicit
   top-k must be 1–100. Labels are limited to 256 UTF-8 bytes by the server.
4. Watch progress and saved results in the table. The server permits **one active
   run at a time**. Many saved runs can be searched, filtered, and paginated.
5. Open run details for fingerprint, source IDs, scope, index watermark, and errors.
   Download CSV after a run stops. A setup failure may produce no CSV; the server's
   error is displayed without substituting fabricated data.

## Reading results

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
partial and incompatible results, loading/running actions, CSV, and Node-only proxy
configuration. The Genesis host has a real registry/loader/navigation test using
backend-shaped responses. No tests require external model calls or dataset downloads.
