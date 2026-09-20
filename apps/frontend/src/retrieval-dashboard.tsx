import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  comparabilityKey,
  createBenchmarkApi,
  type BenchmarkApi,
  type BenchmarkInfo,
  type BenchmarkRun,
  type BenchmarkSnapshot,
  type Scores,
} from './benchmark-api';
import { useBenchmarkWorkspace } from './use-benchmark-workspace';
import styles from './benchmark-dashboard.module.css';

interface DashboardState {
  query: string;
  status: string;
  benchmarkId: string;
  comparison: string;
}

export interface RetrievalDashboardProps {
  initialState?: unknown;
  onStateChange?: (state: unknown) => void;
  onOpenAnswers?: (benchmarkId: string) => void;
  api?: BenchmarkApi;
  pollInterval?: number;
}

const METRICS: { key: keyof Scores; title: string; description: string }[] = [
  {
    key: 'context_hit_at_k',
    title: 'Hit@k',
    description: 'Paired context found in the top k chunks',
  },
  {
    key: 'reciprocal_rank_at_k',
    title: 'MRR@k',
    description: 'Mean reciprocal rank of the paired context',
  },
  {
    key: 'ndcg_at_k',
    title: 'NDCG@k',
    description: 'Binary normalized discounted cumulative gain',
  },
];
const STATUSES = ['running', 'completed', 'failed', 'interrupted'];
const PAGE_SIZE = 25;

function restoreState(saved: unknown): DashboardState {
  const value = saved && typeof saved === 'object' ? (saved as Partial<DashboardState>) : {};
  return {
    query: typeof value.query === 'string' ? value.query : '',
    status: STATUSES.includes(value.status ?? '') ? value.status! : 'all',
    benchmarkId: typeof value.benchmarkId === 'string' ? value.benchmarkId : '',
    comparison: typeof value.comparison === 'string' ? value.comparison : '',
  };
}

function benchmarkName(benchmark?: BenchmarkInfo): string {
  return benchmark?.configuration?.definition.name ?? benchmark?.source ?? 'Unknown snapshot';
}

function score(value: number | undefined): string {
  return value === undefined ? '—' : value.toFixed(3);
}

function download(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

export function RetrievalDashboard({
  initialState,
  onStateChange,
  onOpenAnswers,
  api: providedApi,
  pollInterval = 3000,
}: RetrievalDashboardProps) {
  const api = useMemo(() => providedApi ?? createBenchmarkApi(), [providedApi]);
  const workspace = useBenchmarkWorkspace(api, pollInterval);
  const [state, setState] = useState(() => restoreState(initialState));
  const [topK, setTopK] = useState('');
  const [label, setLabel] = useState('');
  const [pending, setPending] = useState('');
  const [actionError, setActionError] = useState('');
  const [notice, setNotice] = useState('');
  const [detail, setDetail] = useState<BenchmarkRun | null>(null);
  const [snapshot, setSnapshot] = useState<BenchmarkSnapshot | null>(null);
  const [page, setPage] = useState(0);
  const operation = useRef<AbortController | null>(null);
  useEffect(() => () => operation.current?.abort(), []);
  useEffect(() => onStateChange?.(state), [state, onStateChange]);
  useEffect(() => {
    setDetail((current) =>
      current ? (workspace.runs.find((run) => run.id === current.id) ?? current) : null,
    );
  }, [workspace.runs]);

  const retrievalBenchmarks = workspace.benchmarks.filter(
    (item) => item.metric_kind === 'paired_context_recovery_v1',
  );
  const answerBenchmarks = workspace.benchmarks.filter(
    (item) => item.metric_kind === 'hotpotqa_answer_v1',
  );
  const selectedBenchmark =
    retrievalBenchmarks.find((item) => item.id === state.benchmarkId) ??
    retrievalBenchmarks.find((item) => item.configuration?.key === 'ragtruth-qa') ??
    retrievalBenchmarks[0];
  const activeRuns = workspace.runs.filter((run) => run.status === 'running');
  const canRun = workspace.connected && !!selectedBenchmark && !pending && activeRuns.length === 0;
  const byId = new Map(workspace.benchmarks.map((item) => [item.id, item]));
  const comparisonRuns = state.comparison
    ? workspace.runs.filter((run) => comparabilityKey(run) === state.comparison)
    : [];
  const query = state.query.trim().toLowerCase();
  const filteredRuns = workspace.runs
    .filter((run) => !state.comparison || comparabilityKey(run) === state.comparison)
    .filter((run) => state.status === 'all' || run.status === state.status)
    .filter((run) =>
      `${benchmarkName(byId.get(run.request.benchmark_id))} ${run.request.label} ${run.id}`
        .toLowerCase()
        .includes(query),
    )
    .sort((a, b) => b.started_at_ms - a.started_at_ms || a.id.localeCompare(b.id));
  const pages = Math.max(1, Math.ceil(filteredRuns.length / PAGE_SIZE));
  const currentPage = Math.min(page, pages - 1);
  const visibleRuns = filteredRuns.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
  const best = Object.fromEntries(
    METRICS.map(({ key }) => [key, Math.max(...comparisonRuns.map((run) => run.means![key]))]),
  );
  const viewedRun = detail;

  function update(patch: Partial<DashboardState>) {
    setState((current) => ({ ...current, ...patch }));
    setPage(0);
  }

  async function perform(name: string, action: (signal: AbortSignal) => Promise<void>) {
    if (operation.current) return;
    const controller = new AbortController();
    operation.current = controller;
    setPending(name);
    setActionError('');
    setNotice('');
    try {
      await action(controller.signal);
    } catch (cause) {
      if (!controller.signal.aborted) {
        setActionError(cause instanceof Error ? cause.message : 'The request failed.');
        // A timeout does not prove a mutation failed on the server. Refresh its
        // saved state; never automatically retry run creation.
        void workspace.reload();
      }
    } finally {
      if (operation.current === controller) operation.current = null;
      if (!controller.signal.aborted) setPending('');
    }
  }

  function startRun(event: React.FormEvent) {
    event.preventDefault();
    if (!canRun || !selectedBenchmark) return;
    if (new TextEncoder().encode(label).length > 256) {
      setActionError('Architecture label must be at most 256 UTF-8 bytes.');
      return;
    }
    void perform('run', async (signal) => {
      const run = await api.startRun(
        {
          benchmark_id: selectedBenchmark.id,
          label: label.trim(),
          ...(topK ? { top_k: Number(topK) } : {}),
        },
        signal,
      );
      if (signal.aborted) return;
      update({ comparison: '', status: 'all', query: '' });
      setNotice(`Run ${run.id.slice(0, 8)} started. Progress refreshes automatically.`);
      await workspace.reload();
    });
  }

  return (
    <main className={styles.dashboard}>
      <section className={styles.results} aria-label="Benchmark results">
        <header className={styles.resultsHeading}>
          <h1>Architecture comparison</h1>
          <div className={styles.actions}>
            <span className={styles.connection}>
              {workspace.connected
                ? 'Connected'
                : workspace.refreshing && !workspace.updatedAt
                  ? 'Connecting…'
                  : 'Disconnected'}
            </span>
            <button
              className={styles.button}
              disabled={workspace.refreshing}
              onClick={() => void workspace.refresh()}
            >
              {workspace.refreshing ? 'Refreshing…' : 'Refresh'}
            </button>
          </div>
        </header>
        <div className={styles.toolbar}>
          <input
            type="search"
            aria-label="Search benchmarks"
            placeholder="Search benchmark, architecture, or run…"
            value={state.query}
            onChange={(event) => update({ query: event.target.value })}
          />
          <select
            aria-label="Filter run status"
            value={state.status}
            onChange={(event) => update({ status: event.target.value })}
          >
            <option value="all">All run states</option>
            {STATUSES.map((status) => (
              <option key={status} value={status}>
                {status.charAt(0).toUpperCase() + status.slice(1)}
              </option>
            ))}
          </select>
        </div>
        <div
          className={styles.tableScroll}
          tabIndex={0}
          role="region"
          aria-label="Benchmark results table, scroll for more results"
        >
          <table>
            <caption>
              Benchmark results by architecture label. Partial results are excluded from
              comparisons.
            </caption>
            <thead>
              <tr>
                <th scope="col">Benchmark / run</th>
                <th scope="col">Architecture label</th>
                <th scope="col">Status / progress</th>
                <th scope="col">Top k</th>
                {METRICS.map((metric) => (
                  <th scope="col" key={metric.key} title={metric.description}>
                    {metric.title}
                  </th>
                ))}
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {visibleRuns.map((run) => {
                const comparable = comparabilityKey(run);
                const partial = run.status !== 'completed';
                return (
                  <tr key={run.id}>
                    <th scope="row">
                      <strong>{benchmarkName(byId.get(run.request.benchmark_id))}</strong>
                      <small>
                        {run.id.slice(0, 8)} · {new Date(run.started_at_ms).toLocaleString()}
                      </small>
                    </th>
                    <td>
                      <strong>{run.request.label || 'Unlabelled'}</strong>
                      <small>{run.metric_kind}</small>
                    </td>
                    <td>
                      <span className={styles.badge} data-state={run.status}>
                        {run.status}
                      </span>
                      {run.status === 'running' && (
                        <progress
                          aria-label={`Run ${run.id.slice(0, 8)} progress`}
                          max={Math.max(run.total, 1)}
                          value={run.completed + run.failed}
                        />
                      )}
                      <small>
                        {run.completed} / {run.total} successful
                        {run.failed > 0 ? ` · ${run.failed} failed` : ''}
                      </small>
                      {partial && (
                        <small>
                          {run.status === 'running' ? 'In progress' : 'Partial results'}
                        </small>
                      )}
                    </td>
                    <td>{run.request.top_k}</td>
                    {METRICS.map(({ key }) => (
                      <td
                        key={key}
                        className={styles.score}
                        data-best={
                          (!!state.comparison &&
                            comparisonRuns.length > 1 &&
                            run.means?.[key] === best[key]) ||
                          undefined
                        }
                      >
                        {score(run.means?.[key])}
                        {partial && run.means && <small>partial</small>}
                      </td>
                    ))}
                    <td>
                      <div className={styles.rowActions}>
                        <button
                          disabled={!workspace.connected || !!pending}
                          onClick={() =>
                            void perform('detail', async (signal) => {
                              const result = await api.getRun(run.id, signal);
                              if (!signal.aborted) setDetail(result);
                            })
                          }
                          aria-label={`View run ${run.id.slice(0, 8)}`}
                        >
                          Details
                        </button>
                        <button
                          disabled={!workspace.connected || !!pending || run.status === 'running'}
                          onClick={() =>
                            void perform(`csv-${run.id}`, async (signal) => {
                              const blob = await api.downloadScores(run.id, signal);
                              if (!signal.aborted) download(blob, `${run.id}.csv`);
                            })
                          }
                          aria-label={`Download CSV for ${run.id.slice(0, 8)}`}
                        >
                          CSV
                        </button>
                        {comparable && !state.comparison && (
                          <button
                            onClick={() =>
                              update({ comparison: comparable, status: 'all', query: '' })
                            }
                            aria-label={`Compare setup for ${run.id.slice(0, 8)}`}
                          >
                            Compare setup
                          </button>
                        )}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {!filteredRuns.length && (
            <div className={styles.empty}>
              <h3>
                {!workspace.updatedAt
                  ? 'Connect the benchmark server'
                  : workspace.runs.length
                    ? 'No matching runs'
                    : 'No saved runs yet'}
              </h3>
              <p>
                {workspace.runs.length
                  ? 'Change the search or filters to see other results.'
                  : 'Prepare RAGTruth QA using the backend setup instructions, then start a run below.'}
              </p>
            </div>
          )}
        </div>
        <div className={styles.tableFooter}>
          <span>
            {filteredRuns.length} of {workspace.runs.length} runs ·{' '}
            {activeRuns.length ? 'Polling active run' : 'Auto-refresh every 3 seconds'}
          </span>
          <div>
            <button
              className={styles.button}
              disabled={currentPage === 0}
              onClick={() => setPage(currentPage - 1)}
            >
              Previous
            </button>
            <span>
              Page {currentPage + 1} of {pages}
            </span>
            <button
              className={styles.button}
              disabled={currentPage >= pages - 1}
              onClick={() => setPage(currentPage + 1)}
            >
              Next
            </button>
          </div>
        </div>
        {state.comparison ? (
          <div className={styles.comparison}>
            <span>
              Comparing {comparisonRuns.length} completed runs with matching fingerprint, metric,
              top-k, and candidate sources. Best scores are highlighted within this group.
            </span>
            <button className={styles.button} onClick={() => update({ comparison: '' })}>
              Show all runs
            </button>
          </div>
        ) : (
          <p className={styles.tableNote}>
            Use “Compare setup” on a completed run to compare compatible results. Architecture
            labels are supplied by the experimenter.
          </p>
        )}
      </section>
      {!!answerBenchmarks.length && (
        <section className={styles.answerBenchmarks} aria-label="Generated-answer benchmarks">
          <div>
            <strong>Generated-answer benchmarks</strong>
            <small>Automatic exact match and token F1. Open a benchmark to configure a run.</small>
          </div>
          <div className={styles.actions}>
            {answerBenchmarks.map((benchmark) =>
              onOpenAnswers ? (
                <button
                  key={benchmark.id}
                  className={styles.button}
                  aria-label={`Open ${benchmarkName(benchmark)} in Generated answers`}
                  onClick={() => onOpenAnswers(benchmark.id)}
                >
                  {benchmarkName(benchmark)} · {benchmark.case_count} questions →
                </button>
              ) : (
                <span key={benchmark.id}>
                  {benchmarkName(benchmark)} · {benchmark.case_count} questions · Generated answers
                </span>
              ),
            )}
          </div>
        </section>
      )}
      {workspace.error && (
        <div className={styles.error} role="alert">
          <strong>Could not refresh benchmark data.</strong> {workspace.error}
          <p>
            The benchmark server is unavailable.
            {workspace.updatedAt ? ' Previously loaded results remain visible.' : ''} See the module
            README for connection setup.
          </p>
        </div>
      )}
      <p className={styles.explanation}>
        Paired-context recovery measures retrieval of the supplied context, not answer quality.
        Means cover successful queries only; failed and interrupted runs are partial results.
      </p>
      {actionError && (
        <p className={styles.error} role="alert">
          {actionError}
        </p>
      )}
      {notice && (
        <p className={styles.notice} role="status">
          {notice}
        </p>
      )}
      <div className={styles.setup}>
        <form className={styles.panel} onSubmit={startRun} aria-label="Start benchmark run">
          <div className={styles.panelHeading}>
            <h2>Run an architecture</h2>
            <span>Existing Nebula runtime</span>
          </div>
          {!retrievalBenchmarks.length && (
            <p className={styles.muted}>
              No prepared snapshots yet. RAGTruth QA is defined in the team’s Git-managed YAML.
              Follow the backend README to prepare its snapshot and index the corpus in Nebula.
            </p>
          )}
          <label>
            Loaded snapshot
            <select
              value={selectedBenchmark?.id ?? ''}
              onChange={(event) => {
                update({ benchmarkId: event.target.value });
                setTopK('');
                setSnapshot(null);
              }}
              disabled={!!pending || !retrievalBenchmarks.length}
            >
              <option value="" disabled>
                No prepared snapshots
              </option>
              {retrievalBenchmarks.map((item) => (
                <option key={item.id} value={item.id}>
                  {benchmarkName(item)} · {item.case_count} cases · {item.id.slice(0, 8)}
                </option>
              ))}
            </select>
          </label>
          <div className={styles.fields}>
            <label>
              Architecture label
              <input
                value={label}
                onChange={(event) => setLabel(event.target.value)}
                placeholder="e.g. Dense retrieval baseline"
                disabled={!!pending}
              />
            </label>
            <label>
              Top k
              <input
                type="number"
                min="1"
                max="100"
                step="1"
                value={topK}
                placeholder={String(
                  selectedBenchmark?.configuration?.definition.defaults.top_k ?? 8,
                )}
                onChange={(event) => setTopK(event.target.value)}
                disabled={!!pending}
              />
            </label>
            <button className={styles.primaryButton} disabled={!canRun}>
              {pending === 'run'
                ? 'Starting…'
                : activeRuns.length
                  ? 'Run in progress'
                  : 'Start run'}
            </button>
          </div>
          <small>
            The label records your externally configured architecture. It does not change models or
            launch Nebula. Top k defaults to the saved snapshot.
          </small>
        </form>
      </div>
      {selectedBenchmark && (
        <section className={styles.corpus} aria-label="Selected snapshot">
          <div>
            <strong>{benchmarkName(selectedBenchmark)}</strong>
            <span>
              {' '}
              {selectedBenchmark.document_count} candidate documents ·{' '}
              {selectedBenchmark.case_count} cases · {selectedBenchmark.split}
            </span>
            <p>
              Exported corpus: <code>{selectedBenchmark.corpus_path}</code>
            </p>
            <small>
              Use this corpus in the configured Nebula runtime and wait for indexing before starting
              a run.
            </small>
          </div>
          <button
            className={styles.button}
            disabled={!!pending || !workspace.connected}
            onClick={() =>
              void perform('snapshot', async (signal) => {
                const result = await api.getBenchmark(selectedBenchmark.id, signal);
                if (!signal.aborted) setSnapshot(result);
              })
            }
          >
            View snapshot
          </button>
        </section>
      )}
      {snapshot && (
        <section className={styles.detail} aria-label="Snapshot details">
          <div className={styles.panelHeading}>
            <h2>Snapshot {snapshot.id.slice(0, 8)}</h2>
            <button className={styles.button} onClick={() => setSnapshot(null)}>
              Close snapshot
            </button>
          </div>
          <p>
            {snapshot.cases.length} cases · {snapshot.documents.length} documents ·{' '}
            {snapshot.metric_kind}
          </p>
          <ol>
            {snapshot.cases.slice(0, 3).map((item) => (
              <li key={item.id}>{item.query}</li>
            ))}
          </ol>
          <small>Showing the first {Math.min(3, snapshot.cases.length)} saved questions.</small>
        </section>
      )}
      {viewedRun && (
        <section className={styles.detail} aria-label="Run details">
          <div className={styles.panelHeading}>
            <h2>Run {viewedRun.id.slice(0, 8)}</h2>
            <button className={styles.button} onClick={() => setDetail(null)}>
              Close details
            </button>
          </div>
          <p>
            {viewedRun.request.label || 'Unlabelled architecture'} · {viewedRun.status}
            {viewedRun.status !== 'completed' ? ' · Partial results' : ''}
          </p>
          {viewedRun.error && <p className={styles.error}>{viewedRun.error}</p>}
          <dl>
            <dt>Benchmark fingerprint</dt>
            <dd>{viewedRun.benchmark_fingerprint}</dd>
            <dt>Metric</dt>
            <dd>{viewedRun.metric_kind}</dd>
            <dt>Candidate source IDs</dt>
            <dd>{viewedRun.source_ids.join(', ') || 'Not established'}</dd>
            <dt>Scope</dt>
            <dd>
              <code>{JSON.stringify(viewedRun.scope)}</code>
            </dd>
            <dt>Index watermark</dt>
            <dd>
              <code>{JSON.stringify(viewedRun.watermark)}</code>
            </dd>
          </dl>
          <small>
            CSV downloads include per-query scores and evidence. Runs that failed before producing
            rows may have no CSV.
          </small>
        </section>
      )}
      <footer className={styles.footer}>
        Benchmark definitions are maintained in YAML through Git. Results come from the benchmark
        server. One active run per server; saved results remain available across sessions.
      </footer>
    </main>
  );
}
