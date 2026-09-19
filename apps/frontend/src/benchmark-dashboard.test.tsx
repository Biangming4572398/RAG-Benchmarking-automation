// @vitest-environment jsdom
import React from 'react';
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BenchmarkDashboard } from './benchmark-dashboard';
import { type BenchmarkReport } from './benchmark-data';

function reportFixture(): BenchmarkReport {
  return {
    schemaVersion: 1,
    name: 'Evaluation batch',
    sample: false,
    architectures: [
      { id: 'baseline', name: 'Baseline', description: 'Reference design' },
      { id: 'candidate', name: 'Candidate', description: 'Sequential design' },
      { id: 'parallel', name: 'Parallel', description: 'Concurrent design' },
    ],
    benchmarks: [
      {
        id: 'accuracy',
        name: 'Accuracy benchmark',
        suite: 'Correctness',
        metric: 'Accuracy',
        unit: '%',
        direction: 'higher',
      },
      {
        id: 'errors',
        name: 'Error benchmark',
        suite: 'Correctness',
        metric: 'Error rate',
        unit: '%',
        direction: 'lower',
      },
      {
        id: 'reasoning',
        name: 'Reasoning benchmark',
        suite: 'Reasoning',
        metric: 'Points',
        unit: 'points',
        direction: 'higher',
      },
      {
        id: 'recovery',
        name: 'Recovery benchmark',
        suite: 'Reliability',
        metric: 'Success rate',
        unit: '%',
        direction: 'higher',
      },
    ],
    results: [
      { benchmarkId: 'accuracy', architectureId: 'baseline', status: 'completed', value: 80 },
      { benchmarkId: 'accuracy', architectureId: 'candidate', status: 'completed', value: 95 },
      { benchmarkId: 'accuracy', architectureId: 'parallel', status: 'completed', value: 90 },
      { benchmarkId: 'errors', architectureId: 'baseline', status: 'completed', value: 4 },
      { benchmarkId: 'errors', architectureId: 'candidate', status: 'completed', value: 2 },
      { benchmarkId: 'errors', architectureId: 'parallel', status: 'completed', value: 2 },
      { benchmarkId: 'reasoning', architectureId: 'baseline', status: 'completed', value: 0 },
      { benchmarkId: 'reasoning', architectureId: 'candidate', status: 'queued' },
      {
        benchmarkId: 'recovery',
        architectureId: 'baseline',
        status: 'failed',
        note: 'The response did not match the expected structure.',
      },
      { benchmarkId: 'recovery', architectureId: 'candidate', status: 'running' },
      { benchmarkId: 'recovery', architectureId: 'parallel', status: 'completed', value: 75 },
    ],
  };
}

function jsonFile(contents: string, name = 'results.json'): File {
  const file = new File([contents], name, { type: 'application/json' });
  // jsdom's File does not expose the browser's Blob.text implementation.
  Object.defineProperty(file, 'text', { value: async () => contents });
  return file;
}

function readBlob(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsText(blob);
  });
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('architecture comparison dashboard', () => {
  it('compares the full matrix, includes ties, and recalculates best scores among selected architectures', async () => {
    const user = userEvent.setup();
    render(<BenchmarkDashboard initialState={{ report: reportFixture() }} />);

    const table = screen.getByRole('table');
    expect(within(table).getAllByRole('columnheader')).toHaveLength(4);
    expect(within(table).getAllByRole('rowheader')).toHaveLength(4);
    expect(within(table).getAllByRole('cell')).toHaveLength(12);
    expect(
      screen.getByRole('button', { name: 'Accuracy benchmark, Candidate: 95%, best result' }),
    ).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'Error benchmark, Candidate: 2%, best result' }),
    ).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'Error benchmark, Parallel: 2%, best result' }),
    ).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'Reasoning benchmark, Baseline: 0 points, best result' }),
    ).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'Reasoning benchmark, Parallel: Not run' }),
    ).toBeVisible();

    await user.click(screen.getByRole('checkbox', { name: 'Candidate' }));
    expect(within(table).getAllByRole('columnheader')).toHaveLength(3);
    expect(
      screen.getByRole('button', { name: 'Accuracy benchmark, Parallel: 90%, best result' }),
    ).toBeVisible();
    expect(screen.queryByRole('columnheader', { name: /Candidate/ })).not.toBeInTheDocument();

    await user.click(screen.getByRole('checkbox', { name: 'Highlight best in each row' }));
    expect(within(table).queryByRole('button', { name: /best result/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole('checkbox', { name: 'Parallel' }));
    expect(screen.getByRole('checkbox', { name: 'Baseline' })).toBeDisabled();
  });

  it('combines suite, metric search, and cell-status filters and offers recovery from an empty view', async () => {
    const user = userEvent.setup();
    render(<BenchmarkDashboard initialState={{ report: reportFixture() }} />);

    await user.selectOptions(screen.getByLabelText('Filter benchmark suite'), 'Correctness');
    expect(screen.getAllByRole('rowheader')).toHaveLength(2);
    await user.type(screen.getByRole('textbox', { name: 'Search benchmarks' }), 'error rate');
    expect(screen.getAllByRole('rowheader')).toHaveLength(1);
    expect(screen.getByRole('rowheader', { name: /Error benchmark/ })).toBeVisible();
    await user.selectOptions(screen.getByLabelText('Filter result status'), 'running');
    expect(screen.getByRole('heading', { name: 'No matching benchmarks' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Export CSV' })).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(screen.getAllByRole('rowheader')).toHaveLength(4);
    await user.selectOptions(screen.getByLabelText('Filter result status'), 'running');
    expect(screen.getAllByRole('rowheader')).toHaveLength(1);
    expect(screen.getByRole('rowheader', { name: /Recovery benchmark/ })).toBeVisible();
    await user.click(screen.getByRole('checkbox', { name: 'Candidate' }));
    expect(screen.getByRole('heading', { name: 'No matching benchmarks' })).toBeVisible();
    await user.selectOptions(screen.getByLabelText('Filter result status'), 'missing');
    expect(screen.getByRole('rowheader', { name: /Reasoning benchmark/ })).toBeVisible();
  });

  it('paginates large batches and exports every filtered row rather than only the visible page', async () => {
    const user = userEvent.setup();
    const report = reportFixture();
    report.benchmarks = Array.from({ length: 52 }, (_, index) => ({
      ...report.benchmarks[0],
      id: `benchmark-${index}`,
      name: index < 26 ? `Retained ${index + 1}` : `Other ${index + 1}`,
    }));
    report.results = [];
    const createObjectURL = vi.fn<(blob: Blob) => string>(() => 'blob:comparison-csv');
    const OriginalURL = URL;
    vi.stubGlobal(
      'URL',
      class extends OriginalURL {
        static createObjectURL = createObjectURL;
        static revokeObjectURL = vi.fn();
      },
    );
    const download = vi
      .spyOn(HTMLAnchorElement.prototype, 'click')
      .mockImplementation(() => undefined);
    render(<BenchmarkDashboard initialState={{ report }} />);

    expect(screen.getAllByRole('rowheader')).toHaveLength(25);
    expect(screen.getByText('Page 1 of 3')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByText('Page 2 of 3')).toBeVisible();
    await user.type(screen.getByRole('textbox', { name: 'Search benchmarks' }), 'Retained');
    expect(screen.getByText('Page 1 of 2')).toBeVisible();
    expect(screen.queryByRole('rowheader', { name: /Retained 26/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole('checkbox', { name: 'Candidate' }));
    // Reselecting an architecture must not reorder exported columns.
    await user.click(screen.getByRole('checkbox', { name: 'Baseline' }));
    await user.click(screen.getByRole('checkbox', { name: 'Baseline' }));
    await user.click(screen.getByRole('button', { name: 'Export CSV' }));

    expect(download).toHaveBeenCalledOnce();
    const csv = await readBlob(createObjectURL.mock.calls[0][0]);
    expect(csv.split('\r\n')).toHaveLength(27);
    expect(csv).toContain('"Retained 26"');
    expect(csv).not.toContain('"Other ');
    expect(csv.split('\r\n')[0]).toContain('"Baseline","Parallel"');
    expect(csv).not.toContain('"Candidate"');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Exported 26 rows across 2 architectures.',
    );
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getAllByRole('rowheader')).toHaveLength(1);
    expect(screen.getByRole('rowheader', { name: /Retained 26/ })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
  });

  it('replaces illustrative data on valid import and preserves the current table on invalid import', async () => {
    const user = userEvent.setup();
    render(<BenchmarkDashboard />);
    expect(screen.getByText('Sample data')).toBeVisible();
    expect(screen.getAllByRole('rowheader')).toHaveLength(12);
    await user.type(screen.getByRole('textbox', { name: 'Search benchmarks' }), 'Exact match');
    await user.upload(
      screen.getByLabelText('Import benchmark results'),
      jsonFile(JSON.stringify(reportFixture())),
    );

    expect(await screen.findByRole('status')).toHaveTextContent(
      'Imported 4 benchmarks across 3 architectures.',
    );
    expect(screen.getByText('Imported results')).toBeVisible();
    expect(screen.queryByText('Sample data')).not.toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Search benchmarks' })).toHaveValue('');
    expect(screen.getAllByRole('rowheader')).toHaveLength(4);
    expect(screen.getByRole('heading', { name: 'Evaluation batch' })).toBeVisible();

    await user.upload(
      screen.getByLabelText('Import benchmark results'),
      jsonFile('{invalid', 'invalid.json'),
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('not valid JSON');
    expect(screen.getByRole('heading', { name: 'Evaluation batch' })).toBeVisible();
    expect(screen.getAllByRole('rowheader')).toHaveLength(4);
    expect(
      screen.getByRole('button', { name: 'Accuracy benchmark, Candidate: 95%, best result' }),
    ).toBeVisible();
  });

  it('opens result details with failure notes and closes through both its button and native cancel event', async () => {
    const user = userEvent.setup();
    const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
      configurable: true,
      value: function (this: HTMLDialogElement) {
        this.setAttribute('open', '');
      },
    });
    try {
      render(<BenchmarkDashboard initialState={{ report: reportFixture() }} />);
      await user.click(
        screen.getByRole('button', { name: 'Recovery benchmark, Baseline: Failed' }),
      );
      const dialog = screen.getByRole('dialog', { name: 'Recovery benchmark' });
      expect(dialog).toBeVisible();
      expect(
        within(dialog).getByText('The response did not match the expected structure.'),
      ).toBeVisible();
      expect(within(dialog).getByText('Success rate')).toBeVisible();
      await user.click(within(dialog).getByRole('button', { name: 'Close result details' }));
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

      await user.click(
        screen.getByRole('button', { name: 'Error benchmark, Parallel: 2%, best result' }),
      );
      const completedDialog = screen.getByRole('dialog', { name: 'Error benchmark' });
      expect(within(completedDialog).getByText('Lower is better')).toBeVisible();
      expect(within(completedDialog).getByText('2%')).toBeVisible();
      fireEvent(completedDialog, new Event('cancel', { bubbles: false }));
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    } finally {
      if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original);
      else delete (HTMLDialogElement.prototype as Partial<HTMLDialogElement>).showModal;
    }
  });

  it('restores a panel’s report, filters, selected architectures, and display preference on remount', async () => {
    const user = userEvent.setup();
    const onStateChange = vi.fn();
    const first = render(
      <BenchmarkDashboard
        initialState={{ report: reportFixture() }}
        onStateChange={onStateChange}
      />,
    );
    await user.selectOptions(screen.getByLabelText('Filter benchmark suite'), 'Correctness');
    await user.selectOptions(screen.getByLabelText('Filter result status'), 'completed');
    await user.type(screen.getByRole('textbox', { name: 'Search benchmarks' }), 'Accuracy');
    await user.click(screen.getByRole('checkbox', { name: 'Candidate' }));
    await user.click(screen.getByRole('checkbox', { name: 'Highlight best in each row' }));
    await waitFor(() => expect(onStateChange).toHaveBeenCalled());
    const saved = onStateChange.mock.calls.at(-1)![0];
    first.unmount();

    const restored = render(<BenchmarkDashboard initialState={saved} />);
    expect(screen.getByRole('heading', { name: 'Evaluation batch' })).toBeVisible();
    expect(screen.getByRole('textbox', { name: 'Search benchmarks' })).toHaveValue('Accuracy');
    expect(screen.getByLabelText('Filter benchmark suite')).toHaveValue('Correctness');
    expect(screen.getByLabelText('Filter result status')).toHaveValue('completed');
    expect(screen.getByRole('checkbox', { name: 'Candidate' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Highlight best in each row' })).not.toBeChecked();
    expect(screen.getAllByRole('rowheader')).toHaveLength(1);
    expect(screen.getAllByRole('columnheader')).toHaveLength(3);
    restored.unmount();

    render(<BenchmarkDashboard />);
    expect(screen.getByText('Sample data')).toBeVisible();
    expect(screen.getByRole('textbox', { name: 'Search benchmarks' })).toHaveValue('');
    expect(screen.getAllByRole('rowheader')).toHaveLength(12);
  });
});
