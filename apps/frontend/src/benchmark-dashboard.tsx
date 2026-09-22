import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  createBenchmarkApi,
  type BenchmarkApi,
  type BenchmarkDefinition,
  type BenchmarkInfo,
  type Catalog,
} from './benchmark-api';
import { createAnswerApi, type AnswerApi, type AnswerRuntime } from './answer-api';
import { createResultsApi, type Results, type ResultsApi, type SuiteRun } from './results-api';
import { AnswerDashboard } from './answer-dashboard';
import { RetrievalDashboard } from './retrieval-dashboard';
import { usePolling } from './use-polling';
import styles from './benchmark-dashboard.module.css';

export interface BenchmarkDashboardProps {
  initialState?: unknown;
  onStateChange?: (state: unknown) => void;
  api?: BenchmarkApi;
  answerApi?: AnswerApi;
  resultsApi?: ResultsApi;
  pollInterval?: number;
}
interface DashboardState {
  benchmark: string;
  view: 'retrieval' | 'answers';
  query: string;
  status: string;
}
interface WorkspaceData {
  catalog: Catalog;
  snapshots: BenchmarkInfo[];
  results: Results;
  suites: SuiteRun[];
}
const EMPTY: WorkspaceData = {
  catalog: { benchmarks: {} },
  snapshots: [],
  results: { benchmarks: [] },
  suites: [],
};
const PAGE_SIZE = 25;
const STATUSES = ['running', 'completed', 'failed', 'interrupted'];
const METRIC_LABELS: Record<string, string> = {
  context_hit_at_k: 'Hit@k',
  reciprocal_rank_at_k: 'MRR@k',
  ndcg_at_k: 'NDCG@k',
  exact_match: 'Answer EM',
  f1: 'Answer F1',
  correctness: 'Correctness',
  groundedness: 'Groundedness',
  hallucination_rate: 'Hallucinations',
  citation_accuracy: 'Citation accuracy',
};
function restore(saved: unknown): DashboardState {
  const value = saved && typeof saved === 'object' ? (saved as Partial<DashboardState>) : {};
  return {
    benchmark: typeof value.benchmark === 'string' ? value.benchmark : '',
    view: value.view === 'answers' ? 'answers' : 'retrieval',
    query: typeof value.query === 'string' ? value.query : '',
    status: STATUSES.includes(value.status ?? '') ? value.status! : 'all',
  };
}
function snapshotKey(snapshot: BenchmarkInfo) {
  return (
    snapshot.module_key ??
    snapshot.configuration?.key ??
    (snapshot.metric_kind === 'paired_context_recovery_v1'
      ? 'ragtruth-qa'
      : snapshot.metric_kind === 'hotpotqa_answer_v1'
        ? 'hotpotqa'
        : snapshot.metric_kind)
  );
}
function displayName(key: string, definition?: BenchmarkDefinition) {
  return definition?.name ?? { 'ragtruth-qa': 'RAGTruth QA', hotpotqa: 'HotpotQA' }[key] ?? key;
}
function saveDownload(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
function metricLabel(column: string) {
  const [group, key] = column.split('.');
  return `${group === 'review' ? 'Review · ' : ''}${METRIC_LABELS[key] ?? key.replaceAll('_', ' ')}`;
}
function score(value: string | undefined) {
  if (value === undefined || value === '') return '—';
  const number = Number(value);
  return Number.isFinite(number) ? number.toFixed(3) : '—';
}
function statusLabel(status: string) {
  return status === 'completed' ? 'Completed' : status.charAt(0).toUpperCase() + status.slice(1);
}

export function BenchmarkDashboard({
  initialState,
  onStateChange,
  api,
  answerApi,
  resultsApi,
  pollInterval = 3000,
}: BenchmarkDashboardProps) {
  const id = useId();
  const benchmarkApi = useMemo(() => api ?? createBenchmarkApi(), [api]);
  const generatedApi = useMemo(() => answerApi ?? createAnswerApi(), [answerApi]);
  const tablesApi = useMemo(() => resultsApi ?? createResultsApi(), [resultsApi]);
  const [state, setState] = useState(() => restore(initialState));
  const [page, setPage] = useState(0);
  const [showRunForm, setShowRunForm] = useState(false);
  const [architecture, setArchitecture] = useState('baseline');
  const [description, setDescription] = useState('');
  const [profileId, setProfileId] = useState('');
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState('');
  const [notice, setNotice] = useState('');
  const [suiteId, setSuiteId] = useState('');
  const [inspection, setInspection] = useState<string | null>(null);
  const inspectionHeading = useRef<HTMLHeadingElement | null>(null);
  const inspectionTrigger = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (inspection !== null) {
      inspectionHeading.current?.focus();
      inspectionHeading.current?.scrollIntoView?.({ block: 'start' });
    }
  }, [inspection]);
  const operation = useRef<AbortController | null>(null);
  const formHeading = useRef<HTMLInputElement | null>(null);
  useEffect(() => onStateChange?.(state), [state, onStateChange]);
  useEffect(() => () => operation.current?.abort(), []);
  useEffect(() => {
    if (showRunForm) formHeading.current?.focus();
  }, [showRunForm]);
  const load = useCallback(
    async (signal: AbortSignal) => {
      const [catalog, snapshots, results, suites] = await Promise.all([
        benchmarkApi.getCatalog(signal),
        benchmarkApi.listBenchmarks(signal),
        tablesApi.getResults(signal),
        tablesApi.listSuites(signal),
      ]);
      return { catalog, snapshots, results, suites };
    },
    [benchmarkApi, tablesApi],
  );
  const workspace = usePolling(load, EMPTY, pollInterval);
  const loadRuntime = useCallback(
    (signal: AbortSignal) => generatedApi.getRuntime(signal),
    [generatedApi],
  );
  const runtime = usePolling<AnswerRuntime | null>(loadRuntime, null, pollInterval);
  const { catalog, snapshots, results, suites } = workspace.data;
  const definitions = new Map(
    Object.entries(catalog.benchmarks).map(([key, value]) => [
      catalog.module_keys?.[key] ?? key,
      value,
    ]),
  );
  for (const snapshot of snapshots) {
    const key = snapshotKey(snapshot);
    if (!definitions.has(key) && snapshot.configuration)
      definitions.set(key, snapshot.configuration.definition);
  }
  const keys = [
    ...new Set([
      ...definitions.keys(),
      ...snapshots.map(snapshotKey),
      ...results.benchmarks.map((table) => table.benchmark),
    ]),
  ];
  const selectedKey = keys.includes(state.benchmark)
    ? state.benchmark
    : keys.includes('ragtruth-qa')
      ? 'ragtruth-qa'
      : (keys.find((key) =>
          results.benchmarks.some((table) => table.benchmark === key && table.rows.length),
        ) ??
        keys[0] ??
        '');
  const definition = definitions.get(selectedKey);
  const name = displayName(selectedKey, definition);
  const table = results.benchmarks.find((item) => item.benchmark === selectedKey);
  const selectedSnapshots = snapshots.filter((item) => snapshotKey(item) === selectedKey);
  const allRows = table?.rows ?? [];
  const mode = state.view === 'answers' ? 'generation' : 'retrieval';
  const modeRows = allRows.filter((row) => row.mode === mode);
  const query = state.query.trim().toLowerCase();
  const filtered = modeRows
    .filter(
      (row) =>
        (state.status === 'all' || row.status === state.status) &&
        [
          row.suite_run_number,
          row.run_number,
          row.run_id,
          row.architecture_name,
          row.description,
          row.embedding_model,
          row.generation_model,
        ]
          .join(' ')
          .toLowerCase()
          .includes(query),
    )
    .sort(
      (a, b) =>
        Number(b.suite_run_number || b.run_number) - Number(a.suite_run_number || a.run_number) ||
        Number(b.run_number) - Number(a.run_number),
    );
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, pages - 1);
  const visible = filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
  // Each evaluator owns its metrics. Never assume the three original retrieval columns.
  const metricColumns = (table?.columns ?? []).filter(
    (column) =>
      (column.startsWith('metric.') || (mode === 'generation' && column.startsWith('review.'))) &&
      modeRows.some((row) => row[column] !== undefined && row[column] !== ''),
  );
  const sortedSuites = [...suites].sort((a, b) => b.run_number - a.run_number);
  const latestSuite = sortedSuites.find((suite) => suite.id === suiteId) ?? sortedSuites[0];
  const active =
    suites.some((suite) => suite.status === 'running') ||
    results.benchmarks.some((item) => item.rows.some((row) => row.status === 'running'));
  const runtimeSettled = !!runtime.updatedAt || !!runtime.error;
  const profiles = runtime.data?.profiles.filter((item) => item.enabled) ?? [];
  const profile = profiles.find((item) => item.id === profileId) ?? profiles[0];
  const prepared = new Set(snapshots.map(snapshotKey));
  const hasRunnable = snapshots.some((item) =>
    ['paired_context_recovery_v1', 'hotpotqa_answer_v1'].includes(item.metric_kind),
  );
  const selection = useRef({ ids: new Set<string>(), runIds: new Set<string>() });
  selection.current = {
    ids: new Set(selectedSnapshots.map((item) => item.id)),
    runIds: new Set(allRows.map((row) => row.run_id)),
  };
  const scopedApi = useMemo<BenchmarkApi>(
    () => ({
      ...benchmarkApi,
      listBenchmarks: async (signal) =>
        (await benchmarkApi.listBenchmarks(signal)).filter(
          (item) => snapshotKey(item) === selectedKey,
        ),
      listRuns: async (signal) =>
        (await benchmarkApi.listRuns(signal)).filter(
          (run) =>
            selection.current.ids.has(run.request.benchmark_id) ||
            selection.current.runIds.has(run.id),
        ),
    }),
    [benchmarkApi, selectedKey],
  );
  const scopedAnswerApi = useMemo<AnswerApi>(
    () => ({
      ...generatedApi,
      listRuns: async (signal) =>
        (await generatedApi.listRuns(signal)).filter(
          (run) =>
            selection.current.ids.has(run.request.benchmark_id) ||
            selection.current.runIds.has(run.id),
        ),
    }),
    [generatedApi, selectedKey],
  );

  function update(patch: Partial<DashboardState>) {
    setState((current) => ({ ...current, ...patch }));
    setPage(0);
    setInspection(null);
  }
  function selectBenchmark(key: string) {
    const answerOnly =
      definitions.get(key)?.evaluation === 'hotpotqa_answer_v1' ||
      snapshots.some(
        (item) => snapshotKey(item) === key && item.metric_kind === 'hotpotqa_answer_v1',
      );
    update({
      benchmark: key,
      query: '',
      status: 'all',
      ...(answerOnly ? { view: 'answers' } : {}),
    });
  }
  async function perform(action: (signal: AbortSignal) => Promise<void>) {
    if (operation.current) return;
    const controller = new AbortController();
    operation.current = controller;
    setPending(true);
    setActionError('');
    setNotice('');
    try {
      await action(controller.signal);
    } catch (cause) {
      if (!controller.signal.aborted) {
        setActionError(cause instanceof Error ? cause.message : 'The request failed.');
        // A lost response may already have started the suite. Refresh, never retry the write.
        void workspace.reload();
      }
    } finally {
      if (operation.current === controller) operation.current = null;
      if (!controller.signal.aborted) setPending(false);
    }
  }
  function startSuite(event: React.FormEvent) {
    event.preventDefault();
    if (!workspace.connected || active || pending || !hasRunnable || !runtimeSettled) return;
    if (!architecture.trim() || new TextEncoder().encode(architecture.trim()).length > 256) {
      setActionError('Architecture label must contain 1–256 UTF-8 bytes.');
      return;
    }
    if (new TextEncoder().encode(description).length > 4000) {
      setActionError('Run notes must be at most 4000 UTF-8 bytes.');
      return;
    }
    void perform(async (signal) => {
      const suite = await tablesApi.startSuite(
        {
          architecture_label: architecture.trim(),
          description,
          ...(runtime.connected && runtime.data?.available && profile
            ? { profile_id: profile.id }
            : {}),
        },
        signal,
      );
      if (signal.aborted) return;
      setShowRunForm(false);
      setSuiteId(suite.id);
      setNotice(
        `Run #${suite.run_number} started. All benchmark outcomes are saved automatically.`,
      );
      update({ query: '', status: 'all' });
      await workspace.reload();
    });
  }
  function keyboard(event: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    if (!['ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? 1 : 1 - index;
    update({ view: next ? 'answers' : 'retrieval' });
    event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      [next]?.focus();
  }

  return (
    <div className={styles.workspace}>
      <header className={styles.appHeader}>
        <div>
          <h1 className={styles.appTitle}>RAG Benchmarks</h1>
          <p className={styles.appSubtitle}>One experiment. Every benchmark.</p>
        </div>
        <div className={styles.headerActions}>
          <span className={styles.connection}>
            <span
              className={styles.statusDot}
              data-state={workspace.connected ? 'connected' : 'disconnected'}
            />
            {workspace.connected
              ? 'Connected'
              : workspace.refreshing && !workspace.updatedAt
                ? 'Connecting…'
                : 'Disconnected'}
          </span>
          <button
            className={styles.button}
            onClick={() => {
              void workspace.refresh();
              void runtime.refresh();
            }}
            disabled={workspace.refreshing}
          >
            Refresh
          </button>
          <button
            className={styles.primaryButton}
            onClick={() => setShowRunForm((value) => !value)}
            aria-expanded={showRunForm}
            aria-controls={`${id}-new-run`}
            disabled={active}
          >
            {' '}
            {active ? 'Run in progress' : 'Run all benchmarks'}
          </button>
        </div>
      </header>
      <div className={styles.workspaceBody}>
        <nav className={styles.sidebar} aria-label="Benchmarks">
          <p className={styles.sidebarHeading}>
            Benchmarks <span>{keys.length}</span>
          </p>
          {keys.map((key) => {
            const entry = definitions.get(key);
            const rows = results.benchmarks.find((item) => item.benchmark === key)?.rows ?? [];
            const count = new Set(rows.map((row) => row.suite_run_number || row.run_number)).size;
            return (
              <button
                key={key}
                className={styles.benchmarkButton}
                aria-current={selectedKey === key ? 'true' : undefined}
                onClick={() => selectBenchmark(key)}
              >
                <span className={styles.benchmarkIcon} aria-hidden="true">
                  <svg
                    width="18"
                    height="18"
                    viewBox="0 0 20 20"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                  >
                    <rect x="3" y="3" width="14" height="14" rx="3" />
                    <path d="M3 8h14M8 8v9" />
                  </svg>
                </span>
                <span>
                  <span className={styles.benchmarkName}>{displayName(key, entry)}</span>
                  <span className={styles.benchmarkMeta}>
                    {count
                      ? `${count} run${count === 1 ? '' : 's'}`
                      : entry?.adapter === 'external_suite'
                        ? 'Integration required'
                        : prepared.has(key)
                          ? 'Ready to run'
                          : 'Not prepared'}
                  </span>
                </span>
              </button>
            );
          })}
          {!keys.length && (
            <p className={styles.muted}>
              {workspace.connected ? 'No configured benchmarks.' : 'Connect to load benchmarks.'}
            </p>
          )}
          <div className={styles.sidebarFooter}>
            Local experiment workspace
            <br />
            {prepared.size} prepared · {keys.length} benchmarks
          </div>
        </nav>
        <main className={styles.mainContent}>
          {workspace.error && (
            <p className={styles.error} role="alert">
              Could not refresh benchmark data. {workspace.error}
              {workspace.updatedAt ? ' Previously loaded results remain visible.' : ''}
            </p>
          )}
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
          {showRunForm && (
            <form
              id={`${id}-new-run`}
              className={styles.suiteForm}
              aria-label="Run all benchmarks"
              onSubmit={startSuite}
            >
              <div className={styles.panelHeading}>
                <h2>New benchmark run</h2>
                <button
                  type="button"
                  className={styles.button}
                  onClick={() => setShowRunForm(false)}
                >
                  Cancel
                </button>
              </div>
              <p className={styles.muted}>
                One run tests every prepared benchmark in sequence. Retrieval and answer results are
                saved separately under the same run number. Unprepared benchmarks and unavailable
                evaluations are recorded as skipped.
              </p>
              <div className={styles.suiteFields}>
                <label>
                  Architecture label
                  <input
                    ref={formHeading}
                    required
                    value={architecture}
                    onChange={(event) => setArchitecture(event.target.value)}
                    placeholder="e.g. Dense retrieval v2"
                    disabled={pending}
                  />
                </label>
                <label>
                  Generation profile
                  <select
                    value={profile?.id ?? ''}
                    onChange={(event) => setProfileId(event.target.value)}
                    disabled={
                      pending || !runtime.connected || !runtime.data?.available || !profiles.length
                    }
                  >
                    <option value="" disabled>
                      No generation profile available
                    </option>
                    {profiles.map((item) => (
                      <option key={item.id} value={item.id}>
                        {item.label}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <label>
                Run notes
                <textarea
                  value={description}
                  onChange={(event) => setDescription(event.target.value)}
                  rows={2}
                  placeholder="What changed in this experiment?"
                  disabled={pending}
                />
              </label>
              {!runtimeSettled && <p className={styles.muted}>Checking generation availability…</p>}
              {runtimeSettled && (!runtime.connected || !runtime.data?.available || !profile) && (
                <p className={styles.muted}>
                  {runtime.error || runtime.data?.reason || 'Generation is unavailable.'} Answer
                  evaluations will be skipped; prepared retrieval benchmarks can still run.
                </p>
              )}
              <p className={styles.muted}>
                Uses the latest prepared snapshot per benchmark and the configured Nebula runtime.
                Generation uses its configured provider.
              </p>
              <button
                className={styles.primaryButton}
                disabled={
                  pending || active || !workspace.connected || !hasRunnable || !runtimeSettled
                }
              >
                {pending ? 'Starting…' : 'Start suite run'}
              </button>
              {!hasRunnable && (
                <p className={styles.muted}>
                  Prepare a supported benchmark snapshot using the backend setup instructions first.
                </p>
              )}
            </form>
          )}
          <header className={styles.contentHeader}>
            <div>
              <p className={styles.eyebrow}>Benchmark results</p>
              <h2 className={styles.contentTitle}>
                {selectedKey ? name : 'Your benchmark workspace'}
              </h2>
              <p className={styles.muted}>
                {definition?.description ??
                  'Select a benchmark to inspect its saved runs and evaluation results.'}
              </p>
            </div>
          </header>
          <div className={styles.summaryGrid}>
            <div className={styles.summaryCard}>
              <span className={styles.summaryValue}>
                {new Set(allRows.map((row) => row.suite_run_number || row.run_number)).size}
              </span>
              <span className={styles.summaryLabel}>Saved runs</span>
            </div>
            <div className={styles.summaryCard}>
              <span className={styles.summaryValue}>
                {modeRows.filter((row) => row.status === 'completed').length}
                <small> / {modeRows.length}</small>
              </span>
              <span className={styles.summaryLabel}>Completed evaluations</span>
            </div>
            <div className={styles.summaryCard}>
              <span className={styles.summaryValue}>{selectedSnapshots.length}</span>
              <span className={styles.summaryLabel}>Prepared snapshots</span>
            </div>
          </div>
          <section className={styles.resultsCard} aria-label={`${name} results`}>
            <div className={styles.resultsToolbar}>
              <div className={styles.paneTabs} role="tablist" aria-label="Benchmark evaluation">
                {(['retrieval', 'answers'] as const).map((view, index) => (
                  <button
                    key={view}
                    id={`${id}-${view}-tab`}
                    role="tab"
                    aria-selected={state.view === view}
                    aria-controls={`${id}-results-panel`}
                    tabIndex={state.view === view ? 0 : -1}
                    onClick={() => update({ view })}
                    onKeyDown={(event) => keyboard(event, index)}
                  >
                    {view === 'retrieval' ? 'Retrieval' : 'Generated answers'}
                  </button>
                ))}
              </div>
              <button
                className={styles.button}
                disabled={pending || !workspace.connected || !allRows.length}
                onClick={() =>
                  void perform(async (signal) => {
                    const blob = await tablesApi.downloadResults(selectedKey, signal);
                    if (!signal.aborted) saveDownload(blob, `${selectedKey}-results.csv`);
                  })
                }
              >
                Export benchmark CSV
              </button>
            </div>
            <div className={styles.toolbar}>
              <input
                type="search"
                aria-label="Search runs"
                placeholder="Search run number, architecture, or notes…"
                value={state.query}
                onChange={(event) => update({ query: event.target.value })}
              />
              <select
                aria-label="Filter run status"
                value={state.status}
                onChange={(event) => update({ status: event.target.value })}
              >
                <option value="all">All statuses</option>
                {STATUSES.map((status) => (
                  <option key={status} value={status}>
                    {statusLabel(status)}
                  </option>
                ))}
              </select>
            </div>
            <div
              role="tabpanel"
              id={`${id}-results-panel`}
              aria-labelledby={`${id}-${state.view}-tab`}
            >
              <div
                className={styles.tableScroll}
                role="region"
                aria-label={`${name} run results table`}
                tabIndex={0}
              >
                <table>
                  <caption>
                    {name} results by run number. Metrics retain their evaluator and coverage;
                    partial results are not comparable.
                  </caption>
                  <thead>
                    <tr>
                      <th scope="col">Run</th>
                      <th scope="col">Architecture</th>
                      <th scope="col">Status / coverage</th>
                      <th scope="col">Top k</th>
                      {metricColumns.map((column) => (
                        <th scope="col" key={column}>
                          {metricLabel(column)}
                        </th>
                      ))}
                      {mode === 'generation' && <th scope="col">Models</th>}
                      <th scope="col">Details</th>
                    </tr>
                  </thead>
                  <tbody>
                    {visible.map((row) => (
                      <tr key={row.run_id}>
                        <th scope="row">
                          <strong>#{row.suite_run_number || row.run_number}</strong>
                          <small>
                            {Number(row.started_at_ms)
                              ? new Date(Number(row.started_at_ms)).toLocaleString()
                              : 'Saved run'}
                          </small>
                          {row.suite_run_number && <small>Execution #{row.run_number}</small>}
                        </th>
                        <td>
                          <strong>{row.architecture_name || 'Unlabelled architecture'}</strong>
                          {row.description && <small>{row.description}</small>}
                          <small>
                            {row.evaluation} · {row.benchmark_fingerprint?.slice(0, 10)}
                          </small>
                        </td>
                        <td>
                          <span className={styles.badge} data-state={row.status}>
                            {statusLabel(row.status)}
                          </span>
                          <small>
                            {Number(row.completed || 0) + Number(row.failed || 0)} /{' '}
                            {row.total || 0} processed
                            {Number(row.failed) > 0 ? ` · ${row.failed} failed` : ''}
                          </small>
                          {mode === 'generation' && (
                            <>
                              <small>
                                {row.answered || 0} answered · {row.reviewed || 0} reviewed
                              </small>
                              {row.evaluation === 'hotpotqa_answer_v1' && (
                                <small>
                                  {row.scored || 0} / {row.total || 0} scored
                                </small>
                              )}
                            </>
                          )}
                          {row.status === 'running' && (
                            <progress
                              aria-label={`Run ${row.suite_run_number || row.run_number} progress`}
                              value={Number(row.completed || 0) + Number(row.failed || 0)}
                              max={Math.max(1, Number(row.total || 0))}
                            />
                          )}
                        </td>
                        <td>{row.top_k || '—'}</td>
                        {metricColumns.map((column) => (
                          <td key={column} className={styles.score}>
                            {score(row[column])}
                            {row[column] &&
                              (row.status !== 'completed' ||
                                (column.startsWith('review.') &&
                                  Number(row.reviewed) < Number(row.total))) && (
                                <small>Partial coverage</small>
                              )}
                          </td>
                        ))}
                        {mode === 'generation' && (
                          <td>
                            {row.generation_model || '—'}
                            <small>{row.embedding_model || 'No embedding identity recorded'}</small>
                          </td>
                        )}
                        <td>
                          <button
                            className={styles.button}
                            onClick={(event) => {
                              inspectionTrigger.current = event.currentTarget;
                              setInspection(row.run_id);
                            }}
                            aria-expanded={inspection === row.run_id}
                            aria-controls={`${id}-inspection`}
                            aria-label={`Inspect run ${row.suite_run_number || row.run_number}`}
                          >
                            Inspect
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {!visible.length && (
                  <div className={styles.empty}>
                    <h3>{modeRows.length ? 'No matching runs' : 'No results yet'}</h3>
                    <p>
                      {definition?.adapter === 'external_suite'
                        ? 'This benchmark needs a backend integration before it can produce scores.'
                        : modeRows.length
                          ? 'Try a different search or status.'
                          : selectedSnapshots.length
                            ? mode === 'retrieval' &&
                              definition?.evaluation === 'hotpotqa_answer_v1'
                              ? 'This benchmark evaluates generated answers. Switch to Generated answers.'
                              : 'Start a suite run to save results for this benchmark.'
                            : 'Prepare a benchmark snapshot in the backend to get started.'}
                    </p>
                  </div>
                )}
              </div>
              <div className={styles.tableFooter}>
                <span>
                  {filtered.length} {mode === 'generation' ? 'answer' : 'retrieval'} evaluations ·
                  auto-refresh every {pollInterval / 1000}s
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
            </div>
            <p className={styles.tableNote}>
              {mode === 'retrieval'
                ? 'Retrieval scores measure context recovery, with means over successful queries.'
                : 'Automatic scores and human reviews are separate. Unreviewed scores remain blank; answer and review coverage stay visible.'}{' '}
              Open Inspect to compare runs with matching snapshots, settings and candidate sources.
            </p>
          </section>
          {inspection !== null && (
            <section
              id={`${id}-inspection`}
              className={styles.resultsCard}
              aria-label="Run inspection"
            >
              <div className={styles.panelHeading}>
                <h2 ref={inspectionHeading} tabIndex={-1} className={styles.sectionTitle}>
                  Evidence &amp; comparison
                </h2>
                <button
                  className={styles.button}
                  onClick={() => {
                    setInspection(null);
                    inspectionTrigger.current?.focus();
                  }}
                >
                  Close inspection
                </button>
              </div>
              {state.view === 'retrieval' ? (
                <RetrievalDashboard
                  key={`${selectedKey}-retrieval-${inspection}`}
                  api={scopedApi}
                  initialState={{ query: inspection }}
                  inspectionOnly
                  pollInterval={pollInterval}
                />
              ) : (
                <AnswerDashboard
                  key={`${selectedKey}-answers-${inspection}`}
                  api={scopedAnswerApi}
                  benchmarkApi={scopedApi}
                  initialState={{ query: inspection }}
                  inspectionOnly
                  pollInterval={pollInterval}
                />
              )}
            </section>
          )}
          {latestSuite && (
            <label className={styles.suiteHistory}>
              Suite run history
              <select value={latestSuite.id} onChange={(event) => setSuiteId(event.target.value)}>
                {sortedSuites.map((suite) => (
                  <option key={suite.id} value={suite.id}>
                    Run #{suite.run_number} · {suite.request.architecture_label} ·{' '}
                    {statusLabel(suite.status)}
                  </option>
                ))}
              </select>
            </label>
          )}
          {latestSuite && (
            <details
              className={styles.suiteProgress}
              key={latestSuite.id}
              open={latestSuite.status === 'running'}
            >
              <summary>
                Run #{latestSuite.run_number} · {latestSuite.request.architecture_label}{' '}
                <span className={styles.badge} data-state={latestSuite.status}>
                  {statusLabel(latestSuite.status)}
                </span>{' '}
                · {latestSuite.items.filter((item) => item.status === 'completed').length} completed
                · {latestSuite.items.filter((item) => item.status === 'skipped').length} skipped
              </summary>
              {latestSuite.error && <p className={styles.error}>{latestSuite.error}</p>}
              {latestSuite.items.map((item, index) => (
                <div className={styles.suiteItem} key={`${item.benchmark}-${item.mode}-${index}`}>
                  <span>
                    {displayName(item.benchmark, definitions.get(item.benchmark))}
                    {item.mode && (
                      <small>
                        {item.mode === 'generation' ? 'Generated answers' : 'Retrieval'}
                      </small>
                    )}
                  </span>
                  <span>
                    <span className={styles.badge} data-state={item.status}>
                      {statusLabel(item.status)}
                    </span>
                    {item.reason && <small>{item.reason}</small>}
                  </span>
                </div>
              ))}
            </details>
          )}
          {selectedKey && (
            <details className={styles.disclosure}>
              <summary>Benchmark setup &amp; snapshots</summary>
              <p>
                {definition?.preparation ??
                  'Prepare snapshots through the benchmark backend, then index the exported corpus in your configured Nebula runtime.'}
              </p>
              <p className={styles.muted}>
                Definitions and dataset preparation are managed in YAML and Git. Each run keeps its
                original snapshot fingerprint.
              </p>
              {selectedSnapshots.map((snapshot) => (
                <div className={styles.suiteItem} key={snapshot.id}>
                  <span>
                    {snapshot.case_count} cases · {snapshot.document_count} documents ·{' '}
                    {snapshot.split}
                    <small>{snapshot.id}</small>
                  </span>
                  <code>{snapshot.corpus_path}</code>
                </div>
              ))}
            </details>
          )}
        </main>
      </div>
    </div>
  );
}
