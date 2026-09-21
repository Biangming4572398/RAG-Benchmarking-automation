import { describe, expect, it, vi } from 'vitest';
import { createResultsApi } from './results-api';

const suite = {
  id: 'suite-1',
  run_number: 12,
  request: { architecture_label: 'Dense v2', description: 'Notes', profile_id: null, top_k: null },
  status: 'running',
  started_at_ms: 123,
  finished_at_ms: null,
  error: null,
  items: [
    {
      benchmark: 'ragtruth-qa',
      benchmark_id: 'snapshot-1',
      mode: 'retrieval',
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
      reason: 'Integration required',
    },
  ],
};
function respond(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('results and suite API', () => {
  it('reads backend-owned run numbers and evaluator-specific metric columns', async () => {
    const body = {
      benchmarks: [
        {
          benchmark: 'ragtruth-qa',
          columns: ['run_number', 'metric.new_metric', 'review.correctness'],
          rows: [
            {
              run_number: '14',
              run_id: 'run-14',
              mode: 'retrieval',
              status: 'completed',
              suite_run_number: '12',
              'metric.new_metric': '0',
              'review.correctness': '',
            },
          ],
        },
      ],
    };
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(respond(body));
    expect(await createResultsApi(fetcher).getResults()).toEqual(body);
    expect(fetcher).toHaveBeenCalledWith(
      '/api/benchmarks/v1/results',
      expect.objectContaining({ method: 'GET', cache: 'no-store' }),
    );
  });
  it('starts one suite and accepts nullable Rust options and skipped catalog entries', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(respond(suite, 202));
    const signal = new AbortController().signal;
    const body = { architecture_label: 'Dense v2', description: 'Two\nlines' };
    expect(await createResultsApi(fetcher).startSuite(body, signal)).toEqual(suite);
    expect(fetcher).toHaveBeenCalledWith(
      '/api/benchmarks/v1/suite-runs',
      expect.objectContaining({ method: 'POST', body: JSON.stringify(body), signal }),
    );
    expect(new Headers(fetcher.mock.calls[0][1]?.headers).has('Authorization')).toBe(false);
  });
  it('lists saved suites including interrupted runs', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValue(respond([{ ...suite, status: 'interrupted', finished_at_ms: 456 }]));
    expect(await createResultsApi(fetcher).listSuites()).toHaveLength(1);
  });
  it('rejects malformed results and suite responses', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        respond({ benchmarks: [{ benchmark: 'x', columns: [], rows: [{ run_number: 42 }] }] }),
      )
      .mockResolvedValueOnce(
        respond([{ ...suite, items: [{ ...suite.items[0], status: 'invented' }] }]),
      );
    const api = createResultsApi(fetcher);
    await expect(api.getResults()).rejects.toThrow('Invalid benchmark response');
    await expect(api.listSuites()).rejects.toThrow('Invalid benchmark response');
  });
  it('reports backend conflicts without retrying a suite creation', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValue(respond({ error: 'A benchmark run is already active' }, 409));
    await expect(
      createResultsApi(fetcher).startSuite({ architecture_label: 'Dense' }),
    ).rejects.toMatchObject({ status: 409, message: 'A benchmark run is already active' });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it('downloads the selected benchmark CSV and rejects proxy HTML', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        new Response('run_number\n12', { headers: { 'Content-Type': 'text/csv' } }),
      )
      .mockResolvedValueOnce(new Response('<html/>', { headers: { 'Content-Type': 'text/html' } }));
    const api = createResultsApi(fetcher);
    expect(await (await api.downloadResults('ragtruth-qa')).text()).toBe('run_number\n12');
    expect(fetcher.mock.calls[0][0]).toBe('/api/benchmarks/v1/results/ragtruth-qa/scores.csv');
    await expect(api.downloadResults('ragtruth-qa')).rejects.toThrow('Invalid CSV response');
  });
});
