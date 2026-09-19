import React, { useEffect, useRef, useState } from 'react';
import {
  SAMPLE_REPORT,
  parseBenchmarkReport,
  formatResult,
  exportComparisonCsv,
  type BenchmarkReport,
  type BenchmarkDefinition,
  type BenchmarkResult,
} from './benchmark-data';
import styles from './benchmark-dashboard.module.css';

interface DashboardState {
  report: BenchmarkReport;
  query: string;
  suite: string;
  status: string;
  architectureIds: string[];
  highlightBest: boolean;
}

export interface BenchmarkDashboardProps {
  initialState?: unknown;
  onStateChange?: (state: unknown) => void;
}

const STATUS_LABELS: Record<BenchmarkResult['status'], string> = {
  completed: 'Completed',
  running: 'Running',
  queued: 'Queued',
  failed: 'Failed',
};

function initialDashboardState(saved?: unknown): DashboardState {
  const defaults = {
    report: SAMPLE_REPORT,
    query: '',
    suite: 'all',
    status: 'all',
    architectureIds: SAMPLE_REPORT.architectures.map((item) => item.id),
    highlightBest: true,
  };
  if (!saved || typeof saved !== 'object') return defaults;
  const candidate = saved as Partial<DashboardState>;
  try {
    const report = parseBenchmarkReport(JSON.stringify(candidate.report));
    report.sample = candidate.report?.sample === true;
    const architectureIds = report.architectures
      .filter((item) => candidate.architectureIds?.includes(item.id))
      .map((item) => item.id);
    return {
      report,
      query: typeof candidate.query === 'string' ? candidate.query : '',
      suite: report.benchmarks.some((item) => item.suite === candidate.suite)
        ? candidate.suite!
        : 'all',
      status: ['completed', 'running', 'queued', 'failed', 'missing'].includes(
        candidate.status ?? '',
      )
        ? candidate.status!
        : 'all',
      architectureIds: architectureIds.length
        ? architectureIds
        : report.architectures.map((item) => item.id),
      highlightBest: candidate.highlightBest !== false,
    };
  } catch {
    return defaults;
  }
}

function Icon({ name }: { name: 'upload' | 'download' | 'search' | 'grid' | 'close' }) {
  const paths = {
    upload: 'M12 16V4m-4 4 4-4 4 4M4 16v4h16v-4',
    download: 'M12 4v12m-4-4 4 4 4-4M4 16v4h16v-4',
    search: 'm16 16 4 4M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0',
    grid: 'M3 3h18v18H3zM3 9h18M3 15h18M9 3v18',
    close: 'm6 6 12 12M6 18 18 6',
  };
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      aria-hidden="true"
    >
      <path d={paths[name]} />
    </svg>
  );
}

function ResultDetail({
  benchmark,
  architecture,
  result,
  onClose,
}: {
  benchmark: BenchmarkDefinition;
  architecture: string;
  result?: BenchmarkResult;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  return (
    <dialog
      ref={dialog}
      className={styles.dialog}
      onCancel={onClose}
      aria-labelledby="result-detail-title"
    >
      <div className={styles.dialogHeader}>
        <span className={styles.eyebrow}>Result details</span>
        <button onClick={onClose} aria-label="Close result details">
          <Icon name="close" />
        </button>
      </div>
      <h2 id="result-detail-title">{benchmark.name}</h2>
      <p className={styles.muted}>
        {architecture} · {benchmark.suite}
      </p>
      <div className={styles.detailValue}>
        {result?.status === 'completed'
          ? formatResult(result.value!, benchmark.unit)
          : result
            ? STATUS_LABELS[result.status]
            : 'Not run'}
      </div>
      <dl className={styles.detailList}>
        <div>
          <dt>Metric</dt>
          <dd>{benchmark.metric}</dd>
        </div>
        <div>
          <dt>Scoring</dt>
          <dd>{benchmark.direction === 'higher' ? 'Higher is better' : 'Lower is better'}</dd>
        </div>
        <div>
          <dt>Status</dt>
          <dd>{result ? STATUS_LABELS[result.status] : 'Not run'}</dd>
        </div>
      </dl>
      <p className={styles.detailNote}>{result?.note || 'No additional notes in this report.'}</p>
    </dialog>
  );
}

export function BenchmarkDashboard({ initialState, onStateChange }: BenchmarkDashboardProps = {}) {
  const [state, setState] = useState(() => initialDashboardState(initialState));
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [importing, setImporting] = useState(false);
  const [page, setPage] = useState(1);
  const [selection, setSelection] = useState<{
    benchmarkId: string;
    architectureId: string;
  } | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const { report, query, suite, status, architectureIds, highlightBest } = state;
  useEffect(() => {
    onStateChange?.(state);
  }, [state, onStateChange]);
  const update = (patch: Partial<DashboardState>) =>
    setState((previous) => ({ ...previous, ...patch }));
  const architectures = report.architectures.filter((item) => architectureIds.includes(item.id));
  const cells = new Map(
    report.results.map((result) => [
      JSON.stringify([result.benchmarkId, result.architectureId]),
      result,
    ]),
  );
  const resultFor = (benchmarkId: string, architectureId: string) =>
    cells.get(JSON.stringify([benchmarkId, architectureId]));
  const benchmarks = report.benchmarks.filter((benchmark) => {
    if (suite !== 'all' && benchmark.suite !== suite) return false;
    if (
      !`${benchmark.name} ${benchmark.suite} ${benchmark.metric}`
        .toLowerCase()
        .includes(query.toLowerCase().trim())
    )
      return false;
    return (
      status === 'all' ||
      architectures.some((architecture) => {
        const result = resultFor(benchmark.id, architecture.id);
        return status === 'missing' ? !result : result?.status === status;
      })
    );
  });
  const pageCount = Math.max(1, Math.ceil(benchmarks.length / 25));
  const activePage = Math.min(page, pageCount);
  const visibleBenchmarks = benchmarks.slice((activePage - 1) * 25, activePage * 25);
  useEffect(() => {
    setPage(1);
  }, [query, suite, status, architectureIds, report]);
  const total = report.benchmarks.length * report.architectures.length;
  const completed = report.results.filter((result) => result.status === 'completed').length;
  const running = report.results.filter((result) => result.status === 'running').length;
  const queued = report.results.filter((result) => result.status === 'queued').length;
  const failed = report.results.filter((result) => result.status === 'failed').length;
  const unstarted = total - report.results.length;
  const completion = Math.round((completed / total) * 100);
  const suites = [...new Set(report.benchmarks.map((benchmark) => benchmark.suite))];

  function bestValue(benchmark: BenchmarkDefinition): number | null {
    const values = architectures
      .map((architecture) => resultFor(benchmark.id, architecture.id))
      .filter((result): result is BenchmarkResult => result?.status === 'completed')
      .map((result) => result.value!);
    return values.length
      ? benchmark.direction === 'higher'
        ? Math.max(...values)
        : Math.min(...values)
      : null;
  }

  async function importReport(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    setImporting(true);
    setError('');
    setNotice('');
    try {
      if (file.size > 8 * 1024 * 1024)
        throw new Error('Choose a JSON results file smaller than 8 MB.');
      const imported = parseBenchmarkReport(await file.text());
      update({
        report: imported,
        query: '',
        suite: 'all',
        status: 'all',
        architectureIds: imported.architectures.map((architecture) => architecture.id),
      });
      setSelection(null);
      setNotice(
        `Imported ${imported.benchmarks.length} benchmarks across ${imported.architectures.length} architectures.`,
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Could not read this report.');
    } finally {
      setImporting(false);
    }
  }

  function exportResults() {
    const url = URL.createObjectURL(
      new Blob(
        [
          exportComparisonCsv(
            report,
            benchmarks,
            architectures.map(({ id }) => id),
          ),
        ],
        {
          type: 'text/csv;charset=utf-8;',
        },
      ),
    );
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = 'architecture-comparison.csv';
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    setNotice(`Exported ${benchmarks.length} rows across ${architectures.length} architectures.`);
  }

  return (
    <main className={styles.dashboard}>
      <div className={styles.breadcrumb}>
        <span>
          <Icon name="grid" /> Benchmarking <span className={styles.slash}>/</span> Comparison
          workspace
        </span>
        <span className={styles.prototypeTag}>Prototype</span>
      </div>
      <header className={styles.header}>
        <div>
          <div className={styles.eyebrow}>Benchmark workspace</div>
          <h1>Architecture comparison</h1>
          <p>Every benchmark. Every architecture. One view.</p>
        </div>
        <div className={styles.actions}>
          <button className={styles.button} onClick={exportResults} disabled={!benchmarks.length}>
            <Icon name="download" /> Export CSV
          </button>
          <button
            className={styles.primaryButton}
            disabled={importing}
            onClick={() => fileInput.current?.click()}
          >
            <Icon name="upload" /> {importing ? 'Importing…' : 'Import results'}
          </button>
          <input
            ref={fileInput}
            className={styles.hiddenInput}
            type="file"
            accept=".json,application/json"
            aria-label="Import benchmark results"
            onChange={importReport}
          />
        </div>
      </header>

      <section className={styles.batchPanel} aria-label="Batch summary">
        <div className={styles.batchIdentity}>
          <span className={styles.eyebrow}>Current batch</span>
          <h2>{report.name}</h2>
          <span className={styles.sourceTag}>
            {report.sample ? 'Sample data' : 'Imported results'}
          </span>
        </div>
        <div className={styles.batchStat}>
          <strong>{report.benchmarks.length}</strong>
          <span>Benchmarks</span>
        </div>
        <div className={styles.batchStat}>
          <strong>{report.architectures.length}</strong>
          <span>Architectures</span>
        </div>
        <div className={styles.batchProgress}>
          <div>
            <span>
              <strong>{completed}</strong> / {total} completed
            </span>
            <span>{completion}%</span>
          </div>
          <progress max={total} value={completed} aria-label="Completed benchmark results" />
          <div className={styles.progressLegend}>
            <span>
              <i className={styles.runningDot} />
              {running} running
            </span>
            <span>
              <i className={styles.queuedDot} />
              {queued} queued
            </span>
            <span>
              <i className={styles.failedDot} />
              {failed} failed
            </span>
            {unstarted > 0 && <span>{unstarted} not run</span>}
          </div>
        </div>
      </section>

      <div className={styles.messages}>
        {error ? (
          <p role="alert" className={styles.error}>
            {error}
          </p>
        ) : notice ? (
          <p role="status">{notice}</p>
        ) : (
          <p>
            {report.sample
              ? 'Illustrative scores and run states. Import a batch report to compare your own architectures.'
              : 'Results from the imported report. Import an updated report to refresh run states.'}
          </p>
        )}
      </div>

      <section className={styles.resultsPanel} aria-labelledby="comparison-heading">
        <div className={styles.panelHeading}>
          <div>
            <h2 id="comparison-heading">Benchmark results</h2>
            <span className={styles.resultCount}>{benchmarks.length} rows</span>
          </div>
          <label className={styles.highlightToggle}>
            <input
              type="checkbox"
              checked={highlightBest}
              onChange={(event) => update({ highlightBest: event.target.checked })}
            />{' '}
            Highlight best in each row
          </label>
        </div>
        <div className={styles.toolbar}>
          <label className={styles.search}>
            <Icon name="search" />
            <input
              value={query}
              onChange={(event) => update({ query: event.target.value })}
              placeholder="Search benchmarks or metrics…"
              aria-label="Search benchmarks"
            />
          </label>
          <select
            aria-label="Filter benchmark suite"
            value={suite}
            onChange={(event) => update({ suite: event.target.value })}
          >
            <option value="all">All suites</option>
            {suites.map((item) => (
              <option key={item} value={item}>
                {item}
              </option>
            ))}
          </select>
          <select
            aria-label="Filter result status"
            value={status}
            onChange={(event) => update({ status: event.target.value })}
          >
            <option value="all">All run states</option>
            <option value="completed">With completed results</option>
            <option value="running">With running results</option>
            <option value="queued">With queued results</option>
            <option value="failed">With failed results</option>
            <option value="missing">With unstarted results</option>
          </select>
        </div>
        <fieldset className={styles.architectureFilters}>
          <legend>Architectures</legend>
          {report.architectures.map((architecture) => (
            <label key={architecture.id}>
              <input
                type="checkbox"
                checked={architectureIds.includes(architecture.id)}
                disabled={architectureIds.length === 1 && architectureIds.includes(architecture.id)}
                onChange={(event) =>
                  update({
                    architectureIds: event.target.checked
                      ? [...architectureIds, architecture.id]
                      : architectureIds.filter((id) => id !== architecture.id),
                  })
                }
              />
              <span>{architecture.name}</span>
            </label>
          ))}
        </fieldset>

        <div
          className={styles.tableScroll}
          tabIndex={0}
          role="region"
          aria-label="Architecture comparison table, scroll for more results"
        >
          <table className={styles.table}>
            <caption className={styles.srOnly}>
              Benchmark scores by architecture. Each row declares its metric and whether higher or
              lower is better. Highlighted values are best among completed results in the visible
              architectures.
            </caption>
            <thead>
              <tr>
                <th scope="col" className={styles.benchmarkColumn}>
                  <span>Benchmark</span>
                  <small>Suite / evaluation metric</small>
                </th>
                {architectures.map((architecture, index) => (
                  <th scope="col" key={architecture.id}>
                    <div className={styles.architectureHeading}>
                      <span className={styles.architectureNumber}>
                        {String(index + 1).padStart(2, '0')}
                      </span>
                      <span>{architecture.name}</span>
                    </div>
                    <small title={architecture.description}>{architecture.description}</small>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {visibleBenchmarks.map((benchmark, index) => {
                const best = bestValue(benchmark);
                return (
                  <tr key={benchmark.id}>
                    <th scope="row" className={styles.benchmarkColumn}>
                      <div className={styles.benchmarkName}>
                        <span className={styles.rowNumber}>
                          {String((activePage - 1) * 25 + index + 1).padStart(2, '0')}
                        </span>
                        <div>
                          {benchmark.name}
                          <small>
                            {benchmark.suite} · {benchmark.metric}
                          </small>
                        </div>
                      </div>
                      <span className={styles.direction}>
                        {benchmark.direction === 'higher' ? 'Higher is better' : 'Lower is better'}
                      </span>
                    </th>
                    {architectures.map((architecture) => {
                      const result = resultFor(benchmark.id, architecture.id);
                      const isBest =
                        highlightBest && result?.status === 'completed' && result.value === best;
                      const label =
                        result?.status === 'completed'
                          ? formatResult(result.value!, benchmark.unit)
                          : result
                            ? STATUS_LABELS[result.status]
                            : 'Not run';
                      return (
                        <td key={architecture.id} className={isBest ? styles.bestCell : ''}>
                          <button
                            className={styles.resultButton}
                            onClick={() =>
                              setSelection({
                                benchmarkId: benchmark.id,
                                architectureId: architecture.id,
                              })
                            }
                            aria-label={`${benchmark.name}, ${architecture.name}: ${label}${isBest ? ', best result' : ''}`}
                          >
                            <span
                              className={
                                result?.status === 'completed'
                                  ? styles.score
                                  : `${styles.cellStatus} ${styles[result?.status ?? 'missing']}`
                              }
                            >
                              {result?.status !== 'completed' && <i />}
                              {label}
                            </span>
                            {isBest ? (
                              <span className={styles.bestLabel}>Best</span>
                            ) : result?.status === 'failed' ? (
                              <span className={styles.cellHint}>View error</span>
                            ) : null}
                          </button>
                        </td>
                      );
                    })}
                  </tr>
                );
              })}
            </tbody>
          </table>
          {!benchmarks.length && (
            <div className={styles.emptyState}>
              <h3>No matching benchmarks</h3>
              <p>Change the search, suite, or run-state filter.</p>
              <button
                className={styles.button}
                onClick={() => update({ query: '', suite: 'all', status: 'all' })}
              >
                Clear filters
              </button>
            </div>
          )}
        </div>
        <div className={styles.tableFooter}>
          <span>
            {benchmarks.length} of {report.benchmarks.length} benchmarks · {architectures.length} of{' '}
            {report.architectures.length} architectures
          </span>
          <span>
            <i className={styles.bestKey} /> Best completed score per row · Ties included
          </span>
          {pageCount > 1 && (
            <nav className={styles.pagination} aria-label="Benchmark pages">
              <button disabled={activePage === 1} onClick={() => setPage(activePage - 1)}>
                Previous
              </button>
              <span>
                Page {activePage} of {pageCount}
              </span>
              <button disabled={activePage === pageCount} onClick={() => setPage(activePage + 1)}>
                Next
              </button>
            </nav>
          )}
        </div>
      </section>
      <footer className={styles.footer}>
        <span>
          Results are compared within each benchmark. Scores from different metrics are not
          averaged.
        </span>
        <span>
          {report.sample ? 'Sample batch' : 'Report snapshot'} · No benchmark runner connected
        </span>
      </footer>
      {selection && (
        <ResultDetail
          key={`${selection.benchmarkId}:${selection.architectureId}`}
          benchmark={report.benchmarks.find((item) => item.id === selection.benchmarkId)!}
          architecture={
            report.architectures.find((item) => item.id === selection.architectureId)!.name
          }
          result={resultFor(selection.benchmarkId, selection.architectureId)}
          onClose={() => setSelection(null)}
        />
      )}
    </main>
  );
}
