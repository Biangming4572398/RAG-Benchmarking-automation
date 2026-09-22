// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import React from 'react';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RetrievalDashboard as BenchmarkDashboard } from './retrieval-dashboard';
import {
  ApiError,
  type BenchmarkApi,
  type BenchmarkInfo,
  type BenchmarkRun,
  type Catalog,
} from './benchmark-api';

const catalog: Catalog = {
  benchmarks: {
    'ragtruth-qa': {
      name: 'RAGTruth QA',
      source: 'fixture.parquet',
      split: 'test',
      adapter: 'ragtruth_qa',
      evaluation: 'paired_context_recovery_v1',
      defaults: { limit: 100, top_k: 8 },
    },
  },
};
const benchmark: BenchmarkInfo = {
  id: 'snapshot-a',
  source: 'fixture.parquet',
  split: 'test',
  metric_kind: 'paired_context_recovery_v1',
  case_count: 10,
  document_count: 3,
  corpus_path: '/experiment/corpus',
  fingerprint: 'fingerprint-a',
  configuration: {
    key: 'ragtruth-qa',
    definition: { ...catalog.benchmarks['ragtruth-qa'], defaults: { limit: 10, top_k: 4 } },
  },
};
function run(overrides: Partial<BenchmarkRun> = {}): BenchmarkRun {
  return {
    id: 'run-first',
    request: { benchmark_id: benchmark.id, top_k: 4, label: 'Dense baseline' },
    metric_kind: benchmark.metric_kind,
    benchmark_fingerprint: benchmark.fingerprint,
    status: 'completed',
    started_at_ms: 1000,
    finished_at_ms: 2000,
    total: 10,
    completed: 10,
    failed: 0,
    means: { context_hit_at_k: 0.8, reciprocal_rank_at_k: 0.5, ndcg_at_k: 0.6 },
    error: null,
    scope: { corpusId: 'corpus-a' },
    watermark: { generation: 1 },
    source_ids: ['source-a', 'source-b'],
    ...overrides,
  };
}
function apiWith(runs: BenchmarkRun[] = []) {
  return {
    getCatalog: vi.fn().mockResolvedValue(catalog),
    listBenchmarks: vi.fn().mockResolvedValue([benchmark]),
    listRuns: vi.fn().mockResolvedValue(runs),
    getBenchmark: vi.fn().mockResolvedValue({
      id: benchmark.id,
      source: benchmark.source,
      split: 'test',
      metric_kind: benchmark.metric_kind,
      cases: [
        {
          id: 'case-a',
          query: 'Where is the evidence?',
          document_id: 'doc-a',
          reference_outputs: [],
        },
      ],
      documents: [],
    }),
    getRun: vi.fn().mockImplementation(async (id: string) => runs.find((item) => item.id === id)),
    loadBenchmark: vi.fn().mockResolvedValue(benchmark),
    startRun: vi.fn().mockResolvedValue(run({ status: 'running', completed: 0, means: null })),
    downloadScores: vi.fn().mockResolvedValue(new Blob(['scores'], { type: 'text/csv' })),
  } satisfies BenchmarkApi;
}
async function connected() {
  expect(await screen.findByText('Connected')).toBeVisible();
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('Live benchmarking dashboard', () => {
  it('loads real backend records and compares only completed runs with matching settings', async () => {
    const candidate = run({
      id: 'run-second',
      request: { ...run().request, label: 'Hybrid retrieval' },
      means: { context_hit_at_k: 0.9, reciprocal_rank_at_k: 0.7, ndcg_at_k: 0.8 },
    });
    const partial = run({
      id: 'run-partial',
      status: 'failed',
      completed: 3,
      failed: 1,
      means: { context_hit_at_k: 1, reciprocal_rank_at_k: 1, ndcg_at_k: 1 },
      error: 'Index changed',
    });
    const different = run({ id: 'run-other', request: { ...run().request, top_k: 8 } });
    const api = apiWith([run(), candidate, partial, different]);
    const user = userEvent.setup();
    render(<BenchmarkDashboard api={api} />);
    await connected();
    const table = within(screen.getByRole('table', { name: /Benchmark results by architecture/ }));
    expect(table.getByText('Hybrid retrieval')).toBeVisible();
    expect(table.getByText('0.900')).toBeVisible();
    expect(table.getByText('Partial results')).toBeVisible();
    expect(
      screen.queryByRole('button', { name: 'Compare setup for run-part' }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Compare setup for run-firs' }));
    expect(table.getAllByRole('row')).toHaveLength(3);
    expect(table.getByText('0.900')).toHaveAttribute('data-best', 'true');
    const baseline = within(table.getByRole('row', { name: /Dense baseline/ }));
    expect(baseline.getByText('0.800')).not.toHaveAttribute('data-best');
    expect(table.queryByText('Partial results')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Show all runs' }));
    expect(table.getAllByRole('row')).toHaveLength(5);
  });

  it('keeps saved results and run setup available if only the catalog request fails', async () => {
    const api = apiWith([run()]);
    api.getCatalog.mockRejectedValue(new Error('Catalog unavailable'));
    render(<BenchmarkDashboard api={api} />);
    await connected();
    expect(
      screen.getByRole('table', { name: /Benchmark results by architecture/ }),
    ).toHaveTextContent('Dense baseline');
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Could not refresh the benchmark catalog',
    );
    expect(screen.getByRole('button', { name: 'Start run' })).toBeEnabled();
    expect(api.loadBenchmark).not.toHaveBeenCalled();
  });

  it('uses saved RAGTruth defaults without managing benchmark definitions or loading snapshots', async () => {
    const user = userEvent.setup();
    const api = apiWith();
    render(<BenchmarkDashboard api={api} />);
    await connected();
    expect(screen.getByLabelText('Top k')).toHaveAttribute('placeholder', '4');
    expect(screen.queryByRole('form', { name: 'Load benchmark' })).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Catalog benchmark')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Case limit')).not.toBeInTheDocument();
    expect(screen.getByRole('main').firstElementChild).toBe(
      screen.getByRole('region', { name: 'Benchmark results' }),
    );
    expect(api.getCatalog).toHaveBeenCalled();
    expect(api.loadBenchmark).not.toHaveBeenCalled();
    expect(screen.getByLabelText('Architecture label')).toHaveValue('baseline');
    await user.clear(screen.getByLabelText('Architecture label'));
    await user.type(screen.getByLabelText('Architecture label'), 'Hybrid retrieval');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() =>
      expect(api.startRun).toHaveBeenCalledWith(
        { benchmark_id: benchmark.id, label: 'Hybrid retrieval' },
        expect.any(AbortSignal),
      ),
    );
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('started'));
    expect(api.startRun).toHaveBeenCalledTimes(1);
  });

  it('keeps generation-only HotpotQA snapshots out of retrieval, including a saved HotpotQA selection', async () => {
    const hotpot: BenchmarkInfo = {
      ...benchmark,
      id: 'snapshot-hotpot',
      metric_kind: 'hotpotqa_answer_v1',
      configuration: {
        key: 'hotpotqa',
        definition: {
          ...catalog.benchmarks['ragtruth-qa'],
          name: 'HotpotQA',
          adapter: 'hotpotqa',
          evaluation: 'hotpotqa_answer_v1',
        },
      },
    };
    const api = apiWith();
    api.listBenchmarks.mockResolvedValue([hotpot, benchmark]);
    const user = userEvent.setup();
    render(<BenchmarkDashboard api={api} initialState={{ benchmarkId: hotpot.id }} />);
    await connected();
    const snapshots = screen.getByLabelText('Loaded snapshot');
    expect(snapshots).toHaveValue(benchmark.id);
    expect(within(snapshots).getByRole('option', { name: /RAGTruth QA/ })).toBeInTheDocument();
    expect(within(snapshots).queryByRole('option', { name: /HotpotQA/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() =>
      expect(api.startRun).toHaveBeenCalledWith(
        { benchmark_id: benchmark.id, label: 'baseline' },
        expect.any(AbortSignal),
      ),
    );
  });

  it('leaves preparation to the YAML workflow when no saved snapshot exists', async () => {
    const api = apiWith();
    api.listBenchmarks.mockResolvedValue([]);
    render(<BenchmarkDashboard api={api} />);
    await connected();
    expect(screen.getByRole('button', { name: 'Start run' })).toBeDisabled();
    expect(screen.getByText(/No prepared snapshots yet/)).toHaveTextContent('RAGTruth QA');
    expect(api.loadBenchmark).not.toHaveBeenCalled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('reads future benchmark names from saved YAML configuration without frontend registration', async () => {
    const api = apiWith([run({ request: { ...run().request, benchmark_id: 'snapshot-future' } })]);
    api.listBenchmarks.mockResolvedValue([
      {
        ...benchmark,
        id: 'snapshot-future',
        configuration: {
          key: 'team-evaluation',
          definition: { ...catalog.benchmarks['ragtruth-qa'], name: 'Team evaluation' },
        },
      },
      benchmark,
    ]);
    render(<BenchmarkDashboard api={api} />);
    await connected();
    expect(
      within(screen.getByRole('table', { name: /Benchmark results by architecture/ })).getByText(
        'Team evaluation',
      ),
    ).toBeVisible();
    expect(screen.getByLabelText('Loaded snapshot')).toHaveValue(benchmark.id);
    expect(screen.getByRole('option', { name: /Team evaluation/ })).toBeInTheDocument();
    expect(api.getCatalog).toHaveBeenCalled();
  });

  it('shows active progress, updates it by polling, and aborts requests when closed', async () => {
    vi.useFakeTimers();
    const active = run({ status: 'running', completed: 2, finished_at_ms: null });
    const api = apiWith([active]);
    const view = render(<BenchmarkDashboard api={api} pollInterval={3000} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Run in progress' })).toBeDisabled();
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '2');
    expect(screen.getByRole('button', { name: 'Download CSV for run-firs' })).toBeDisabled();
    api.listRuns.mockResolvedValue([run()]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(screen.getByRole('button', { name: 'Start run' })).toBeEnabled();
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
    expect(api.listRuns).toHaveBeenCalledTimes(2);
    const signal = api.listRuns.mock.calls.at(-1)?.[0] as AbortSignal;
    view.unmount();
    expect(signal.aborted).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9000);
    });
    expect(api.listRuns).toHaveBeenCalledTimes(2);
  });

  it('reports disconnected data honestly and preserves previous results after a refresh failure', async () => {
    const user = userEvent.setup();
    const api = apiWith([run()]);
    api.listBenchmarks.mockRejectedValueOnce(new ApiError('Benchmark server unavailable', 503));
    render(<BenchmarkDashboard api={api} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Benchmark server unavailable');
    expect(screen.getByText('Disconnected')).toBeVisible();
    expect(screen.queryByText('Sample data')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start run' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await connected();
    expect(
      screen.getByRole('table', { name: /Benchmark results by architecture/ }),
    ).toHaveTextContent('Dense baseline');
    api.listRuns.mockRejectedValueOnce(new Error('Server unavailable'));
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Previously loaded results remain visible',
    );
    expect(
      screen.getByRole('table', { name: /Benchmark results by architecture/ }),
    ).toHaveTextContent('Dense baseline');
    expect(screen.getByRole('button', { name: 'Start run' })).toBeDisabled();
  });

  it('keeps failed and interrupted scores partial, opens details, and handles missing CSV', async () => {
    const user = userEvent.setup();
    const failed = run({
      status: 'failed',
      completed: 0,
      failed: 1,
      means: null,
      error: 'Nebula is unavailable',
    });
    const interrupted = run({
      id: 'run-stopped',
      status: 'interrupted',
      completed: 2,
      means: { context_hit_at_k: 0, reciprocal_rank_at_k: 0, ndcg_at_k: 0 },
    });
    const api = apiWith([failed, interrupted]);
    api.downloadScores.mockRejectedValueOnce(new ApiError('This run produced no CSV rows', 404));
    render(<BenchmarkDashboard api={api} />);
    await connected();
    expect(
      within(screen.getByRole('table', { name: /Benchmark results by architecture/ })).getAllByText(
        '0.000',
      ),
    ).toHaveLength(3);
    const downloadButton = screen.getByRole('button', { name: 'Download CSV for run-firs' });
    expect(downloadButton).toBeEnabled();
    await user.click(downloadButton);
    expect(await screen.findByRole('alert')).toHaveTextContent('This run produced no CSV rows');
    await user.click(screen.getByRole('button', { name: 'View run run-firs' }));
    expect(await screen.findByRole('region', { name: 'Run details' })).toHaveTextContent(
      'Nebula is unavailable',
    );
    expect(api.getRun).toHaveBeenCalledWith('run-first', expect.any(AbortSignal));
  });

  it('downloads backend CSV and shows saved snapshot questions', async () => {
    const user = userEvent.setup();
    const api = apiWith([run()]);
    const createObjectURL = vi.fn(() => 'blob:results');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal(
      'URL',
      class extends URL {
        static createObjectURL = createObjectURL;
        static revokeObjectURL = revokeObjectURL;
      },
    );
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    render(<BenchmarkDashboard api={api} />);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Download CSV for run-firs' }));
    await waitFor(() => expect(click).toHaveBeenCalled());
    expect(api.downloadScores).toHaveBeenCalledWith('run-first', expect.any(AbortSignal));
    expect(createObjectURL).toHaveBeenCalledWith(expect.any(Blob));
    await user.click(screen.getByRole('button', { name: 'View snapshot' }));
    expect(await screen.findByRole('region', { name: 'Snapshot details' })).toHaveTextContent(
      'Where is the evidence?',
    );
  });

  it('shows run conflicts and service errors without retrying mutations', async () => {
    const user = userEvent.setup();
    const api = apiWith();
    api.startRun.mockRejectedValueOnce(new ApiError('A benchmark run is already active', 409));
    render(<BenchmarkDashboard api={api} />);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('already active');
    expect(api.startRun).toHaveBeenCalledTimes(1);
    api.startRun.mockRejectedValueOnce(
      new ApiError('Configure NEBULA_API_BASE and NEBULA_API_TOKEN before starting runs', 503),
    );
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Configure NEBULA_API_BASE');
    expect(api.startRun).toHaveBeenCalledTimes(2);
  });

  it('paginates many runs and retains filters in module state', async () => {
    const user = userEvent.setup();
    const api = apiWith(
      Array.from({ length: 31 }, (_, index) =>
        run({ id: `run-${index}`, request: { ...run().request, label: `Architecture ${index}` } }),
      ),
    );
    const onStateChange = vi.fn();
    render(
      <BenchmarkDashboard
        api={api}
        onStateChange={onStateChange}
        initialState={{ query: '', status: 'completed' }}
      />,
    );
    await connected();
    expect(
      within(screen.getByRole('table', { name: /Benchmark results by architecture/ })).getAllByRole(
        'row',
      ),
    ).toHaveLength(26);
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(
      within(screen.getByRole('table', { name: /Benchmark results by architecture/ })).getAllByRole(
        'row',
      ),
    ).toHaveLength(7);
    await user.type(
      screen.getByRole('searchbox', { name: 'Search benchmarks' }),
      'Architecture 30',
    );
    expect(
      within(screen.getByRole('table', { name: /Benchmark results by architecture/ })).getAllByRole(
        'row',
      ),
    ).toHaveLength(2);
    expect(onStateChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: 'Architecture 30', status: 'completed' }),
    );
  });
});
