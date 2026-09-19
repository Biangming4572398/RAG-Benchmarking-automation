# Benchmarking

A frontend-only architecture comparison module for Genesis. Benchmarks are rows;
architectures are columns. Each row declares a metric, unit, and scoring direction.
The table shows completed scores, running/queued/failed results, and unstarted cells.
Best scores are highlighted among the visible architectures, including ties. Scores
from different benchmarks are never averaged.

## Open in Genesis

From the Genesis repository root, initialize the development submodule and install
workspace dependencies:

```sh
git submodule update --init genesis/Modules/dev/RAG-Banchmarks
pnpm install
pnpm start
```

Development Genesis discovers this package's `genesis`
manifest during registry generation, registers it through `@genesis/sdk`, and adds
**Benchmarking** to the default dock. Select it to open the module inside Genesis.
Restart an already-running Genesis app after adding the module. An explicitly
customized dock layout continues to take precedence over the generated default.
Release packaging excludes development modules from its registry and resources.

The module owns its report and filters. Switching away and back preserves them in
the open Genesis module instance. No backend server or SDK contracts are changed.

## Standalone

```sh
pnpm benchmark:ui
```

This opens `/benchmarking.html` in a browser. `pnpm --filter @genesis/benchmarking build`
creates the standalone frontend build in this submodule's `dist` directory. All UI,
styles, sample data, and the SDK adapter are owned by this submodule; Genesis only
discovers and displays its exported module.

## Import and export

The initial table contains explicitly illustrative results. **Import results** replaces
them with a JSON batch report; use [example-results.json](example-results.json) as a
template. Architecture names are arbitrary and can identify system designs, model
architectures, or hardware configurations.

- `schemaVersion`: `1`.
- `name`: batch name.
- `architectures`: objects with unique `id`, `name`, and `description`.
- `benchmarks`: objects with unique `id`, `name`, `suite`, `metric`, `unit`, and
  `direction` (`higher` or `lower`).
- `results`: one object per benchmark/architecture pair, with `benchmarkId`,
  `architectureId`, `status`, and optional `note`. Only `completed` results have a
  finite numeric `value`; other states are `running`, `queued`, or `failed`.
- Omitted benchmark/architecture pairs appear as **Not run**.

Reports describe one comparable result per cell. Combine or aggregate repeated trials
in the producing runner before importing. Run states are snapshots from the report;
this frontend does not launch jobs or poll a backend. Import an updated report to
refresh them. Reports stay in memory; refreshing the standalone page resets them.

Search, suite/status filters, architecture selection, and pagination support larger
batches. A status filter keeps rows with at least one visible result in that state.
The table shows 25 rows per page. CSV export includes all filtered rows, including
other pages, and only the selected architectures. Select any result for its details
and error notes.

The import limit is 8 MB, 2,000 benchmarks, and 50 architectures. There are no remote
requests, provider credentials, backend dependencies, or benchmark execution hooks.
