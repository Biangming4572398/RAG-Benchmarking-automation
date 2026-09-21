import React, { useCallback, useEffect, useRef, useState } from 'react';
import { BenchmarkCatalog } from './benchmark-catalog';
import type { BenchmarkApi, BenchmarkInfo, BenchmarkRun } from './benchmark-api';
import {
  answerComparabilityKey,
  answerFailureLog,
  type AnswerApi,
  type AnswerCase,
  type AnswerMeans,
  type AnswerReviewRequest,
  type AnswerRun,
  type AnswerRunSummary,
  type AnswerRuntime,
} from './answer-api';
import { usePolling } from './use-polling';
import styles from './benchmark-dashboard.module.css';

interface AnswerState {
  query: string;
  status: string;
  benchmarkId: string;
  comparison: string;
}
interface Props {
  api: AnswerApi;
  benchmarkApi: BenchmarkApi;
  initialState?: unknown;
  onStateChange?: (state: unknown) => void;
  pollInterval?: number;
  inspectionOnly?: boolean;
}
const EMPTY: {
  runs: AnswerRunSummary[];
  benchmarks: BenchmarkInfo[];
  retrieval: BenchmarkRun[];
  runtime: AnswerRuntime | null;
} = { runs: [], benchmarks: [], retrieval: [], runtime: null };
const STATUSES = ['running', 'completed', 'failed', 'interrupted'];
const METRICS: { key: keyof AnswerMeans; label: string; lower?: boolean }[] = [
  { key: 'correctness', label: 'Correctness' },
  { key: 'groundedness', label: 'Groundedness' },
  { key: 'hallucination_rate', label: 'Hallucinations', lower: true },
  { key: 'citation_accuracy', label: 'Citation accuracy' },
];
const AUTOMATIC_METRICS = [
  { key: 'exact_match', label: 'Answer EM' },
  { key: 'f1', label: 'Answer F1' },
] as const;
function evaluationLabel(evaluation: string) {
  return evaluation === 'hotpotqa_answer_v1' ? 'HotpotQA EM/F1 v1' : 'Human review v1';
}
function restore(saved: unknown): AnswerState {
  const value = saved && typeof saved === 'object' ? (saved as Partial<AnswerState>) : {};
  return {
    query: typeof value.query === 'string' ? value.query : '',
    status: STATUSES.includes(value.status ?? '') ? value.status! : 'all',
    benchmarkId: typeof value.benchmarkId === 'string' ? value.benchmarkId : '',
    comparison: typeof value.comparison === 'string' ? value.comparison : '',
  };
}
function name(benchmark?: BenchmarkInfo) {
  return benchmark?.configuration?.definition.name ?? benchmark?.source ?? 'Unknown snapshot';
}
function percent(value: number | undefined) {
  return value === undefined ? '—' : `${(value * 100).toFixed(1)}%`;
}

function saveDownload(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

export function AnswerDashboard({
  api,
  benchmarkApi,
  initialState,
  onStateChange,
  pollInterval = 3000,
  inspectionOnly = false,
}: Props) {
  const [state, setState] = useState(() => restore(initialState));
  const [architecture, setArchitecture] = useState('');
  const [profileId, setProfileId] = useState('');
  const [pending, setPending] = useState('');
  const [actionError, setActionError] = useState('');
  const [notice, setNotice] = useState('');
  const [page, setPage] = useState(0);
  const [detail, setDetail] = useState<AnswerRun | null>(null);
  const [selectedCaseId, setSelectedCaseId] = useState('');
  const operation = useRef<AbortController | null>(null);
  const detailElement = useRef<HTMLElement | null>(null);
  const runForm = useRef<HTMLFormElement | null>(null);
  useEffect(() => () => operation.current?.abort(), []);
  useEffect(() => onStateChange?.(state), [state, onStateChange]);
  const load = useCallback(
    async (signal: AbortSignal) => {
      const [runs, benchmarks, retrieval, runtime] = await Promise.all([
        api.listRuns(signal),
        benchmarkApi.listBenchmarks(signal),
        benchmarkApi.listRuns(signal),
        api.getRuntime(signal),
      ]);
      return { runs, benchmarks, retrieval, runtime };
    },
    [api, benchmarkApi],
  );
  const workspace = usePolling(load, EMPTY, pollInterval);
  const { runs, benchmarks, retrieval, runtime } = workspace.data;
  useEffect(() => {
    if (!detail || detail.status !== 'running') return;
    const controller = new AbortController();
    void api
      .getRun(detail.id, controller.signal)
      .then((run) => {
        if (!controller.signal.aborted) setDetail(run);
      })
      .catch((cause) => {
        if (!controller.signal.aborted)
          setActionError(cause instanceof Error ? cause.message : 'Cannot refresh answers.');
      });
    return () => controller.abort();
  }, [api, runs, detail?.id, detail?.status]);
  useEffect(() => {
    if (detail) detailElement.current?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
  }, [detail?.id]);
  const selected =
    benchmarks.find((item) => item.id === state.benchmarkId) ??
    benchmarks.find((item) => item.configuration?.key === 'ragtruth-qa') ??
    benchmarks[0];
  const selectedHotpot = selected?.metric_kind === 'hotpotqa_answer_v1';
  const showAutomatic =
    benchmarks.some((item) => item.metric_kind === 'hotpotqa_answer_v1') ||
    runs.some((run) => run.evaluation === 'hotpotqa_answer_v1');
  const profiles = runtime?.profiles.filter((item) => item.enabled) ?? [];
  const profile = profiles.find((item) => item.id === profileId) ?? profiles[0];
  const active = [...runs, ...retrieval].some((run) => run.status === 'running');
  const byId = new Map(benchmarks.map((item) => [item.id, item]));
  const comparison = state.comparison
    ? runs.filter((run) => answerComparabilityKey(run) === state.comparison)
    : [];
  const query = state.query.trim().toLowerCase();
  const filtered = runs
    .filter((run) => !state.comparison || answerComparabilityKey(run) === state.comparison)
    .filter((run) => state.status === 'all' || run.status === state.status)
    .filter((run) =>
      `${name(byId.get(run.request.benchmark_id))} ${run.request.architecture_label} ${run.embedding_model.id} ${run.generation_model.label} ${run.id}`
        .toLowerCase()
        .includes(query),
    )
    .sort((a, b) => b.started_at_ms - a.started_at_ms || a.id.localeCompare(b.id));
  const pages = Math.max(1, Math.ceil(filtered.length / 25));
  const currentPage = Math.min(page, pages - 1);
  const visible = filtered.slice(currentPage * 25, (currentPage + 1) * 25);
  const best = Object.fromEntries(
    METRICS.map(({ key, lower }) => [
      key,
      (lower ? Math.min : Math.max)(
        ...comparison
          .filter((run) => run.evaluation === 'manual_review_v1' && run.means)
          .map((run) => run.means![key]),
      ),
    ]),
  );
  const bestAutomatic = Object.fromEntries(
    AUTOMATIC_METRICS.map(({ key }) => [
      key,
      Math.max(
        ...comparison
          .filter((run) => run.automatic_scores)
          .map((run) => run.automatic_scores![key]),
      ),
    ]),
  );
  const caseIndex = Math.max(
    0,
    detail?.cases.findIndex((item) => item.case_id === selectedCaseId) ?? 0,
  );
  const selectedCase = detail?.cases[caseIndex];
  useEffect(() => {
    if (!selectedCaseId && selectedCase) setSelectedCaseId(selectedCase.case_id);
  }, [selectedCaseId, selectedCase?.case_id]);
  function update(patch: Partial<AnswerState>) {
    setState((current) => ({ ...current, ...patch }));
    setPage(0);
  }
  async function perform(label: string, action: (signal: AbortSignal) => Promise<void>) {
    if (operation.current) return;
    const controller = new AbortController();
    operation.current = controller;
    setPending(label);
    setActionError('');
    setNotice('');
    try {
      await action(controller.signal);
    } catch (cause) {
      if (!controller.signal.aborted) {
        setActionError(cause instanceof Error ? cause.message : 'The request failed.');
        // A lost response never triggers another generation or review write.
        void workspace.reload();
      }
    } finally {
      if (operation.current === controller) operation.current = null;
      if (!controller.signal.aborted) setPending('');
    }
  }
  function start(event: React.FormEvent) {
    event.preventDefault();
    if (!selected || !profile || !runtime?.available || !workspace.connected || active || pending)
      return;
    if (!architecture.trim() || new TextEncoder().encode(architecture.trim()).length > 256) {
      setActionError('Architecture label must contain 1–256 UTF-8 bytes.');
      return;
    }
    void perform('start', async (signal) => {
      const run = await api.startRun(
        {
          benchmark_id: selected.id,
          architecture_label: architecture.trim(),
          profile_id: profile.id,
        },
        signal,
      );
      if (signal.aborted) return;
      update({ query: '', status: 'all', comparison: '' });
      setNotice(
        run.evaluation === 'hotpotqa_answer_v1'
          ? `Answer run ${run.id.slice(0, 8)} started. Exact match and F1 are scored automatically.`
          : `Answer run ${run.id.slice(0, 8)} started. Review the generated answers when it finishes.`,
      );
      await workspace.reload();
    });
  }
  function open(run: AnswerRunSummary) {
    void perform('open', async (signal) => {
      const result = await api.getRun(run.id, signal);
      if (!signal.aborted) {
        setDetail(result);
        setSelectedCaseId(result.cases[0]?.case_id ?? '');
      }
    });
  }
  function download(run: AnswerRunSummary) {
    void perform('csv', async (signal) => {
      const blob = await api.downloadScores(run.id, signal);
      if (signal.aborted) return;
      saveDownload(blob, `${run.id}-answers.csv`);
    });
  }
  const Container = inspectionOnly ? 'div' : 'main';
  return (
    <Container className={styles.dashboard}>
      <section className={styles.results} aria-label="Generated answer results">
        <header className={styles.resultsHeading}>
          <h1>Architecture &amp; model comparison</h1>
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
            aria-label="Search answer runs"
            placeholder="Search benchmark, architecture, or model…"
            value={state.query}
            onChange={(event) => update({ query: event.target.value })}
          />
          <select
            aria-label="Filter answer run status"
            value={state.status}
            onChange={(event) => update({ status: event.target.value })}
          >
            <option value="all">All run states</option>
            {STATUSES.map((status) => (
              <option key={status} value={status}>
                {status}
              </option>
            ))}
          </select>
        </div>
        <div
          className={`${styles.tableScroll} ${styles.answerTable} ${showAutomatic ? styles.automaticTable : ''}`}
          role="region"
          tabIndex={0}
          aria-label="Generated answer comparison table, scroll for more results"
        >
          <table>
            <caption>
              Generated answer quality by architecture, embedding model and generation model.
              HotpotQA EM/F1 cover all cases; human review scores cover reviewed answers only.
            </caption>
            <thead>
              <tr>
                <th scope="col">Benchmark / run</th>
                <th scope="col">Architecture</th>
                <th scope="col">Model pairing</th>
                {showAutomatic &&
                  AUTOMATIC_METRICS.map(({ key, label }) => (
                    <th
                      scope="col"
                      key={key}
                      title="Automatic answer matching against the reference; denominator is all cases"
                    >
                      {label}
                    </th>
                  ))}
                <th scope="col">Status / answers</th>
                <th scope="col">Reviewed</th>
                {METRICS.map(({ key, label, lower }) => (
                  <th
                    scope="col"
                    key={key}
                    title={
                      lower
                        ? 'Lower is better; fraction of reviewed answers with hallucinations'
                        : 'Fraction of reviewed answers marked as passing'
                    }
                  >
                    {label}
                  </th>
                ))}
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((run) => (
                <tr key={run.id}>
                  <th scope="row">
                    <strong>{name(byId.get(run.request.benchmark_id))}</strong>
                    <small>
                      {run.id.slice(0, 8)} · {new Date(run.started_at_ms).toLocaleString()}
                    </small>
                  </th>
                  <td>
                    <strong>{run.request.architecture_label}</strong>
                    <small>
                      {evaluationLabel(run.evaluation)} · top k {run.top_k}
                    </small>
                  </td>
                  <td>
                    <strong>{run.generation_model.label}</strong>
                    <small>Embedding: {run.embedding_model.id}</small>
                  </td>
                  {showAutomatic &&
                    AUTOMATIC_METRICS.map(({ key }) => (
                      <td
                        className={styles.score}
                        key={key}
                        data-best={
                          (!!state.comparison &&
                            comparison.length > 1 &&
                            run.automatic_scores?.[key] === bestAutomatic[key]) ||
                          undefined
                        }
                      >
                        {percent(run.automatic_scores?.[key])}
                        {run.automatic_scores && (
                          <small>
                            {run.scored ?? 0} / {run.total} scored
                            {run.completed + run.failed < run.total ? ' · partial run' : ''}
                          </small>
                        )}
                      </td>
                    ))}
                  <td>
                    <span className={styles.badge} data-state={run.status}>
                      {run.status === 'failed' && run.completed + run.failed === run.total
                        ? 'finished with errors'
                        : run.status}
                    </span>
                    <small>
                      {run.answered} / {run.total} answered
                    </small>
                    <small>
                      {run.completed + run.failed} processed
                      {run.failed ? ` · ${run.failed} failed` : ''}
                    </small>
                    {run.status === 'running' && (
                      <progress
                        aria-label={`Answer run ${run.id.slice(0, 8)} progress`}
                        max={Math.max(1, run.total)}
                        value={run.completed + run.failed}
                      />
                    )}
                  </td>
                  <td>
                    {run.reviewed} / {run.answered}
                    <small>
                      {run.reviewed < run.answered
                        ? run.evaluation === 'hotpotqa_answer_v1'
                          ? 'Optional review'
                          : 'Review pending'
                        : run.answered
                          ? 'Reviewed answers'
                          : 'No answers to review'}
                    </small>
                  </td>
                  {METRICS.map(({ key }) => (
                    <td
                      className={styles.score}
                      key={key}
                      data-best={
                        (!!state.comparison &&
                          run.evaluation === 'manual_review_v1' &&
                          comparison.length > 1 &&
                          run.means?.[key] === best[key]) ||
                        undefined
                      }
                    >
                      {percent(run.means?.[key])}
                      {run.means &&
                        (run.evaluation === 'manual_review_v1'
                          ? !answerComparabilityKey(run)
                          : run.reviewed < run.answered || run.answered < run.total) && (
                          <small>partial coverage</small>
                        )}
                    </td>
                  ))}
                  <td>
                    <div className={styles.rowActions}>
                      <button
                        disabled={!!pending || !workspace.connected}
                        onClick={() => open(run)}
                        aria-label={`Review answers for ${run.id.slice(0, 8)}`}
                      >
                        Answers &amp; review
                      </button>
                      <button
                        disabled={!!pending || !workspace.connected || run.status === 'running'}
                        onClick={() => download(run)}
                        aria-label={`Download answer CSV for ${run.id.slice(0, 8)}`}
                      >
                        CSV
                      </button>
                      {answerComparabilityKey(run) && !state.comparison && (
                        <button
                          onClick={() =>
                            update({
                              comparison: answerComparabilityKey(run)!,
                              query: '',
                              status: 'all',
                            })
                          }
                          aria-label={`Compare answer setup for ${run.id.slice(0, 8)}`}
                        >
                          Compare setup
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {!visible.length && (
            <div className={styles.empty}>
              <h3>
                {workspace.connected
                  ? runs.length
                    ? 'No matching answer runs'
                    : 'No generated-answer runs yet'
                  : 'Connect the answer benchmark server'}
              </h3>
              <p>
                {runs.length
                  ? 'Change the search or run filters.'
                  : 'Start an architecture and model pairing below. HotpotQA scores automatically; RAGTruth uses human review.'}
              </p>
            </div>
          )}
        </div>
        <div className={styles.tableFooter}>
          <span>
            {filtered.length} of {runs.length} answer runs · auto-refresh every 3 seconds
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
              Comparing {comparison.length}{' '}
              {comparison[0]?.evaluation === 'hotpotqa_answer_v1'
                ? 'completed automatically scored'
                : 'fully answered and reviewed'}{' '}
              runs with the same snapshot, top-k, evaluator and candidate sources.
            </span>
            <button className={styles.button} onClick={() => update({ comparison: '' })}>
              Show all answer runs
            </button>
          </div>
        ) : (
          <p className={styles.tableNote}>
            {showAutomatic &&
              'HotpotQA EM/F1 score the full returned answer against the reference over all cases, including zeros for missing answers. '}
            Human review scores cover reviewed answers only. Hallucinations: lower is better. Other
            scores: higher is better. Refusals and evidence-only responses remain visible in answer
            coverage.
          </p>
        )}
      </section>
      {!inspectionOnly && (
        <BenchmarkCatalog
          api={benchmarkApi}
          snapshots={benchmarks}
          pollInterval={pollInterval}
          onOpenAnswers={(benchmarkId) => {
            update({ benchmarkId });
            runForm.current?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
          }}
        />
      )}
      {workspace.error && (
        <p className={styles.error} role="alert">
          {workspace.error}
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
      {!inspectionOnly && (
        <form
          className={`${styles.panel} ${styles.setup}`}
          aria-label="Start answer run"
          ref={runForm}
          onSubmit={start}
        >
          <div className={styles.panelHeading}>
            <h2>Run an architecture + model pairing</h2>
            <span>
              {evaluationLabel(selectedHotpot ? 'hotpotqa_answer_v1' : 'manual_review_v1')}
            </span>
          </div>
          {runtime && !runtime.available && (
            <p className={styles.notice}>
              {runtime.reason ??
                'The configured Nebula runtime does not have an available generation model.'}
            </p>
          )}
          <div className={styles.pairingFields}>
            <label>
              Benchmark snapshot
              <select
                value={selected?.id ?? ''}
                disabled={!!pending || !benchmarks.length}
                onChange={(event) => update({ benchmarkId: event.target.value })}
              >
                <option value="" disabled>
                  No prepared snapshots
                </option>
                {benchmarks.map((item) => (
                  <option key={item.id} value={item.id}>
                    {name(item)} · {item.case_count} cases · {item.id.slice(0, 8)}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Architecture label
              <input
                required
                value={architecture}
                onChange={(event) => setArchitecture(event.target.value)}
                disabled={!!pending}
                placeholder="e.g. Dense retrieval baseline"
              />
            </label>
            <label>
              Generation model
              <select
                value={profile?.id ?? ''}
                disabled={!!pending || !profiles.length}
                onChange={(event) => setProfileId(event.target.value)}
              >
                <option value="" disabled>
                  No enabled generation model
                </option>
                {profiles.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className={styles.panelHeading}>
            <p className={styles.muted}>
              Embedding: {runtime?.embedding_model?.id ?? 'Not available'}
              <small>
                {runtime?.embedding_model?.revision
                  ? `Revision ${runtime.embedding_model.revision}`
                  : ''}
              </small>
            </p>
            <button
              className={styles.primaryButton}
              disabled={
                !workspace.connected ||
                !runtime?.available ||
                !selected ||
                !profile ||
                active ||
                !!pending
              }
            >
              {pending === 'start' ? 'Starting…' : active ? 'Run in progress' : 'Generate answers'}
            </button>
          </div>
          {selectedHotpot && (
            <p className={styles.explanation}>
              HotpotQA uses each question’s supplied candidate passages (typically ten). Reference
              answers are kept out of retrieval and generation. EM/F1 score the full returned answer
              without extracting a shorter answer. Nebula’s strict evidence quotations can therefore
              score low against short references. These are subset scores, not an official
              leaderboard result.
            </p>
          )}
          <small>
            Uses the configured Nebula model and retrieval settings (top k 8). Generation may use
            your configured provider and incur its charges. Each question gets a fresh conversation.
            Requests run in batches of up to four; the next batch waits for every response. Request
            failures are recorded and the remaining questions continue.
          </small>
        </form>
      )}
      {detail && (
        <section className={styles.detail} ref={detailElement} aria-label="Answer run details">
          <div className={styles.panelHeading}>
            <h2>Answers &amp; review · {detail.id.slice(0, 8)}</h2>
            <div className={styles.actions}>
              {detail.cases.some((item) => item.status === 'error') && (
                <button
                  className={styles.button}
                  onClick={() =>
                    saveDownload(
                      new Blob([answerFailureLog(detail)], { type: 'application/x-ndjson' }),
                      `${detail.id}-failures.jsonl`,
                    )
                  }
                >
                  Download failure log
                </button>
              )}
              <button className={styles.button} onClick={() => setDetail(null)}>
                Close answers
              </button>
            </div>
          </div>
          <p>
            {detail.request.architecture_label} · {detail.generation_model.label} · {detail.status}
          </p>
          <p>
            {detail.max_in_flight === undefined || detail.max_in_flight === 1
              ? 'One request at a time'
              : `Batches of up to ${detail.max_in_flight} requests`}{' '}
            · {detail.completed + detail.failed} / {detail.total} processed · {detail.failed} failed
          </p>
          <small>
            {detail.embedding_model.id} · revision {detail.embedding_model.revision} · snapshot{' '}
            {detail.benchmark_fingerprint} · {evaluationLabel(detail.evaluation)}
          </small>
          {detail.error && <p className={styles.error}>{detail.error}</p>}
          {!selectedCase ? (
            <p>No answers have been recorded yet.</p>
          ) : (
            <>
              <div className={styles.caseNavigation}>
                <label>
                  Question
                  <select
                    value={selectedCase.case_id}
                    onChange={(event) => setSelectedCaseId(event.target.value)}
                  >
                    {detail.cases.map((item, index) => (
                      <option key={item.case_id} value={item.case_id}>
                        {index + 1}. {item.query.slice(0, 100)}
                        {item.review ? ' · reviewed' : ''}
                      </option>
                    ))}
                  </select>
                </label>
                <div className={styles.actions}>
                  <button
                    className={styles.button}
                    disabled={caseIndex === 0}
                    onClick={() => setSelectedCaseId(detail.cases[caseIndex - 1].case_id)}
                  >
                    Previous answer
                  </button>
                  <button
                    className={styles.button}
                    disabled={caseIndex >= detail.cases.length - 1}
                    onClick={() => setSelectedCaseId(detail.cases[caseIndex + 1].case_id)}
                  >
                    Next answer
                  </button>
                </div>
              </div>
              <h3>{selectedCase.query}</h3>
              <span className={styles.badge}>{selectedCase.outcome}</span>
              {selectedCase.answer && (
                <blockquote className={styles.answerText}>{selectedCase.answer}</blockquote>
              )}
              {selectedCase.reference_answer !== undefined && (
                <section
                  className={styles.referenceAnswer}
                  aria-label="Automatic answer assessment"
                >
                  <h3>Reference answer</h3>
                  <p>{selectedCase.reference_answer}</p>
                  <p>
                    Exact match: {percent(selectedCase.automatic_scores?.exact_match)} · Token F1:{' '}
                    {percent(selectedCase.automatic_scores?.f1)}
                  </p>
                  <small>
                    Compared with the full returned answer. Human review below is separate.
                  </small>
                </section>
              )}
              {selectedCase.reason && <p>{selectedCase.reason}</p>}
              {selectedCase.error && <p className={styles.error}>{selectedCase.error}</p>}
              {selectedCase.failure ? (
                <section className={styles.referenceAnswer} aria-label="Failure details">
                  <h3>Failure details</h3>
                  <p>
                    {new Date(selectedCase.failure.timestamp_ms).toLocaleString()} ·{' '}
                    {selectedCase.failure.operation.replaceAll('_', ' ')} ·{' '}
                    {selectedCase.failure.http_status
                      ? `HTTP ${selectedCase.failure.http_status} · `
                      : ''}
                    {selectedCase.failure.code}
                  </p>
                  {selectedCase.failure.provider_diagnostic && (
                    <>
                      <p>
                        Provider: {selectedCase.failure.provider_diagnostic.category}
                        {selectedCase.failure.provider_diagnostic.upstreamStatus
                          ? ` · HTTP ${selectedCase.failure.provider_diagnostic.upstreamStatus}`
                          : ''}
                        {selectedCase.failure.provider_diagnostic.upstreamCode
                          ? ` · ${selectedCase.failure.provider_diagnostic.upstreamCode}`
                          : ''}
                      </p>
                      <p>
                        Request:{' '}
                        {selectedCase.failure.provider_diagnostic.requestBytes.toLocaleString()}{' '}
                        bytes · Output limit:{' '}
                        {selectedCase.failure.provider_diagnostic.maxOutputTokens.toLocaleString()}{' '}
                        tokens · Attempt {selectedCase.failure.provider_diagnostic.attempt}
                      </p>
                      {selectedCase.failure.provider_diagnostic.responseTruncated && (
                        <small>Provider error response exceeded the diagnostic read limit.</small>
                      )}
                    </>
                  )}
                  <small>The failure log records metadata without prompts or credentials.</small>
                </section>
              ) : selectedCase.status === 'error' ? (
                <small>This older run did not record detailed provider diagnostics.</small>
              ) : null}
              {selectedCase.model_receipt && (
                <small>
                  Model receipt: {selectedCase.model_receipt.modelLabel} ·{' '}
                  {selectedCase.model_receipt.route}
                </small>
              )}
              <details className={styles.evidence}>
                <summary>Evidence and citations ({selectedCase.evidence.length} passages)</summary>
                {selectedCase.lineage.map((claim) => (
                  <p key={claim.id}>
                    <strong>{claim.claim}</strong>
                    <small>Cites {claim.evidenceIds.join(', ')}</small>
                  </p>
                ))}
                {selectedCase.evidence.map((item) => (
                  <article key={item.id}>
                    <strong>
                      [{item.ordinal}] {item.sourceTitle}
                    </strong>
                    <small>
                      {item.id} · {item.location}
                    </small>
                    <p>{item.excerpt}</p>
                  </article>
                ))}
              </details>
              {selectedCase.outcome === 'answered' && selectedCase.status === 'ok' ? (
                <AnswerReviewForm
                  key={`${detail.id}-${selectedCase.case_id}-${selectedCase.review?.reviewed_at_ms ?? 'new'}`}
                  item={selectedCase}
                  disabled={!!pending || !workspace.connected || detail.status === 'running'}
                  onSave={(body) =>
                    void perform('review', async (signal) => {
                      const updated = await api.saveReview(
                        detail.id,
                        selectedCase.case_id,
                        body,
                        signal,
                      );
                      if (signal.aborted) return;
                      setDetail((current) => (current?.id === updated.id ? updated : current));
                      setNotice('Review saved. Scores cover reviewed answers only.');
                      await workspace.reload();
                    })
                  }
                />
              ) : (
                <p className={styles.explanation}>
                  This case produced no generated answer to review. It remains part of the run’s
                  coverage;{' '}
                  {detail.evaluation === 'hotpotqa_answer_v1'
                    ? 'automatic answer scores are zero.'
                    : 'no quality scores are assigned.'}
                </p>
              )}
              {detail.status === 'running' && (
                <p className={styles.explanation}>
                  Review becomes available when generation stops.
                </p>
              )}
            </>
          )}
        </section>
      )}
      <footer className={styles.footer}>
        Benchmark definitions stay in YAML. HotpotQA uses reference-answer matching; RAGTruth’s
        historical annotations are not reused as scores for newly generated answers.
      </footer>
    </Container>
  );
}

function AnswerReviewForm({
  item,
  disabled,
  onSave,
}: {
  item: AnswerCase;
  disabled: boolean;
  onSave: (body: AnswerReviewRequest) => void;
}) {
  const [reviewer, setReviewer] = useState(item.review?.reviewer ?? '');
  const [notes, setNotes] = useState(item.review?.notes ?? '');
  const [error, setError] = useState('');
  const fields = [
    { key: 'correctness', label: 'Correctness', yes: 'Correct', no: 'Incorrect' },
    {
      key: 'groundedness',
      label: 'Groundedness',
      yes: 'Fully supported',
      no: 'Unsupported claims',
    },
    { key: 'hallucination', label: 'Hallucination', yes: 'Present', no: 'None found' },
    {
      key: 'citation_accuracy',
      label: 'Citation accuracy',
      yes: 'Accurate and sufficient',
      no: 'Incorrect or missing',
    },
  ] as const;
  type Flag = (typeof fields)[number]['key'];
  const [flags, setFlags] = useState<Record<Flag, string>>(
    () =>
      Object.fromEntries(
        fields.map(({ key }) => [key, item.review ? String(item.review[key]) : '']),
      ) as Record<Flag, string>,
  );
  function submit(event: React.FormEvent) {
    event.preventDefault();
    if (disabled) return;
    const bytes = new TextEncoder();
    if (
      !reviewer.trim() ||
      bytes.encode(reviewer.trim()).length > 128 ||
      bytes.encode(notes).length > 4000
    ) {
      setError('Enter a reviewer (up to 128 UTF-8 bytes) and notes of up to 4000 UTF-8 bytes.');
      return;
    }
    if (fields.some(({ key }) => !flags[key])) {
      setError('Review all four quality criteria before saving.');
      return;
    }
    setError('');
    onSave({
      reviewer: reviewer.trim(),
      notes,
      correctness: flags.correctness === 'true',
      groundedness: flags.groundedness === 'true',
      hallucination: flags.hallucination === 'true',
      citation_accuracy: flags.citation_accuracy === 'true',
    });
  }
  return (
    <form className={styles.reviewForm} aria-label="Review generated answer" onSubmit={submit}>
      <h3>Human review</h3>
      <p className={styles.muted}>
        Read the question, generated answer and evidence. Leave the answer unreviewed if you cannot
        assess it.
      </p>
      {error && (
        <p role="alert" className={styles.error}>
          {error}
        </p>
      )}
      <fieldset disabled={disabled}>
        <legend className={styles.visuallyHidden}>Quality assessment</legend>
        <div className={styles.reviewFields}>
          {fields.map(({ key, label, yes, no }) => (
            <label key={key}>
              {label}
              <select
                required
                value={flags[key]}
                onChange={(event) => setFlags({ ...flags, [key]: event.target.value })}
              >
                <option value="">Choose a judgement</option>
                <option value="true">{yes}</option>
                <option value="false">{no}</option>
              </select>
            </label>
          ))}
        </div>
        <label>
          Reviewer
          <input
            required
            value={reviewer}
            onChange={(event) => setReviewer(event.target.value)}
            placeholder="Name or team handle"
          />
        </label>
        <label>
          Review notes
          <textarea value={notes} rows={3} onChange={(event) => setNotes(event.target.value)} />
        </label>
        <button className={styles.primaryButton}>
          {item.review ? 'Update review' : 'Save review'}
        </button>
      </fieldset>
      {item.review && (
        <small>
          Last reviewed by {item.review.reviewer} ·{' '}
          {new Date(item.review.reviewed_at_ms).toLocaleString()}
        </small>
      )}
    </form>
  );
}
