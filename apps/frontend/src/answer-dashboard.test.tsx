// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import React from 'react';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AnswerDashboard } from './answer-dashboard';
import { ApiError, type BenchmarkApi, type BenchmarkInfo } from './benchmark-api';
import type {
  AnswerApi,
  AnswerCase,
  AnswerReviewRequest,
  AnswerRun,
  AnswerRunSummary,
  AnswerRuntime,
} from './answer-api';

const benchmark: BenchmarkInfo = {
  id: 'snapshot-a',
  source: 'fixture.parquet',
  split: 'test',
  metric_kind: 'paired_context_recovery_v1',
  case_count: 2,
  document_count: 2,
  corpus_path: '/experiment/corpus',
  fingerprint: 'fingerprint-a',
  configuration: {
    key: 'ragtruth-qa',
    definition: {
      name: 'RAGTruth QA',
      source: 'fixture.parquet',
      split: 'test',
      adapter: 'ragtruth_qa',
      evaluation: 'paired_context_recovery_v1',
      defaults: { limit: 2, top_k: 8 },
    },
  },
};
const hotpotBenchmark: BenchmarkInfo = {
  ...benchmark,
  id: 'snapshot-hotpot',
  source: 'hotpot-dev.json',
  split: 'dev',
  metric_kind: 'hotpotqa_answer_v1',
  fingerprint: 'hotpot-fingerprint',
  configuration: {
    key: 'hotpotqa',
    definition: {
      name: 'HotpotQA',
      source: 'hotpot-dev.json',
      split: 'dev',
      adapter: 'hotpotqa',
      evaluation: 'hotpotqa_answer_v1',
      defaults: { limit: 2, top_k: 8 },
    },
  },
};
const runtime: AnswerRuntime = {
  available: true,
  reason: null,
  embedding_model: { id: 'intfloat/multilingual-e5-small', revision: 'embedding-revision' },
  profiles: [
    { id: 'generator-a', label: 'Generator A', enabled: true, disabled_reason: null },
    { id: 'generator-b', label: 'Generator B', enabled: true, disabled_reason: null },
    {
      id: 'disabled-profile',
      label: 'Unavailable generator',
      enabled: false,
      disabled_reason: 'Not configured',
    },
  ],
};
function run(changes: Partial<AnswerRunSummary> = {}): AnswerRunSummary {
  return {
    id: 'baseline-run',
    request: {
      benchmark_id: benchmark.id,
      architecture_label: 'Dense baseline',
      profile_id: 'generator-a',
    },
    benchmark_fingerprint: benchmark.fingerprint,
    evaluation: 'manual_review_v1',
    embedding_model: runtime.embedding_model!,
    generation_model: { profile_id: 'generator-a', label: 'Generator A' },
    top_k: 8,
    status: 'completed',
    started_at_ms: 1000,
    finished_at_ms: 2000,
    total: 2,
    completed: 2,
    failed: 0,
    answered: 2,
    reviewed: 2,
    means: { correctness: 0.5, groundedness: 0.5, hallucination_rate: 0.5, citation_accuracy: 0.5 },
    error: null,
    scope: { corpusId: 'corpus-a' },
    watermark: { generation: 1 },
    source_ids: ['source-a', 'source-b'],
    ...changes,
  };
}
function answer(changes: Partial<AnswerCase> = {}): AnswerCase {
  return {
    case_id: 'case-a',
    query: 'What does the source establish?',
    status: 'ok',
    outcome: 'answered',
    answer: 'The source establishes the observed result [1].',
    reason: null,
    evidence: [
      {
        id: 'evidence-a',
        ordinal: 1,
        sourceId: 'source-a',
        sourceTitle: 'Original context',
        sourceRevision: 'source-revision',
        location: 'Lines 3–5',
        excerpt: 'The observed result was verified in the source.',
      },
    ],
    lineage: [
      { id: 'claim-a', claim: 'The observed result was verified.', evidenceIds: ['evidence-a'] },
    ],
    model_receipt: {
      profileId: 'generator-a',
      route: 'configured-runtime',
      modelLabel: 'Generator A',
    },
    latency_ms: 1000,
    error: null,
    review: null,
    ...changes,
  };
}
function detail(changes: Partial<AnswerRun> = {}): AnswerRun {
  return {
    ...run({ total: 1, completed: 1, answered: 1, reviewed: 0, means: null }),
    cases: [answer()],
    ...changes,
  };
}
function hotpotRun(changes: Partial<AnswerRunSummary> = {}): AnswerRunSummary {
  return run({
    id: 'hotpot-baseline',
    request: {
      ...run().request,
      benchmark_id: hotpotBenchmark.id,
      architecture_label: 'Hotpot baseline',
    },
    benchmark_fingerprint: hotpotBenchmark.fingerprint,
    evaluation: 'hotpotqa_answer_v1',
    answered: 1,
    reviewed: 0,
    means: null,
    scored: 2,
    automatic_scores: { exact_match: 0.5, f1: 0.5 },
    ...changes,
  });
}
function setup(runs: AnswerRunSummary[] = []) {
  const api = {
    getRuntime: vi.fn<AnswerApi['getRuntime']>().mockResolvedValue(runtime),
    listRuns: vi.fn<AnswerApi['listRuns']>().mockResolvedValue(runs),
    getRun: vi.fn<AnswerApi['getRun']>().mockResolvedValue(detail()),
    startRun: vi.fn<AnswerApi['startRun']>().mockResolvedValue(
      run({
        status: 'running',
        completed: 0,
        answered: 0,
        reviewed: 0,
        means: null,
        finished_at_ms: null,
      }),
    ),
    saveReview: vi.fn<AnswerApi['saveReview']>().mockResolvedValue(detail()),
    downloadScores: vi
      .fn<AnswerApi['downloadScores']>()
      .mockResolvedValue(new Blob(['csv'], { type: 'text/csv' })),
  };
  const benchmarkApi = {
    getCatalog: vi.fn<BenchmarkApi['getCatalog']>(),
    listBenchmarks: vi.fn<BenchmarkApi['listBenchmarks']>().mockResolvedValue([benchmark]),
    listRuns: vi.fn<BenchmarkApi['listRuns']>().mockResolvedValue([]),
    getBenchmark: vi.fn<BenchmarkApi['getBenchmark']>(),
    getRun: vi.fn<BenchmarkApi['getRun']>(),
    loadBenchmark: vi.fn<BenchmarkApi['loadBenchmark']>(),
    startRun: vi.fn<BenchmarkApi['startRun']>(),
    downloadScores: vi.fn<BenchmarkApi['downloadScores']>(),
  };
  return { api, benchmarkApi };
}
async function connected() {
  expect(await screen.findByText('Connected')).toBeVisible();
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('Generated answer dashboard', () => {
  it('starts only the selected saved snapshot, architecture label and enabled runtime profile', async () => {
    const user = userEvent.setup();
    const { api, benchmarkApi } = setup();
    benchmarkApi.listBenchmarks.mockResolvedValue([benchmark, hotpotBenchmark]);
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    expect(screen.getByRole('main').firstElementChild).toBe(
      screen.getByRole('region', { name: 'Generated answer results' }),
    );
    expect(screen.queryByRole('option', { name: 'Unavailable generator' })).not.toBeInTheDocument();
    const snapshots = within(screen.getByLabelText('Benchmark snapshot'));
    expect(snapshots.getByRole('option', { name: /RAGTruth QA/ })).toBeInTheDocument();
    expect(snapshots.getByRole('option', { name: /HotpotQA/ })).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText('Benchmark snapshot'), hotpotBenchmark.id);
    expect(screen.getByText('HotpotQA EM/F1 v1')).toBeVisible();
    await user.selectOptions(screen.getByLabelText('Generation model'), 'generator-b');
    await user.type(screen.getByLabelText('Architecture label'), '  Hybrid candidate  ');
    await user.click(screen.getByRole('button', { name: 'Generate answers' }));
    await waitFor(() =>
      expect(api.startRun).toHaveBeenCalledWith(
        {
          benchmark_id: hotpotBenchmark.id,
          architecture_label: 'Hybrid candidate',
          profile_id: 'generator-b',
        },
        expect.any(AbortSignal),
      ),
    );
    expect(api.startRun).toHaveBeenCalledTimes(1);
    expect(api.startRun.mock.calls[0][0]).not.toHaveProperty('top_k');
    expect(api.startRun.mock.calls[0][0]).not.toHaveProperty('token');
    expect(benchmarkApi.loadBenchmark).not.toHaveBeenCalled();
    expect(benchmarkApi.getCatalog).not.toHaveBeenCalled();
  });

  it('shows automatic scores over all HotpotQA cases before optional human review and keeps reference answers separate', async () => {
    const hotpot: AnswerRun = {
      ...hotpotRun(),
      cases: [
        answer({
          query: 'Which city is described?',
          answer: 'London',
          reference_answer: 'London',
          automatic_scores: { exact_match: 1, f1: 1 },
        }),
        answer({
          case_id: 'case-b',
          query: 'Which country is described?',
          outcome: 'refused',
          answer: null,
          model_receipt: null,
          reason: 'The evidence is insufficient.',
          reference_answer: 'France',
          automatic_scores: { exact_match: 0, f1: 0 },
        }),
      ],
    };
    const { api, benchmarkApi } = setup([hotpot]);
    api.getRun.mockResolvedValue(hotpot);
    benchmarkApi.listBenchmarks.mockResolvedValue([benchmark, hotpotBenchmark]);
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    const table = within(screen.getByRole('table'));
    expect(table.getByRole('columnheader', { name: 'Answer EM' })).toBeVisible();
    expect(table.getByRole('columnheader', { name: 'Answer F1' })).toBeVisible();
    const row = within(table.getByRole('row', { name: /Hotpot baseline/ }));
    expect(row.getAllByRole('cell', { name: '50.0% 2 / 2 scored' })).toHaveLength(2);
    expect(row.getByText('1 / 2 answered')).toBeVisible();
    expect(row.getByText('Optional review')).toBeVisible();
    expect(row.getAllByRole('cell', { name: '—' })).toHaveLength(4);
    expect(table.getByText(/HotpotQA EM\/F1 cover all cases/)).toBeVisible();

    await user.click(row.getByRole('button', { name: 'Review answers for hotpot-b' }));
    const panel = within(await screen.findByRole('region', { name: 'Answer run details' }));
    const automatic = within(panel.getByRole('region', { name: 'Automatic answer assessment' }));
    expect(automatic.getByRole('heading', { name: 'Reference answer' })).toBeVisible();
    expect(automatic.getByText('London')).toBeVisible();
    expect(automatic.getByText('Exact match: 100.0% · Token F1: 100.0%')).toBeVisible();
    expect(automatic.getByText(/Human review below is separate/)).toBeVisible();
    const reviewForm = within(panel.getByRole('form', { name: 'Review generated answer' }));
    for (const select of reviewForm.getAllByRole('combobox')) expect(select).toHaveValue('');

    await user.click(panel.getByRole('button', { name: 'Next answer' }));
    const refusalScores = within(
      panel.getByRole('region', { name: 'Automatic answer assessment' }),
    );
    expect(refusalScores.getByText('France')).toBeVisible();
    expect(refusalScores.getByText('Exact match: 0.0% · Token F1: 0.0%')).toBeVisible();
    expect(panel.getByText(/automatic answer scores are zero/)).toBeVisible();
    expect(panel.queryByRole('form', { name: 'Review generated answer' })).not.toBeInTheDocument();
    expect(api.saveReview).not.toHaveBeenCalled();
  });

  it('compares completed HotpotQA runs without human reviews and highlights automatic scores only', async () => {
    const candidate = hotpotRun({
      id: 'hotpot-candidate',
      request: { ...hotpotRun().request, architecture_label: 'Hotpot candidate' },
      answered: 2,
      automatic_scores: { exact_match: 1, f1: 1 },
    });
    const partial = hotpotRun({
      id: 'hotpot-partial',
      request: { ...hotpotRun().request, architecture_label: 'Incomplete Hotpot run' },
      status: 'failed',
      completed: 1,
      failed: 1,
    });
    const manual = run({ benchmark_fingerprint: hotpotBenchmark.fingerprint });
    const { api, benchmarkApi } = setup([hotpotRun(), candidate, partial, manual]);
    benchmarkApi.listBenchmarks.mockResolvedValue([benchmark, hotpotBenchmark]);
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    expect(
      screen.queryByRole('button', { name: 'Compare answer setup for hotpot-p' }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Compare answer setup for hotpot-b' }));
    const table = within(screen.getByRole('table'));
    expect(table.getAllByRole('row')).toHaveLength(3);
    const candidateRow = within(table.getByRole('row', { name: /Hotpot candidate/ }));
    for (const cell of candidateRow.getAllByRole('cell', { name: '100.0% 2 / 2 scored' })) {
      expect(cell).toHaveAttribute('data-best', 'true');
    }
    expect(candidateRow.getAllByRole('cell', { name: '—' })).toHaveLength(4);
    const baselineRow = within(table.getByRole('row', { name: /Hotpot baseline/ }));
    expect(baselineRow.getByText('1 / 2 answered')).toBeVisible();
    for (const cell of baselineRow.getAllByRole('cell', { name: '50.0% 2 / 2 scored' })) {
      expect(cell).not.toHaveAttribute('data-best');
    }
    expect(table.queryByText('Dense baseline')).not.toBeInTheDocument();
    expect(table.queryByText('Incomplete Hotpot run')).not.toBeInTheDocument();
    expect(screen.getByText(/Comparing 2 completed automatically scored runs/)).toBeVisible();
  });

  it('disables generation when runtime is unavailable while displaying saved results and missing review scores', async () => {
    const { api, benchmarkApi } = setup([run({ reviewed: 0, means: null })]);
    api.getRuntime.mockResolvedValue({
      ...runtime,
      available: false,
      reason: 'No generation model configured',
      profiles: [],
    });
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    const row = within(screen.getByRole('row', { name: /Dense baseline/ }));
    expect(row.getByText('2 / 2 answered')).toBeVisible();
    expect(row.getByText('Review pending')).toBeVisible();
    expect(row.getAllByRole('cell', { name: '—' })).toHaveLength(4);
    expect(screen.getByText('No generation model configured')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Generate answers' })).toBeDisabled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('respects the active retrieval run that shares the server generation slot', async () => {
    const { api, benchmarkApi } = setup();
    benchmarkApi.listRuns.mockResolvedValue([
      {
        id: 'retrieval-active',
        request: { benchmark_id: benchmark.id, top_k: 8, label: 'Retrieval baseline' },
        metric_kind: benchmark.metric_kind,
        benchmark_fingerprint: benchmark.fingerprint,
        status: 'running',
        started_at_ms: 1000,
        finished_at_ms: null,
        total: 2,
        completed: 0,
        failed: 0,
        means: null,
        error: null,
        scope: null,
        watermark: null,
        source_ids: [],
      },
    ]);
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    expect(screen.getByRole('button', { name: 'Run in progress' })).toBeDisabled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('highlights lower hallucination and higher other scores only among fully reviewed matching runs', async () => {
    const candidate = run({
      id: 'candidate-run',
      request: { ...run().request, architecture_label: 'Hybrid candidate' },
      means: { correctness: 1, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
    });
    const pending = run({
      id: 'pending-run',
      request: { ...run().request, architecture_label: 'Unreviewed architecture' },
      reviewed: 0,
      means: null,
    });
    const partial = run({
      id: 'partial-run',
      request: { ...run().request, architecture_label: 'Partial architecture' },
      status: 'failed',
      completed: 1,
      failed: 1,
      answered: 1,
      reviewed: 1,
    });
    const other = run({ id: 'other-run', benchmark_fingerprint: 'different-data' });
    const { api, benchmarkApi } = setup([run(), candidate, pending, partial, other]);
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    expect(
      screen.queryByRole('button', { name: 'Compare answer setup for pending-' }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Compare answer setup for partial-' }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Compare answer setup for baseline' }));
    const table = within(screen.getByRole('table'));
    expect(table.getAllByRole('row')).toHaveLength(3);
    const candidateRow = within(table.getByRole('row', { name: /Hybrid candidate/ }));
    expect(candidateRow.getByRole('cell', { name: '0.0%' })).toHaveAttribute('data-best', 'true');
    for (const cell of candidateRow.getAllByRole('cell', { name: '100.0%' }))
      expect(cell).toHaveAttribute('data-best', 'true');
    const baseline = within(table.getByRole('row', { name: /Dense baseline/ }));
    for (const cell of baseline.getAllByRole('cell', { name: '50.0%' }))
      expect(cell).not.toHaveAttribute('data-best');
    expect(table.queryByText('Unreviewed architecture')).not.toBeInTheDocument();
    expect(table.queryByText('Partial architecture')).not.toBeInTheDocument();
  });

  it('shows the actual answer and evidence, requires all four human judgments, and saves updated scores', async () => {
    const pending = detail();
    const { api, benchmarkApi } = setup([pending]);
    api.getRun.mockResolvedValue(pending);
    api.saveReview.mockImplementation(async (_runId, _caseId, body) => {
      const updated = detail({
        reviewed: 1,
        means: { correctness: 1, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
        cases: [answer({ review: { ...body, reviewed_at_ms: 3000 } })],
      });
      api.listRuns.mockResolvedValue([updated]);
      return updated;
    });
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Review answers for baseline' }));
    const panel = within(await screen.findByRole('region', { name: 'Answer run details' }));
    expect(panel.getByText('The source establishes the observed result [1].')).toBeVisible();
    await user.click(panel.getByText('Evidence and citations (1 passages)'));
    expect(panel.getByText('The observed result was verified in the source.')).toBeVisible();
    expect(panel.getByText('Cites evidence-a')).toBeVisible();
    const form = within(panel.getByRole('form', { name: 'Review generated answer' }));
    for (const select of form.getAllByRole('combobox')) expect(select).toHaveValue('');
    await user.click(form.getByRole('button', { name: 'Save review' }));
    expect(api.saveReview).not.toHaveBeenCalled();
    await user.type(form.getByLabelText('Reviewer'), '  Researcher  ');
    await user.selectOptions(form.getByLabelText('Correctness'), 'true');
    await user.selectOptions(form.getByLabelText('Groundedness'), 'true');
    await user.selectOptions(form.getByLabelText('Hallucination'), 'false');
    await user.click(form.getByRole('button', { name: 'Save review' }));
    expect(api.saveReview).not.toHaveBeenCalled();
    await user.selectOptions(form.getByLabelText('Citation accuracy'), 'true');
    await user.type(form.getByLabelText('Review notes'), 'Supported by the displayed context.');
    await user.click(form.getByRole('button', { name: 'Save review' }));
    const expected: AnswerReviewRequest = {
      reviewer: 'Researcher',
      correctness: true,
      groundedness: true,
      hallucination: false,
      citation_accuracy: true,
      notes: 'Supported by the displayed context.',
    };
    await waitFor(() =>
      expect(api.saveReview).toHaveBeenCalledWith(
        'baseline-run',
        'case-a',
        expected,
        expect.any(AbortSignal),
      ),
    );
    expect(api.saveReview).toHaveBeenCalledTimes(1);
    const row = within(screen.getByRole('row', { name: /Dense baseline/ }));
    await waitFor(() => expect(row.getByText('Reviewed answers')).toBeVisible());
    expect(row.getByRole('cell', { name: '0.0%' })).toBeVisible();
    expect(row.getAllByRole('cell', { name: '100.0%' })).toHaveLength(3);
    expect(screen.getByRole('status')).toHaveTextContent('Review saved');
    expect(panel.getByRole('button', { name: 'Update review' })).toBeVisible();
  });

  it('keeps refusals and evidence-only cases in coverage without a review form or assigned scores', async () => {
    const refused = detail({
      total: 2,
      completed: 2,
      answered: 0,
      reviewed: 0,
      means: null,
      cases: [
        answer({
          outcome: 'refused',
          answer: null,
          reason: 'Evidence cannot support an answer',
          model_receipt: null,
        }),
        answer({
          case_id: 'case-b',
          query: 'What else is known?',
          outcome: 'evidence-only',
          answer: null,
          model_receipt: null,
        }),
      ],
    });
    const { api, benchmarkApi } = setup([refused]);
    api.getRun.mockResolvedValue(refused);
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    expect(screen.getByText('0 / 2 answered')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Review answers for baseline' }));
    const panel = within(await screen.findByRole('region', { name: 'Answer run details' }));
    expect(panel.getByText('refused')).toBeVisible();
    expect(panel.queryByRole('form', { name: 'Review generated answer' })).not.toBeInTheDocument();
    await user.click(panel.getByRole('button', { name: 'Next answer' }));
    expect(panel.getByText('evidence-only')).toBeVisible();
    expect(panel.queryByRole('form', { name: 'Review generated answer' })).not.toBeInTheDocument();
    expect(panel.getByText(/no quality scores are assigned/)).toBeVisible();
    expect(api.saveReview).not.toHaveBeenCalled();
  });

  it('keeps answers closed when a pending review finishes while refreshing the saved summary', async () => {
    const priorReview = {
      reviewer: 'Researcher',
      correctness: false,
      groundedness: true,
      hallucination: false,
      citation_accuracy: true,
      notes: '',
      reviewed_at_ms: 3000,
    };
    const reviewed = detail({
      reviewed: 1,
      means: { correctness: 0, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
      cases: [answer({ review: priorReview })],
    });
    const updated = detail({
      reviewed: 1,
      means: { correctness: 1, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
      cases: [answer({ review: { ...priorReview, correctness: true, reviewed_at_ms: 4000 } })],
    });
    const { api, benchmarkApi } = setup([reviewed]);
    api.getRun.mockResolvedValue(reviewed);
    let resolveReview!: (value: AnswerRun) => void;
    api.saveReview.mockReturnValue(
      new Promise((resolve) => {
        resolveReview = resolve;
      }),
    );
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Review answers for baseline' }));
    const form = within(await screen.findByRole('form', { name: 'Review generated answer' }));
    await user.selectOptions(form.getByLabelText('Correctness'), 'true');
    await user.click(form.getByRole('button', { name: 'Update review' }));
    expect(api.saveReview).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Close answers' }));
    expect(screen.queryByRole('region', { name: 'Answer run details' })).not.toBeInTheDocument();

    api.listRuns.mockResolvedValue([updated]);
    await act(async () => {
      resolveReview(updated);
    });
    await waitFor(() => expect(api.listRuns).toHaveBeenCalledTimes(2));
    const row = within(screen.getByRole('row', { name: /Dense baseline/ }));
    expect(row.getAllByRole('cell', { name: '100.0%' })).toHaveLength(3);
    expect(screen.getByRole('status')).toHaveTextContent('Review saved');
    expect(screen.queryByRole('region', { name: 'Answer run details' })).not.toBeInTheDocument();
    expect(api.getRun).toHaveBeenCalledTimes(1);
  });

  it('reports a rejected review once and refreshes saved data without repeating the write', async () => {
    const reviewed = detail({
      reviewed: 1,
      means: { correctness: 1, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
      cases: [
        answer({
          review: {
            reviewer: 'Researcher',
            correctness: true,
            groundedness: true,
            hallucination: false,
            citation_accuracy: true,
            notes: '',
            reviewed_at_ms: 3000,
          },
        }),
      ],
    });
    const { api, benchmarkApi } = setup([reviewed]);
    api.getRun.mockResolvedValue(reviewed);
    api.saveReview.mockRejectedValue(
      new ApiError('Review cannot be saved while generation is active', 409),
    );
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Review answers for baseline' }));
    await user.click(await screen.findByRole('button', { name: 'Update review' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Review cannot be saved while generation is active',
    );
    await waitFor(() => expect(api.listRuns).toHaveBeenCalledTimes(2));
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(api.listRuns).toHaveBeenCalledTimes(3));
    expect(api.saveReview).toHaveBeenCalledTimes(1);
    expect(api.getRun).toHaveBeenCalledTimes(1);
  });

  it.each([
    new ApiError('A benchmark run is already active', 409),
    new TypeError('Connection lost after generation request'),
  ])('shows generation errors and refreshes without retrying the mutation', async (error) => {
    const { api, benchmarkApi } = setup();
    api.startRun.mockRejectedValue(error);
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    await user.type(screen.getByLabelText('Architecture label'), 'Experiment');
    await user.click(screen.getByRole('button', { name: 'Generate answers' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(error.message);
    await waitFor(() => expect(api.listRuns).toHaveBeenCalledTimes(2));
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(api.listRuns).toHaveBeenCalledTimes(3));
    expect(api.startRun).toHaveBeenCalledTimes(1);
  });

  it('polls active generation, updates coverage, and stops polling with aborted reads on unmount', async () => {
    vi.useFakeTimers();
    const active = run({
      status: 'running',
      completed: 1,
      answered: 1,
      reviewed: 0,
      means: null,
      finished_at_ms: null,
    });
    const { api, benchmarkApi } = setup([active]);
    const view = render(
      <AnswerDashboard api={api} benchmarkApi={benchmarkApi} pollInterval={3000} />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '1');
    expect(screen.getByRole('button', { name: 'Run in progress' })).toBeDisabled();
    api.listRuns.mockResolvedValue([run({ reviewed: 0, means: null })]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
    expect(screen.getByText('2 / 2 answered')).toBeVisible();
    expect(api.listRuns).toHaveBeenCalledTimes(2);
    const signal = api.listRuns.mock.calls.at(-1)![0]!;
    view.unmount();
    expect(signal.aborted).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9000);
    });
    expect(api.listRuns).toHaveBeenCalledTimes(2);
  });

  it('aborts a pending answer detail request when its tab closes', async () => {
    const { api, benchmarkApi } = setup([run()]);
    api.getRun.mockImplementation(() => new Promise(() => {}));
    const user = userEvent.setup();
    const view = render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Review answers for baseline' }));
    expect(api.getRun).toHaveBeenCalledTimes(1);
    const signal = api.getRun.mock.calls[0][1]!;
    view.unmount();
    expect(signal.aborted).toBe(true);
  });

  it('paginates large histories and resets the page when filtering by architecture', async () => {
    const runs = Array.from({ length: 26 }, (_, index) =>
      run({
        id: `run-${String(index).padStart(4, '0')}`,
        request: { ...run().request, architecture_label: `Architecture ${index + 1}` },
      }),
    );
    const { api, benchmarkApi } = setup(runs);
    const user = userEvent.setup();
    render(<AnswerDashboard api={api} benchmarkApi={benchmarkApi} />);
    await connected();
    expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(26);
    expect(screen.queryByText('Architecture 26')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByText('Page 2 of 2')).toBeVisible();
    expect(screen.getByText('Architecture 26')).toBeVisible();
    await user.type(
      screen.getByRole('searchbox', { name: 'Search answer runs' }),
      'Architecture 7',
    );
    expect(screen.getByText('Page 1 of 1')).toBeVisible();
    expect(screen.getByText('Architecture 7')).toBeVisible();
    expect(screen.queryByText('Architecture 26')).not.toBeInTheDocument();
  });
});
