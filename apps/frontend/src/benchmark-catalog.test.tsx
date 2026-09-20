// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import React from 'react';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BenchmarkCatalog } from './benchmark-catalog';
import type { BenchmarkApi, BenchmarkDefinition, BenchmarkInfo, Catalog } from './benchmark-api';

const definition: BenchmarkDefinition = {
  name: 'Team benchmark',
  source: 'https://datasets.example.org/questions.json',
  split: 'dev',
  adapter: 'hotpotqa_distractor',
  evaluation: 'hotpotqa_answer_v1',
  defaults: { limit: 100, top_k: 8 },
};
const snapshot: BenchmarkInfo = {
  id: 'snapshot-one',
  source: definition.source,
  split: 'dev',
  metric_kind: 'hotpotqa_answer_v1',
  case_count: 100,
  document_count: 991,
  corpus_path: '/local/corpus',
  fingerprint: 'fingerprint-one',
  configuration: { key: 'team-suite', definition },
};
function apiWith(catalog: Catalog) {
  return {
    getCatalog: vi.fn<BenchmarkApi['getCatalog']>().mockResolvedValue(catalog),
    loadBenchmark: vi.fn<BenchmarkApi['loadBenchmark']>(),
    startRun: vi.fn<BenchmarkApi['startRun']>(),
  } as unknown as BenchmarkApi;
}

afterEach(cleanup);

describe('Read-only YAML benchmark catalog', () => {
  it('shows configured suites before preparation without claiming a runnable evaluator', async () => {
    const api = apiWith({
      benchmarks: {
        future: {
          ...definition,
          name: 'Future evaluation suite',
          adapter: 'external_suite',
          evaluation: 'external_evaluation',
          description: 'Checks long conversational memory.',
          preparation: 'Prepare the licensed dataset and its dataset-specific evaluator.',
          homepage: 'https://research.example.org/suite',
        },
      },
    });
    render(<BenchmarkCatalog api={api} snapshots={[]} onOpenAnswers={vi.fn()} />);
    const row = within(await screen.findByRole('row', { name: /Future evaluation suite/ }));
    expect(row.getByText('Integration required')).toBeVisible();
    expect(row.getByText('Dataset-specific evaluation')).toBeVisible();
    expect(row.getByText('Checks long conversational memory.')).toBeVisible();
    expect(row.getByText(/Prepare the licensed dataset/)).not.toBeVisible();
    await userEvent.click(row.getByText('Setup requirements'));
    expect(row.getByText(/Prepare the licensed dataset/)).toBeVisible();
    expect(
      screen.getByRole('region', { name: 'Benchmark catalog table, scroll for all columns' }),
    ).toHaveAttribute('tabindex', '0');
    expect(screen.getAllByRole('columnheader').map((heading) => heading.textContent)).toEqual([
      'Benchmark',
      'Preparation',
      'Evaluation',
      'Links',
    ]);
    expect(row.getByRole('link', { name: 'Dataset source ↗' })).toHaveAttribute(
      'href',
      definition.source,
    );
    expect(row.getByRole('link', { name: 'Project page ↗' })).toHaveAttribute(
      'href',
      'https://research.example.org/suite',
    );
    expect(row.queryByRole('button')).not.toBeInTheDocument();
    expect(api.loadBenchmark).not.toHaveBeenCalled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('counts snapshots by catalog key and opens the chosen prepared snapshot without starting work', async () => {
    const api = apiWith({
      benchmarks: {
        'team-suite': definition,
        unprepared: { ...definition, name: 'Unprepared suite' },
      },
    });
    const onOpenAnswers = vi.fn();
    const user = userEvent.setup();
    render(
      <BenchmarkCatalog
        api={api}
        snapshots={[snapshot, { ...snapshot, id: 'snapshot-two', case_count: 40 }]}
        onOpenAnswers={onOpenAnswers}
      />,
    );
    const row = within(await screen.findByRole('row', { name: /Team benchmark/ }));
    expect(row.getByText('2 prepared snapshots')).toBeVisible();
    expect(row.getByText('Answer exact match / token F1')).toBeVisible();
    await user.selectOptions(row.getByRole('combobox'), 'snapshot-two');
    await user.click(row.getByRole('button', { name: 'Open Team benchmark in Generated answers' }));
    expect(onOpenAnswers).toHaveBeenCalledWith('snapshot-two');
    const unprepared = within(screen.getByRole('row', { name: /Unprepared suite/ }));
    expect(unprepared.getByText('Not prepared')).toBeVisible();
    expect(unprepared.queryByRole('button')).not.toBeInTheDocument();
    expect(api.loadBenchmark).not.toHaveBeenCalled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('preserves generated-answer access for legacy snapshots outside the current catalog', async () => {
    const user = userEvent.setup();
    const onOpenAnswers = vi.fn();
    render(
      <BenchmarkCatalog
        api={apiWith({ benchmarks: {} })}
        snapshots={[{ ...snapshot, configuration: undefined, source: 'Saved evaluation' }]}
        onOpenAnswers={onOpenAnswers}
      />,
    );
    await user.click(
      screen.getByRole('button', { name: 'Open Saved evaluation in Generated answers' }),
    );
    expect(onOpenAnswers).toHaveBeenCalledWith(snapshot.id);
    expect(screen.getByText('Saved snapshot definition')).toBeVisible();
  });

  it.each([
    'javascript:alert(1)',
    'file:///private/dataset.json',
    '/private/dataset.json',
    'http://localhost:4319/secret',
    'http://127.0.0.1/secret',
    'http://[::1]/secret',
    'http://private.local/secret',
    'https://user:credential@example.org/data',
  ])('does not link an unsafe or non-public source: %s', async (source) => {
    const api = apiWith({ benchmarks: { dataset: { ...definition, source, homepage: source } } });
    render(<BenchmarkCatalog api={api} snapshots={[]} />);
    await screen.findByText('Team benchmark');
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(screen.getByText('Source configured in YAML')).toBeVisible();
  });

  it('does not pretend unknown metric kinds have a supported generated-answer integration', async () => {
    const custom = { ...definition, evaluation: 'future_judge_v7' };
    render(
      <BenchmarkCatalog
        api={apiWith({ benchmarks: { 'team-suite': custom } })}
        snapshots={[{ ...snapshot, metric_kind: 'future_judge_v7' }]}
        onOpenAnswers={vi.fn()}
      />,
    );
    expect(await screen.findByText('future_judge_v7')).toBeVisible();
    expect(
      screen.queryByRole('button', { name: /Open.*Generated answers/ }),
    ).not.toBeInTheDocument();
  });

  it('retains prior definitions after a catalog read fails and aborts requests when the tab closes', async () => {
    const api = apiWith({ benchmarks: { 'team-suite': definition } });
    const getCatalog = vi.mocked(api.getCatalog);
    const user = userEvent.setup();
    const view = render(<BenchmarkCatalog api={api} snapshots={[]} pollInterval={60000} />);
    await screen.findByText('Team benchmark');
    getCatalog.mockRejectedValueOnce(new Error('Catalog is unavailable'));
    await user.click(screen.getByRole('button', { name: 'Refresh catalog' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Previously loaded definitions remain visible',
    );
    expect(screen.getByText('Team benchmark')).toBeVisible();
    getCatalog.mockImplementationOnce(() => new Promise(() => {}));
    await user.click(screen.getByRole('button', { name: 'Refresh catalog' }));
    await waitFor(() => expect(getCatalog).toHaveBeenCalledTimes(3));
    const signal = getCatalog.mock.calls[2][0]!;
    view.unmount();
    expect(signal.aborted).toBe(true);
  });
});
