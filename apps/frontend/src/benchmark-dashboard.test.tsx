// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import React from 'react';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BenchmarkDashboard } from './benchmark-dashboard';
import type { BenchmarkApi, BenchmarkDefinition, BenchmarkInfo } from './benchmark-api';
import type { AnswerApi } from './answer-api';
import type { ResultsApi, SuiteRun } from './results-api';

const definition: BenchmarkDefinition = {
  name: 'RAGTruth QA',
  source: '/data',
  split: 'test',
  adapter: 'ragtruth_qa',
  evaluation: 'paired_context_recovery_v1',
  defaults: { limit: 10, top_k: 8 },
};
const hotpot: BenchmarkDefinition = {
  ...definition,
  name: 'HotpotQA',
  adapter: 'hotpotqa_distractor',
  evaluation: 'hotpotqa_answer_v1',
};
const snapshot: BenchmarkInfo = {
  id: 'rag-snapshot',
  source: '/data',
  split: 'test',
  metric_kind: definition.evaluation,
  fingerprint: 'fp',
  case_count: 10,
  document_count: 10,
  corpus_path: '/corpus',
  configuration: { key: 'ragtruth-qa', definition },
};
const row = {
  run_number: '13',
  suite_run_number: '12',
  run_id: 'rag-run',
  architecture_name: 'Dense v2',
  description: 'New chunking',
  mode: 'retrieval',
  status: 'completed',
  evaluation: definition.evaluation,
  total: '10',
  completed: '10',
  failed: '0',
  top_k: '8',
  started_at_ms: '123456789',
  benchmark_fingerprint: 'fp',
  'metric.context_hit_at_k': '0',
  'metric.reciprocal_rank_at_k': '0.75',
  'metric.custom_metric': '0.5',
};
const suite: SuiteRun = {
  id: 'suite-id',
  run_number: 12,
  request: { architecture_label: 'Dense v2', description: '', profile_id: 'profile' },
  status: 'running',
  started_at_ms: 123,
  finished_at_ms: null,
  error: null,
  items: [
    {
      benchmark: 'ragtruth-qa',
      benchmark_id: snapshot.id,
      mode: 'retrieval',
      run_id: 'rag-run',
      status: 'running',
      reason: null,
    },
    {
      benchmark: 'hotpotqa',
      benchmark_id: 'hotpot-snapshot',
      mode: 'generation',
      run_id: null,
      status: 'queued',
      reason: null,
    },
    {
      benchmark: 'qasper',
      benchmark_id: null,
      mode: null,
      run_id: null,
      status: 'skipped',
      reason: 'Dataset loader is not integrated',
    },
  ],
};
function setup() {
  const api = {
    getCatalog: vi.fn<BenchmarkApi['getCatalog']>().mockResolvedValue({
      benchmarks: {
        'ragtruth-qa': definition,
        hotpotqa: hotpot,
        qasper: {
          ...definition,
          name: 'QASPER',
          adapter: 'external_suite',
          evaluation: 'external_evaluation',
        },
      },
    }),
    listBenchmarks: vi.fn<BenchmarkApi['listBenchmarks']>().mockResolvedValue([
      snapshot,
      {
        ...snapshot,
        id: 'hotpot-snapshot',
        metric_kind: hotpot.evaluation,
        configuration: { key: 'hotpotqa', definition: hotpot },
      },
    ]),
    listRuns: vi.fn<BenchmarkApi['listRuns']>().mockResolvedValue([]),
    getBenchmark: vi.fn<BenchmarkApi['getBenchmark']>(),
    getRun: vi.fn<BenchmarkApi['getRun']>(),
    loadBenchmark: vi.fn<BenchmarkApi['loadBenchmark']>(),
    startRun: vi.fn<BenchmarkApi['startRun']>(),
    downloadScores: vi.fn<BenchmarkApi['downloadScores']>(),
  };
  const answerApi = {
    getRuntime: vi.fn<AnswerApi['getRuntime']>().mockResolvedValue({
      available: true,
      reason: null,
      embedding_model: { id: 'embed', revision: 'v1' },
      profiles: [
        { id: 'profile', label: 'Generation model', enabled: true, disabled_reason: null },
      ],
    }),
    listRuns: vi.fn<AnswerApi['listRuns']>().mockResolvedValue([]),
    getRun: vi.fn<AnswerApi['getRun']>(),
    startRun: vi.fn<AnswerApi['startRun']>(),
    saveReview: vi.fn<AnswerApi['saveReview']>(),
    downloadScores: vi.fn<AnswerApi['downloadScores']>(),
  };
  const resultsApi = {
    getResults: vi.fn<ResultsApi['getResults']>().mockResolvedValue({
      benchmarks: [
        { benchmark: 'ragtruth-qa', columns: Object.keys(row), rows: [row] },
        {
          benchmark: 'hotpotqa',
          columns: [...Object.keys(row), 'metric.exact_match', 'review.correctness'],
          rows: [
            {
              ...row,
              run_number: '14',
              run_id: 'hotpot-run',
              mode: 'generation',
              architecture_name: 'Hotpot architecture',
              evaluation: hotpot.evaluation,
              answered: '8',
              reviewed: '0',
              scored: '10',
              'metric.context_hit_at_k': '',
              'metric.reciprocal_rank_at_k': '',
              'metric.custom_metric': '',
              'metric.exact_match': '0.4',
              'review.correctness': '',
            },
          ],
        },
      ],
    }),
    listSuites: vi.fn<ResultsApi['listSuites']>().mockResolvedValue([]),
    startSuite: vi.fn<ResultsApi['startSuite']>().mockResolvedValue(suite),
    downloadResults: vi.fn<ResultsApi['downloadResults']>(),
  };
  return { api, answerApi, resultsApi };
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Benchmark workspace', () => {
  it('shows one benchmark per table with stable run numbers, zero scores and dynamic metrics', async () => {
    const user = userEvent.setup();
    const props = setup();
    render(<BenchmarkDashboard {...props} />);
    const table = within(await screen.findByRole('table'));
    expect(await table.findByText('Dense v2')).toBeVisible();
    expect(table.getByRole('rowheader', { name: /#12/ })).toBeVisible();
    expect(table.getByRole('cell', { name: '0.000' })).toBeVisible();
    expect(table.getByRole('columnheader', { name: 'custom metric' })).toBeVisible();
    expect(table.queryByText('Hotpot architecture')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /HotpotQA 1 run/ }));
    expect(screen.getByRole('tab', { name: 'Generated answers' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    const next = within(screen.getByRole('table'));
    expect(next.getByText('Hotpot architecture')).toBeVisible();
    expect(next.queryByText('Dense v2')).not.toBeInTheDocument();
    expect(next.getByRole('columnheader', { name: 'Answer EM' })).toBeVisible();
    expect(next.queryByRole('columnheader', { name: 'Hit@k' })).not.toBeInTheDocument();
    expect(
      next.queryByRole('columnheader', { name: 'Review · Correctness' }),
    ).not.toBeInTheDocument();
    expect(next.getByText('8 answered · 0 reviewed')).toBeVisible();
    await user.click(screen.getByRole('button', { name: /QASPER Integration required/ }));
    expect(
      screen.getByText('This benchmark needs a backend integration before it can produce scores.'),
    ).toBeVisible();
    expect(props.api.startRun).not.toHaveBeenCalled();
    expect(props.answerApi.startRun).not.toHaveBeenCalled();
  });
  it('starts one durable suite across all benchmarks regardless of the selected table', async () => {
    const user = userEvent.setup();
    const props = setup();
    render(<BenchmarkDashboard {...props} />);
    await screen.findByText('Connected');
    await user.click(screen.getByRole('button', { name: /HotpotQA 1 run/ }));
    await user.click(screen.getByRole('button', { name: 'Run all benchmarks' }));
    await user.type(screen.getByRole('textbox', { name: 'Architecture label' }), 'Dense v3');
    await user.type(screen.getByRole('textbox', { name: 'Run notes' }), 'New index\nSame passages');
    props.resultsApi.listSuites.mockResolvedValue([suite]);
    await user.click(screen.getByRole('button', { name: 'Start suite run' }));
    await waitFor(() =>
      expect(props.resultsApi.startSuite).toHaveBeenCalledWith(
        {
          architecture_label: 'Dense v3',
          description: 'New index\nSame passages',
          profile_id: 'profile',
        },
        expect.any(AbortSignal),
      ),
    );
    expect(await screen.findByText(/Run #12 started/)).toBeVisible();
    expect(screen.getByText('Dataset loader is not integrated')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Run in progress' })).toBeDisabled();
    expect(props.api.startRun).not.toHaveBeenCalled();
    expect(props.answerApi.startRun).not.toHaveBeenCalled();
  });
  it('retains benchmark and filters across remounts and supports keyboard evaluation tabs', async () => {
    const user = userEvent.setup();
    const props = setup();
    const onStateChange = vi.fn();
    const view = render(<BenchmarkDashboard {...props} onStateChange={onStateChange} />);
    await screen.findByText('Connected');
    await user.click(screen.getByRole('button', { name: /HotpotQA 1 run/ }));
    await user.type(screen.getByRole('searchbox', { name: 'Search runs' }), 'Hotpot');
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Filter run status' }),
      'completed',
    );
    const saved = onStateChange.mock.calls.at(-1)![0];
    view.unmount();
    render(<BenchmarkDashboard {...props} initialState={saved} />);
    await screen.findByText('Connected');
    expect(screen.getByRole('button', { name: /HotpotQA 1 run/ })).toHaveAttribute(
      'aria-current',
      'true',
    );
    expect(screen.getByRole('searchbox', { name: 'Search runs' })).toHaveValue('Hotpot');
    expect(screen.getByRole('combobox', { name: 'Filter run status' })).toHaveValue('completed');
    screen.getByRole('tab', { name: 'Generated answers' }).focus();
    await user.keyboard('{Home}');
    expect(screen.getByRole('tab', { name: 'Retrieval' })).toHaveFocus();
    expect(screen.getByRole('tabpanel')).toHaveAccessibleName('Retrieval');
    await user.keyboard('{End}');
    expect(screen.getByRole('tab', { name: 'Generated answers' })).toHaveFocus();
  });
  it('waits for generation availability before submitting the suite', async () => {
    const user = userEvent.setup();
    const props = setup();
    const available = await props.answerApi.getRuntime();
    let resolveRuntime!: (value: typeof available) => void;
    props.answerApi.getRuntime.mockReturnValue(
      new Promise((resolve) => {
        resolveRuntime = resolve;
      }),
    );
    render(<BenchmarkDashboard {...props} />);
    await screen.findByText('Connected');
    await user.click(screen.getByRole('button', { name: 'Run all benchmarks' }));
    await user.type(screen.getByRole('textbox', { name: 'Architecture label' }), 'Full suite');
    expect(screen.getByText('Checking generation availability…')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Start suite run' })).toBeDisabled();
    await act(async () => resolveRuntime(available));
    await user.click(screen.getByRole('button', { name: 'Start suite run' }));
    expect(props.resultsApi.startSuite).toHaveBeenCalledWith(
      expect.objectContaining({ profile_id: 'profile' }),
      expect.any(AbortSignal),
    );
    expect(props.resultsApi.startSuite).toHaveBeenCalledTimes(1);
  });

  it('groups custom catalog aliases with their canonical result table', async () => {
    const props = setup();
    props.api.getCatalog.mockResolvedValue({
      benchmarks: { 'team-ragtruth': definition },
      module_keys: { 'team-ragtruth': 'ragtruth-qa' },
    });
    props.api.listBenchmarks.mockResolvedValue([
      {
        ...snapshot,
        module_key: 'ragtruth-qa',
        configuration: { key: 'team-ragtruth', definition },
      },
    ]);
    render(<BenchmarkDashboard {...props} />);
    await screen.findByText('Connected');
    expect(screen.getAllByRole('button', { name: /RAGTruth QA/ })).toHaveLength(1);
    expect(screen.getByText('Dense v2')).toBeVisible();
  });

  it('moves focus to run inspection and returns it to the originating control', async () => {
    const props = setup();
    const user = userEvent.setup();
    render(<BenchmarkDashboard {...props} />);
    await screen.findByText('Connected');
    const trigger = screen.getByRole('button', { name: 'Inspect run 12' });
    await user.click(trigger);
    expect(screen.getByRole('heading', { name: 'Evidence & comparison' })).toHaveFocus();
    expect(screen.queryByRole('form', { name: 'Start benchmark run' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close inspection' }));
    expect(trigger).toHaveFocus();
  });

  it('retains saved results when refresh fails and never retries an uncertain suite write', async () => {
    const user = userEvent.setup();
    const props = setup();
    render(<BenchmarkDashboard {...props} />);
    await screen.findByText('Connected');
    props.resultsApi.startSuite.mockRejectedValue(new Error('Connection lost'));
    await user.click(screen.getByRole('button', { name: 'Run all benchmarks' }));
    await user.type(screen.getByRole('textbox', { name: 'Architecture label' }), 'Baseline');
    await user.click(screen.getByRole('button', { name: 'Start suite run' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Connection lost');
    expect(props.resultsApi.startSuite).toHaveBeenCalledTimes(1);
    props.resultsApi.getResults.mockRejectedValue(new Error('Offline'));
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText(/Previously loaded results remain visible/)).toBeVisible();
    expect(screen.getByText('Dense v2')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Start suite run' })).toBeDisabled();
  });
  it('continues browsing without a generation runtime and submits no profile for retrieval-only availability', async () => {
    const user = userEvent.setup();
    const props = setup();
    props.answerApi.getRuntime.mockRejectedValue(new Error('Nebula unavailable'));
    render(<BenchmarkDashboard {...props} />);
    await screen.findByText('Connected');
    await user.click(screen.getByRole('button', { name: 'Run all benchmarks' }));
    expect(screen.getByText(/Answer evaluations will be skipped/)).toBeVisible();
    await user.type(screen.getByRole('textbox', { name: 'Architecture label' }), 'Retrieval');
    await user.click(screen.getByRole('button', { name: 'Start suite run' }));
    expect(props.resultsApi.startSuite).toHaveBeenCalledWith(
      { architecture_label: 'Retrieval', description: '' },
      expect.any(AbortSignal),
    );
  });
});
