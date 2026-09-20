// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import React from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BenchmarkDashboard } from './benchmark-dashboard';
import type { BenchmarkApi, BenchmarkInfo } from './benchmark-api';
import type { AnswerApi } from './answer-api';

function setup() {
  const api = {
    getCatalog: vi.fn<BenchmarkApi['getCatalog']>().mockResolvedValue({ benchmarks: {} }),
    listBenchmarks: vi.fn<BenchmarkApi['listBenchmarks']>().mockResolvedValue([]),
    listRuns: vi.fn<BenchmarkApi['listRuns']>().mockResolvedValue([]),
    getBenchmark: vi.fn<BenchmarkApi['getBenchmark']>(),
    getRun: vi.fn<BenchmarkApi['getRun']>(),
    loadBenchmark: vi.fn<BenchmarkApi['loadBenchmark']>(),
    startRun: vi.fn<BenchmarkApi['startRun']>(),
    downloadScores: vi.fn<BenchmarkApi['downloadScores']>(),
  };
  const answerApi = {
    getRuntime: vi.fn<AnswerApi['getRuntime']>().mockResolvedValue({
      available: false,
      reason: 'No runtime configured',
      embedding_model: null,
      profiles: [],
    }),
    listRuns: vi.fn<AnswerApi['listRuns']>().mockResolvedValue([]),
    getRun: vi.fn<AnswerApi['getRun']>(),
    startRun: vi.fn<AnswerApi['startRun']>(),
    saveReview: vi.fn<AnswerApi['saveReview']>(),
    downloadScores: vi.fn<AnswerApi['downloadScores']>(),
  };
  return { api, answerApi };
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Benchmark evaluation tabs', () => {
  it('shows prepared HotpotQA on the default view and opens its answer setup without starting a run', async () => {
    const { api, answerApi } = setup();
    const user = userEvent.setup();
    const snapshot: BenchmarkInfo = {
      id: 'hotpot-snapshot',
      source: 'HotpotQA',
      split: 'dev',
      metric_kind: 'hotpotqa_answer_v1',
      case_count: 100,
      document_count: 991,
      corpus_path: '/corpus/hotpotqa',
      fingerprint: 'hotpot-fingerprint',
    };
    api.listBenchmarks.mockResolvedValue([
      {
        ...snapshot,
        id: 'ragtruth-snapshot',
        source: 'RAGTruth QA',
        metric_kind: 'paired_context_recovery_v1',
      },
      snapshot,
    ]);
    render(
      <BenchmarkDashboard
        api={api}
        answerApi={answerApi}
        initialState={{ answers: { benchmarkId: 'ragtruth-snapshot', query: 'Saved filter' } }}
      />,
    );
    const shortcut = await screen.findByRole('button', {
      name: 'Open HotpotQA in Generated answers',
    });
    expect(shortcut).toBeVisible();
    expect(screen.getByRole('combobox', { name: 'Loaded snapshot' })).toHaveValue(
      'ragtruth-snapshot',
    );
    await user.click(shortcut);
    expect(screen.getByRole('tab', { name: 'Generated answers' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await waitFor(() =>
      expect(screen.getByRole('combobox', { name: 'Benchmark snapshot' })).toHaveValue(
        'hotpot-snapshot',
      ),
    );
    expect(screen.getByRole('searchbox', { name: 'Search answer runs' })).toHaveValue(
      'Saved filter',
    );
    expect(screen.getByText('HotpotQA EM/F1 v1')).toBeVisible();
    expect(api.startRun).not.toHaveBeenCalled();
    expect(answerApi.startRun).not.toHaveBeenCalled();
  });

  it('shows unprepared YAML entries below the results in both tabs without loading or running them', async () => {
    const { api, answerApi } = setup();
    api.getCatalog.mockResolvedValue({
      benchmarks: {
        pending: {
          name: 'Pending team evaluation',
          source: 'https://example.org/dataset',
          split: 'test',
          adapter: 'external_suite',
          evaluation: 'external_evaluation',
          defaults: { limit: 100, top_k: 8 },
          description: 'A team-configured evaluation suite.',
          preparation: 'Integrate its dataset-specific judging protocol.',
        },
      },
    });
    const user = userEvent.setup();
    render(<BenchmarkDashboard api={api} answerApi={answerApi} />);
    expect(await screen.findByText('Pending team evaluation')).toBeVisible();
    expect(screen.getByText('Integration required')).toBeVisible();
    expect(screen.getByRole('main').firstElementChild).toBe(
      screen.getByRole('region', { name: 'Benchmark results' }),
    );
    expect(screen.getByRole('main').children[1]).toBe(
      screen.getByRole('region', { name: 'Configured benchmarks' }),
    );
    await user.click(screen.getByRole('tab', { name: 'Generated answers' }));
    expect(await screen.findByText('Pending team evaluation')).toBeVisible();
    expect(screen.getByText('Dataset-specific evaluation')).toBeVisible();
    expect(screen.getByRole('main').firstElementChild).toBe(
      screen.getByRole('region', { name: 'Generated answer results' }),
    );
    expect(screen.getByRole('main').children[1]).toBe(
      screen.getByRole('region', { name: 'Configured benchmarks' }),
    );
    expect(api.loadBenchmark).not.toHaveBeenCalled();
    expect(api.startRun).not.toHaveBeenCalled();
    expect(answerApi.startRun).not.toHaveBeenCalled();
  });

  it('opens each real panel on click and retains legacy retrieval filters and answer filters separately', async () => {
    const { api, answerApi } = setup();
    const user = userEvent.setup();
    const onStateChange = vi.fn();
    render(
      <BenchmarkDashboard
        api={api}
        answerApi={answerApi}
        initialState={{ query: 'Dense', status: 'completed' }}
        onStateChange={onStateChange}
      />,
    );
    expect(await screen.findByText('Connected')).toBeVisible();
    expect(screen.getByRole('tab', { name: 'Retrieval' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('searchbox', { name: 'Search benchmarks' })).toHaveValue('Dense');
    expect(screen.getByRole('combobox', { name: 'Filter run status' })).toHaveValue('completed');
    expect(answerApi.listRuns).not.toHaveBeenCalled();
    const retrievalSignal = api.listRuns.mock.calls.at(-1)![0]!;
    await user.click(screen.getByRole('tab', { name: 'Generated answers' }));
    expect(
      await screen.findByRole('heading', { name: 'Architecture & model comparison' }),
    ).toBeVisible();
    await waitFor(() => expect(answerApi.listRuns).toHaveBeenCalledTimes(1));
    expect(retrievalSignal.aborted).toBe(true);
    expect(screen.getByRole('tabpanel')).toHaveAccessibleName('Generated answers');
    await user.type(screen.getByRole('searchbox', { name: 'Search answer runs' }), 'Generator B');
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Filter answer run status' }),
      'failed',
    );
    const answerSignal = answerApi.listRuns.mock.calls.at(-1)![0]!;
    await user.click(screen.getByRole('tab', { name: 'Retrieval' }));
    expect(answerSignal.aborted).toBe(true);
    expect(screen.getByRole('searchbox', { name: 'Search benchmarks' })).toHaveValue('Dense');
    expect(screen.getByRole('combobox', { name: 'Filter run status' })).toHaveValue('completed');
    await user.click(screen.getByRole('tab', { name: 'Generated answers' }));
    expect(screen.getByRole('searchbox', { name: 'Search answer runs' })).toHaveValue(
      'Generator B',
    );
    expect(screen.getByRole('combobox', { name: 'Filter answer run status' })).toHaveValue(
      'failed',
    );
    await waitFor(() =>
      expect(onStateChange).toHaveBeenLastCalledWith(
        expect.objectContaining({
          view: 'answers',
          retrieval: expect.objectContaining({ query: 'Dense', status: 'completed' }),
          answers: expect.objectContaining({ query: 'Generator B', status: 'failed' }),
        }),
      ),
    );
  });

  it('supports arrow, Home and End keyboard navigation with associated tab panels', async () => {
    const { api, answerApi } = setup();
    const user = userEvent.setup();
    render(<BenchmarkDashboard api={api} answerApi={answerApi} />);
    await screen.findByText('Connected');
    const retrieval = screen.getByRole('tab', { name: 'Retrieval' });
    const answers = screen.getByRole('tab', { name: 'Generated answers' });
    retrieval.focus();
    await user.keyboard('{ArrowRight}');
    expect(answers).toHaveFocus();
    expect(answers).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel')).toHaveAttribute(
      'id',
      answers.getAttribute('aria-controls'),
    );
    await user.keyboard('{ArrowLeft}');
    expect(retrieval).toHaveFocus();
    expect(retrieval).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{End}');
    expect(answers).toHaveFocus();
    expect(answers).toHaveAttribute('tabindex', '0');
    expect(retrieval).toHaveAttribute('tabindex', '-1');
    await user.keyboard('{Home}');
    expect(retrieval).toHaveFocus();
    expect(screen.getByRole('tabpanel')).toHaveAccessibleName('Retrieval');
  });

  it('restores the saved answer tab and nested filters after a host remount', async () => {
    const { api, answerApi } = setup();
    const state = {
      view: 'answers',
      retrieval: { query: 'Retrieval filter', status: 'running' },
      answers: { query: 'Saved model', status: 'interrupted' },
    };
    const user = userEvent.setup();
    render(<BenchmarkDashboard api={api} answerApi={answerApi} initialState={state} />);
    await screen.findByText('Connected');
    expect(screen.getByRole('tab', { name: 'Generated answers' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByRole('searchbox', { name: 'Search answer runs' })).toHaveValue(
      'Saved model',
    );
    expect(screen.getByRole('combobox', { name: 'Filter answer run status' })).toHaveValue(
      'interrupted',
    );
    await user.click(screen.getByRole('tab', { name: 'Retrieval' }));
    expect(screen.getByRole('searchbox', { name: 'Search benchmarks' })).toHaveValue(
      'Retrieval filter',
    );
    expect(screen.getByRole('combobox', { name: 'Filter run status' })).toHaveValue('running');
  });
});
