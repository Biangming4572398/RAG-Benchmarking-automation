import { describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  comparabilityKey,
  createBenchmarkApi,
  type BenchmarkInfo,
  type BenchmarkRun,
  type BenchmarkSnapshot,
  type Catalog,
} from './benchmark-api';

const catalog: Catalog = {
  benchmarks: {
    'ragtruth-qa': {
      name: 'RAGTruth QA',
      source: 'hf://datasets/wandb/RAGTruth-processed/data/{split}-*.parquet',
      split: 'test',
      adapter: 'ragtruth_qa',
      evaluation: 'paired_context_recovery_v1',
      defaults: { limit: 100, top_k: 8 },
    },
  },
};

const benchmark: BenchmarkInfo = {
  id: '6a21bb00-7cfe-4978-b1a2-332f14856504',
  source: 'fixture.parquet',
  split: 'test',
  metric_kind: 'paired_context_recovery_v1',
  case_count: 3,
  document_count: 2,
  corpus_path: '/tmp/experiment/benchmarks/fixture/corpus',
  fingerprint: 'fingerprint-a',
  configuration: { key: 'ragtruth-qa', definition: catalog.benchmarks['ragtruth-qa'] },
};

function run(overrides: Partial<BenchmarkRun> = {}): BenchmarkRun {
  return {
    id: '5499f7b1-fe3e-4aa0-85bc-4db7dcfa9467',
    request: { benchmark_id: benchmark.id, top_k: 8, label: 'Baseline' },
    metric_kind: 'paired_context_recovery_v1',
    benchmark_fingerprint: benchmark.fingerprint,
    status: 'completed',
    started_at_ms: 1_790_000_000_000,
    finished_at_ms: 1_790_000_001_000,
    total: 3,
    completed: 3,
    failed: 0,
    means: { context_hit_at_k: 1, reciprocal_rank_at_k: 0.5, ndcg_at_k: 0.63 },
    error: null,
    scope: { projectId: 'project-a', corpusId: 'corpus-a' },
    watermark: { generation: 'generation-a' },
    source_ids: ['source-a', 'source-b'],
    ...overrides,
  };
}

const snapshot: BenchmarkSnapshot = {
  id: benchmark.id,
  source: benchmark.source,
  split: benchmark.split,
  metric_kind: benchmark.metric_kind,
  cases: [
    {
      id: 'case-a',
      query: 'Where is the evidence?',
      document_id: 'document-a',
      reference_outputs: [
        {
          id: 'original-output',
          output: 'A historical response',
          model: 'historical-model',
          quality: 'good',
          hallucination_labels: [],
        },
      ],
    },
  ],
  documents: [{ id: 'document-a', filename: 'ragtruth-a.md', text: 'Evidence', revision: 'a' }],
};

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('benchmark HTTP contract', () => {
  it('does not contact the server on construction', () => {
    const fetcher = vi.fn<typeof fetch>();
    createBenchmarkApi(fetcher);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it('reads the catalog, summary arrays, saved snapshot and run detail using exact routes', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(catalog))
      .mockResolvedValueOnce(jsonResponse([benchmark]))
      .mockResolvedValueOnce(jsonResponse(snapshot))
      .mockResolvedValueOnce(jsonResponse([run()]))
      .mockResolvedValueOnce(jsonResponse(run()));
    const api = createBenchmarkApi(fetcher);
    const signal = new AbortController().signal;
    await expect(api.getCatalog(signal)).resolves.toEqual(catalog);
    await expect(api.listBenchmarks()).resolves.toEqual([benchmark]);
    await expect(api.getBenchmark(benchmark.id)).resolves.toEqual(snapshot);
    await expect(api.listRuns()).resolves.toEqual([run()]);
    await expect(api.getRun(run().id)).resolves.toEqual(run());
    expect(fetcher.mock.calls.map(([url]) => url)).toEqual([
      '/api/benchmarks/v1/catalog',
      '/api/benchmarks/v1/benchmarks',
      `/api/benchmarks/v1/benchmarks/${benchmark.id}`,
      '/api/benchmarks/v1/runs',
      `/api/benchmarks/v1/runs/${run().id}`,
    ]);
    expect(fetcher.mock.calls[0][1]).toMatchObject({ method: 'GET', cache: 'no-store', signal });
    for (const [, options] of fetcher.mock.calls) {
      expect(options?.headers).not.toHaveProperty('Authorization');
      expect(options?.body).toBeUndefined();
    }
  });

  it('posts snake_case fields and preserves an omitted top_k to use the saved snapshot default', async () => {
    const active = run({ status: 'running', completed: 0, means: null, finished_at_ms: null });
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(benchmark, 201))
      .mockResolvedValueOnce(jsonResponse(active, 202));
    const api = createBenchmarkApi(fetcher);
    await expect(api.loadBenchmark({ benchmark: 'ragtruth-qa', limit: 3 })).resolves.toEqual(
      benchmark,
    );
    await expect(
      api.startRun({ benchmark_id: benchmark.id, label: 'Architecture A' }),
    ).resolves.toEqual(active);
    expect(fetcher.mock.calls[0][1]).toMatchObject({
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ benchmark: 'ragtruth-qa', limit: 3 }),
    });
    expect(fetcher.mock.calls[1][1]?.body).toBe(
      JSON.stringify({ benchmark_id: benchmark.id, label: 'Architecture A' }),
    );
  });

  it('retains genuine zero metrics and partial failed results without inventing scores', async () => {
    const rows = [
      run({ means: { context_hit_at_k: 0, reciprocal_rank_at_k: 0, ndcg_at_k: 0 } }),
      run({ status: 'failed', completed: 0, failed: 1, means: null, error: 'Nebula timed out' }),
      run({ status: 'interrupted', completed: 1, error: 'Server stopped' }),
    ];
    const api = createBenchmarkApi(vi.fn<typeof fetch>().mockResolvedValue(jsonResponse(rows)));
    await expect(api.listRuns()).resolves.toEqual(rows);
  });

  it.each([
    ['catalog arrays', [], 'catalog'],
    ['summary wrapper', { benchmarks: [benchmark] }, 'benchmarks'],
    ['unknown run status', [run({ status: 'queued' as BenchmarkRun['status'] })], 'runs'],
    ['missing score field', [{ ...run(), means: { context_hit_at_k: 0.8 } }], 'runs'],
    ['invalid counts', [run({ completed: 4 })], 'runs'],
  ])('rejects malformed %s', async (_name, value, endpoint) => {
    const api = createBenchmarkApi(vi.fn<typeof fetch>().mockResolvedValue(jsonResponse(value)));
    const result =
      endpoint === 'catalog'
        ? api.getCatalog()
        : endpoint === 'benchmarks'
          ? api.listBenchmarks()
          : api.listRuns();
    await expect(result).rejects.toMatchObject({
      name: 'ApiError',
      status: 200,
      message: 'Invalid benchmark response. Check the server connection.',
    });
  });

  it('reports misrouted HTML as a connection error instead of treating it as data', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response('<html>Vite shell</html>'));
    await expect(createBenchmarkApi(fetcher).getCatalog()).rejects.toBeInstanceOf(ApiError);
  });

  it.each([
    [401, { error: 'Bearer token required' }, 'Bearer token required'],
    [409, { error: 'A benchmark run is already active' }, 'A benchmark run is already active'],
    [404, { error: 'This run produced no CSV rows' }, 'This run produced no CSV rows'],
    [
      503,
      { error: 'Configure NEBULA_API_BASE and NEBULA_API_TOKEN before starting runs' },
      'Configure NEBULA_API_BASE and NEBULA_API_TOKEN before starting runs',
    ],
  ])('preserves the server error for HTTP %s', async (status, body, message) => {
    const api = createBenchmarkApi(
      vi.fn<typeof fetch>().mockResolvedValue(jsonResponse(body, status)),
    );
    await expect(api.listRuns()).rejects.toMatchObject({ name: 'ApiError', status, message });
  });

  it('handles Axum plain text errors and avoids displaying an HTML error document', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response('Failed to parse\n JSON body', { status: 422 }))
      .mockResolvedValueOnce(new Response('<html>Bad gateway</html>', { status: 502 }));
    const api = createBenchmarkApi(fetcher);
    await expect(api.listRuns()).rejects.toMatchObject({
      status: 422,
      message: 'Failed to parse JSON body',
    });
    await expect(api.listRuns()).rejects.toMatchObject({
      status: 502,
      message: 'Benchmark server returned HTTP 502.',
    });
  });

  it('preserves abort errors so cancelled requests are not displayed as server failures', async () => {
    const aborted = new DOMException('Aborted', 'AbortError');
    const api = createBenchmarkApi(vi.fn<typeof fetch>().mockRejectedValue(aborted));
    await expect(api.listRuns()).rejects.toBe(aborted);
  });

  it('downloads the server CSV including partial results, with encoded identifiers', async () => {
    const csv = 'case_id,context_hit_at_k\ncase-a,0\n';
    const signal = new AbortController().signal;
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValue(
        new Response(csv, { headers: { 'Content-Type': 'text/csv; charset=utf-8' } }),
      );
    const downloaded = await createBenchmarkApi(fetcher).downloadScores('run/id', signal);
    expect(await downloaded.text()).toBe(csv);
    expect(fetcher).toHaveBeenCalledWith('/api/benchmarks/v1/runs/run%2Fid/scores.csv', {
      headers: { Accept: 'text/csv' },
      cache: 'no-store',
      signal,
    });
  });

  it('rejects unavailable CSV and misrouted downloads', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse({ error: 'This run produced no CSV rows' }, 404))
      .mockResolvedValueOnce(
        new Response('<html>shell</html>', { headers: { 'Content-Type': 'text/html' } }),
      );
    const api = createBenchmarkApi(fetcher);
    await expect(api.downloadScores('run-a')).rejects.toMatchObject({ status: 404 });
    await expect(api.downloadScores('run-a')).rejects.toMatchObject({
      message: 'Invalid CSV response. Check the server connection.',
    });
  });
});

describe('comparable completed results', () => {
  it('matches candidate sets independently of ordering, label, run or snapshot IDs', () => {
    const other = run({
      id: 'another-run',
      request: { benchmark_id: 'another-snapshot', top_k: 8, label: 'Architecture B' },
      source_ids: ['source-b', 'source-a', 'source-b'],
    });
    expect(comparabilityKey(run())).not.toBeNull();
    expect(comparabilityKey(other)).toBe(comparabilityKey(run()));
  });

  it.each([
    { metric_kind: 'another_metric' },
    { benchmark_fingerprint: 'another-fingerprint' },
    { request: { benchmark_id: benchmark.id, top_k: 4, label: 'Baseline' } },
    { source_ids: ['source-a', 'source-c'] },
  ])('separates different comparison conditions: %j', (changes) => {
    expect(comparabilityKey(run(changes))).not.toBe(comparabilityKey(run()));
  });

  it.each<Partial<BenchmarkRun>>([
    { status: 'running' },
    { status: 'failed' },
    { status: 'interrupted' },
    { completed: 2 },
    { failed: 1 },
    { total: 0, completed: 0 },
    { means: null },
    { metric_kind: '' },
    { benchmark_fingerprint: '' },
    { source_ids: [] },
    { source_ids: [''] },
    { request: { benchmark_id: benchmark.id, top_k: 0, label: 'Baseline' } },
  ])('excludes incomplete or unidentified results: %j', (changes) => {
    expect(comparabilityKey(run(changes))).toBeNull();
  });
});
