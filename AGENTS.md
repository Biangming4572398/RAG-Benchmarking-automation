# RAG benchmarking development module

This module is for developers evolving the Genesis storage and Nebula RAG pipeline.
The current task continues backend implementation; frontend work belongs in a
separate task using this handoff.

## Frontend implementation

- Build a metrics dashboard inside the internal version of Genesis. Include its
  registration, host integration, and resources only in internal builds; verify
  they are absent from release builds, rather than merely hiding navigation.
- Read `README.md` for the existing HTTP interface and metric interpretation.
  Use `apps/backend/src/server.rs`, `catalog.rs`, and `trials.rs` as the source of
  truth for request shapes and responses. Backend changes continue independently;
  coordinate contract changes rather than inventing endpoints.
- Support catalog selection, benchmark loading, run creation, progress/status,
  saved results, and CSV downloads through the existing interface. Show failed
  and interrupted runs as partial results, and identify the current metric as
  paired-context recovery rather than general answer quality. Compare completed
  runs with matching benchmark fingerprints, top-k, and candidate source sets.
- Connect through a host-owned proxy or bridge to the loopback benchmark API,
  which requires no authentication. Nebula credentials stay in backend/host
  configuration, outside renderer state and bundles.
  Model selection, Nebula launch, and experiment orchestration remain backend/host
  responsibilities; the current benchmark server does not launch Nebula.
- In the Genesis checkout, inspect `genesis/scripts/generate-registry.ts` and
  `pnpm-workspace.yaml` when wiring the dashboard. They currently discover native
  and external modules only; internal-build discovery of `Modules/dev` is pending.

## Backend direction and isolation

Use the actual Genesis storage and Nebula implementations in experimental
checkouts, with separate storage roots and backend processes. Successful changes
should merge directly into production code. Genesis storage extraction/reuse is
planned, not implemented here: the current Rust backend exports its own Markdown
corpus and calls an already-running standalone Nebula through `/retrieve`.
