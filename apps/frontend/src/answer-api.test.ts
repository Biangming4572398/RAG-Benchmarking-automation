import { describe, expect, it, vi } from 'vitest';
import { ApiError } from './benchmark-api';
import {
  answerComparabilityKey,
  createAnswerApi,
  type AnswerCase,
  type AnswerReviewRequest,
  type AnswerRun,
  type AnswerRunSummary,
  type AnswerRuntime,
  type StartAnswerRunRequest,
} from './answer-api';

const runtime: AnswerRuntime = {
  available: true,
  reason: null,
  embedding_model: { id: 'intfloat/multilingual-e5-small', revision: 'model-revision' },
  profiles: [
    { id: 'configured-model', label: 'Configured generator', enabled: true, disabled_reason: null },
  ],
};
const startRequest: StartAnswerRunRequest = {
  benchmark_id: 'snapshot-a',
  architecture_label: 'Dense E5 baseline',
  profile_id: 'configured-model',
};
const review: AnswerReviewRequest = {
  reviewer: 'Researcher',
  correctness: true,
  groundedness: true,
  hallucination: false,
  citation_accuracy: true,
  notes: 'The cited source directly supports the answer.',
};

function summary(changes: Partial<AnswerRunSummary> = {}): AnswerRunSummary {
  return {
    id: 'run-a',
    request: startRequest,
    benchmark_fingerprint: 'snapshot-fingerprint',
    evaluation: 'manual_review_v1',
    embedding_model: runtime.embedding_model!,
    generation_model: { profile_id: 'configured-model', label: 'Configured generator' },
    top_k: 8,
    status: 'completed',
    started_at_ms: 1_790_000_000_000,
    finished_at_ms: 1_790_000_001_000,
    total: 1,
    completed: 1,
    failed: 0,
    answered: 1,
    reviewed: 1,
    means: { correctness: 1, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
    error: null,
    scope: { projectId: 'project-a', corpusId: 'corpus-a' },
    watermark: { generation: 'generation-a' },
    source_ids: ['source-a', 'source-b'],
    ...changes,
  };
}

function answerCase(changes: Partial<AnswerCase> = {}): AnswerCase {
  return {
    case_id: 'case-a',
    query: 'Where is the source evidence?',
    status: 'ok',
    outcome: 'answered',
    answer: 'The source provides the evidence [1].',
    reason: null,
    evidence: [
      {
        id: 'evidence-a',
        ordinal: 1,
        sourceId: 'source-a',
        sourceTitle: 'RAGTruth context',
        sourceRevision: 'source-revision',
        location: 'Lines 1–3',
        excerpt: 'The source provides the evidence.',
      },
    ],
    lineage: [
      { id: 'claim-a', claim: 'The source provides the evidence.', evidenceIds: ['evidence-a'] },
    ],
    model_receipt: {
      profileId: 'configured-model',
      route: 'configured-route',
      modelLabel: 'Configured generator',
    },
    latency_ms: 1500,
    error: null,
    review: { ...review, reviewed_at_ms: 1_790_000_002_000 },
    ...changes,
  };
}

function detail(changes: Partial<AnswerRun> = {}): AnswerRun {
  return { ...summary(), cases: [answerCase()], ...changes };
}

function hotpotSummary(changes: Partial<AnswerRunSummary> = {}): AnswerRunSummary {
  return summary({
    evaluation: 'hotpotqa_answer_v1',
    reviewed: 0,
    means: null,
    scored: 1,
    automatic_scores: { exact_match: 1, f1: 1 },
    ...changes,
  });
}

function hotpotCase(changes: Partial<AnswerCase> = {}): AnswerCase {
  return answerCase({
    review: null,
    reference_answer: 'The source provides the evidence.',
    automatic_scores: { exact_match: 1, f1: 1 },
    ...changes,
  });
}

function hotpotDetail(changes: Partial<AnswerRun> = {}): AnswerRun {
  return { ...hotpotSummary(), cases: [hotpotCase()], ...changes };
}

function response(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('answer-quality HTTP transport', () => {
  it('reads runtime, summaries and detail through exact same-origin paths with no credentials', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(response(runtime))
      .mockResolvedValueOnce(response([summary()]))
      .mockResolvedValueOnce(response(detail()));
    const api = createAnswerApi(fetcher);
    expect(fetcher).not.toHaveBeenCalled();
    const signal = new AbortController().signal;
    await expect(api.getRuntime(signal)).resolves.toEqual(runtime);
    await expect(api.listRuns()).resolves.toEqual([summary()]);
    await expect(api.getRun('run/a?x=1')).resolves.toEqual(detail());
    expect(fetcher.mock.calls.map(([url]) => url)).toEqual([
      '/api/benchmarks/v1/answer-runtime',
      '/api/benchmarks/v1/answer-runs',
      '/api/benchmarks/v1/answer-runs/run%2Fa%3Fx%3D1',
    ]);
    expect(fetcher.mock.calls[0][1]).toMatchObject({ method: 'GET', cache: 'no-store', signal });
    for (const [, options] of fetcher.mock.calls) {
      expect(new Headers(options?.headers).has('Authorization')).toBe(false);
      expect(options?.credentials).toBeUndefined();
      expect(options?.body).toBeUndefined();
    }
  });

  it('posts exact start and review bodies and encodes both route identifiers', async () => {
    const active = summary({
      status: 'running',
      completed: 0,
      answered: 0,
      reviewed: 0,
      means: null,
      finished_at_ms: null,
    });
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(response(active, 202))
      .mockResolvedValueOnce(response(detail()));
    const api = createAnswerApi(fetcher);
    const signal = new AbortController().signal;
    await expect(api.startRun(startRequest, signal)).resolves.toEqual(active);
    await expect(api.saveReview('run/a', 'case?b/#', review, signal)).resolves.toEqual(detail());
    expect(fetcher.mock.calls.map(([url]) => url)).toEqual([
      '/api/benchmarks/v1/answer-runs',
      '/api/benchmarks/v1/answer-runs/run%2Fa/cases/case%3Fb%2F%23/review',
    ]);
    for (const [index, body] of [startRequest, review].entries()) {
      expect(fetcher.mock.calls[index][1]).toEqual({
        method: 'POST',
        headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
        cache: 'no-store',
        signal,
        body: JSON.stringify(body),
      });
    }
  });

  it('preserves refusal coverage and pending reviews without assigning invented zero scores', async () => {
    const rows = [
      summary({ answered: 0, reviewed: 0, means: null }),
      summary({ reviewed: 0, means: null }),
      summary({
        means: { correctness: 0, groundedness: 0, hallucination_rate: 1, citation_accuracy: 0 },
      }),
      summary({
        status: 'failed',
        completed: 0,
        failed: 1,
        answered: 0,
        reviewed: 0,
        means: null,
        error: 'Generation timed out',
      }),
      summary({ status: 'interrupted', total: 2, reviewed: 0, means: null }),
    ];
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(rows)));
    await expect(api.listRuns()).resolves.toEqual(rows);
  });

  it('accepts unavailable runtime information without substituting a generator', async () => {
    const unavailable = {
      available: false,
      reason: 'No configured generator',
      embedding_model: null,
      profiles: [],
    };
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(unavailable)));
    await expect(api.getRuntime()).resolves.toEqual(unavailable);
  });

  it('keeps failed case diagnostics when receipt or evidence pinning failed', async () => {
    const failed = detail({
      status: 'failed',
      completed: 0,
      failed: 1,
      answered: 0,
      reviewed: 0,
      means: null,
      error: 'Evidence pinning failed',
      cases: [
        answerCase({
          status: 'error',
          outcome: 'answered',
          error: 'Evidence pinning failed',
          review: null,
          model_receipt: null,
          lineage: [{ id: 'claim-a', claim: 'Returned claim', evidenceIds: ['unknown-evidence'] }],
        }),
      ],
    });
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(failed)));
    await expect(api.getRun('run-a')).resolves.toEqual(failed);
    expect(answerComparabilityKey(failed)).toBeNull();
  });

  it('keeps successful refusals without reviews or made-up quality metrics', async () => {
    const refused = detail({
      answered: 0,
      reviewed: 0,
      means: null,
      cases: [answerCase({ outcome: 'refused', answer: null, review: null, model_receipt: null })],
    });
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(refused)));
    await expect(api.getRun('run-a')).resolves.toEqual(refused);
    expect(answerComparabilityKey(refused)).toBeNull();
  });

  it.each([
    { ...runtime, available: 'true' },
    { ...runtime, embedding_model: { id: 'e5' } },
    { ...runtime, embedding_model: null },
    { ...runtime, profiles: [] },
    { ...runtime, profiles: [{ ...runtime.profiles[0], enabled: 'yes' }] },
    { ...runtime, profiles: [{ ...runtime.profiles[0], disabled_reason: 1 }] },
  ])('rejects an invalid runtime shape', async (value) => {
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(value)));
    await expect(api.getRuntime()).rejects.toBeInstanceOf(ApiError);
  });

  it.each([
    { ...summary(), means: { ...summary().means, correctness: 1.01 } },
    { ...summary(), means: { ...summary().means, hallucination_rate: -0.1 } },
    { ...summary(), means: { ...summary().means, citation_accuracy: '1' } },
    { ...summary(), means: { correctness: 1 } },
    { ...summary(), answered: 2 },
    { ...summary(), reviewed: 2 },
    { ...summary(), completed: 2 },
    { ...summary(), failed: 1 },
    { ...summary(), completed: 0.5 },
    { ...summary(), reviewed: 0 },
    { ...summary(), means: null },
    { ...summary(), top_k: 4 },
    { ...summary(), embedding_model: null },
    { ...summary(), generation_model: { profile_id: 'model' } },
    { ...summary(), status: 'queued' },
  ])('rejects invalid summary metrics, provenance or coverage', async (value) => {
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response([value])));
    await expect(api.listRuns()).rejects.toMatchObject({
      name: 'ApiError',
      status: 200,
      message: 'Invalid answer-quality response. Check the server connection.',
    });
  });

  it.each([
    { ...answerCase(), evidence: [{ ...answerCase().evidence[0], excerpt: 5 }] },
    { ...answerCase(), evidence: [{ ...answerCase().evidence[0], ordinal: -1 }] },
    { ...answerCase(), lineage: [{ id: 'claim', claim: 'Claim', evidenceIds: ['missing'] }] },
    { ...answerCase(), lineage: [{ id: 'claim', claim: 'Claim', evidenceIds: [1] }] },
    { ...answerCase(), review: { ...answerCase().review, correctness: 'yes' } },
    { ...answerCase(), review: { ...answerCase().review, hallucination: 0 } },
    { ...answerCase(), review: { ...answerCase().review, reviewer: ' ' } },
    { ...answerCase(), review: { ...answerCase().review, notes: '界'.repeat(1334) } },
    { ...answerCase(), outcome: 'refused' },
    { ...answerCase(), model_receipt: { profileId: 'generator' } },
    { ...answerCase(), model_receipt: null },
  ])('rejects malformed evidence, lineage, receipts and human reviews', async (value) => {
    const api = createAnswerApi(
      vi.fn<typeof fetch>().mockResolvedValue(response({ ...detail(), cases: [value] })),
    );
    await expect(api.getRun('run-a')).rejects.toBeInstanceOf(ApiError);
  });

  it('rejects detail whose finished cases do not match its coverage counts', async () => {
    const api = createAnswerApi(
      vi.fn<typeof fetch>().mockResolvedValue(response(detail({ cases: [] }))),
    );
    await expect(api.getRun('run-a')).rejects.toBeInstanceOf(ApiError);
  });

  it('enforces UTF-8 label and reviewer limits before sending mutations', async () => {
    const fetcher = vi.fn<typeof fetch>();
    const api = createAnswerApi(fetcher);
    await expect(
      api.startRun({ ...startRequest, architecture_label: '界'.repeat(86) }),
    ).rejects.toMatchObject({ status: 400 });
    await expect(api.startRun({ ...startRequest, architecture_label: ' ' })).rejects.toMatchObject({
      status: 400,
    });
    await expect(
      api.saveReview('run-a', 'case-a', { ...review, reviewer: '界'.repeat(43) }),
    ).rejects.toMatchObject({ status: 400 });
    await expect(
      api.saveReview('run-a', 'case-a', { ...review, notes: '界'.repeat(1334) }),
    ).rejects.toMatchObject({ status: 400 });
    expect(fetcher).not.toHaveBeenCalled();
  });

  it.each([
    [400, { error: 'Only answered cases can be reviewed' }, 'Only answered cases can be reviewed'],
    [404, { error: 'Answer run not found' }, 'Answer run not found'],
    [409, { error: 'A benchmark run is already active' }, 'A benchmark run is already active'],
    [503, { error: 'No generation profile is enabled' }, 'No generation profile is enabled'],
  ])('preserves the server explanation for HTTP %s', async (status, body, message) => {
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(body, status)));
    await expect(api.listRuns()).rejects.toMatchObject({ name: 'ApiError', status, message });
  });

  it('identifies an older server without answer routes and hides HTML error content', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response('', { status: 404 }))
      .mockResolvedValueOnce(new Response('<html>Bad gateway</html>', { status: 502 }))
      .mockResolvedValueOnce(new Response('<html>Vite shell</html>', { status: 200 }));
    const api = createAnswerApi(fetcher);
    await expect(api.getRuntime()).rejects.toMatchObject({
      status: 404,
      message: expect.stringContaining('does not support answer-quality runs'),
    });
    await expect(api.listRuns()).rejects.toMatchObject({
      status: 502,
      message: 'Benchmark server returned HTTP 502.',
    });
    await expect(api.listRuns()).rejects.toMatchObject({
      status: 200,
      message: 'Invalid answer-quality response. Check the server connection.',
    });
  });

  it('preserves cancellation and connection errors', async () => {
    const aborted = new DOMException('Aborted', 'AbortError');
    const offline = new TypeError('Failed to fetch');
    const fetcher = vi
      .fn<typeof fetch>()
      .mockRejectedValueOnce(aborted)
      .mockRejectedValueOnce(offline);
    const api = createAnswerApi(fetcher);
    await expect(api.getRuntime()).rejects.toBe(aborted);
    await expect(api.listRuns()).rejects.toBe(offline);
  });

  it('downloads current review CSV via an encoded same-origin run URL', async () => {
    const csv = 'case_id,correctness\ncase-a,true\n';
    const signal = new AbortController().signal;
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValue(
        new Response(csv, { headers: { 'Content-Type': 'text/csv; charset=utf-8' } }),
      );
    const blob = await createAnswerApi(fetcher).downloadScores('run/a', signal);
    expect(await blob.text()).toBe(csv);
    expect(fetcher).toHaveBeenCalledWith('/api/benchmarks/v1/answer-runs/run%2Fa/scores.csv', {
      method: 'GET',
      headers: { Accept: 'text/csv' },
      cache: 'no-store',
      signal,
    });
  });

  it('rejects active-run CSV conflicts and non-CSV downloads', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(response({ error: 'The run is still active' }, 409))
      .mockResolvedValueOnce(
        new Response('<html>shell</html>', { headers: { 'Content-Type': 'text/html' } }),
      );
    const api = createAnswerApi(fetcher);
    await expect(api.downloadScores('run-a')).rejects.toMatchObject({
      status: 409,
      message: 'The run is still active',
    });
    await expect(api.downloadScores('run-a')).rejects.toMatchObject({
      message: 'Invalid CSV response. Check the server connection.',
    });
  });
});

describe('answer-quality comparability', () => {
  it('compares fully reviewed matching data across different architectures and model choices', () => {
    const candidate = summary({
      id: 'run-b',
      request: {
        benchmark_id: 'snapshot-b',
        architecture_label: 'Reranked candidate',
        profile_id: 'another-model',
      },
      embedding_model: { id: 'another-embedder', revision: 'other-revision' },
      generation_model: { profile_id: 'another-model', label: 'Another generator' },
      source_ids: ['source-b', 'source-a', 'source-b'],
    });
    expect(answerComparabilityKey(summary())).not.toBeNull();
    expect(answerComparabilityKey(candidate)).toBe(answerComparabilityKey(summary()));
  });

  it.each<Partial<AnswerRunSummary>>([
    { status: 'running' },
    { status: 'failed' },
    { status: 'interrupted' },
    { total: 2 },
    { failed: 1 },
    { total: 0, completed: 0, answered: 0, reviewed: 0, means: null },
    { answered: 0, reviewed: 0, means: null },
    { reviewed: 0, means: null },
    { means: null },
    { benchmark_fingerprint: '' },
    { source_ids: [] },
    { source_ids: [''] },
    { means: { correctness: 1.1, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 } },
  ])('excludes partial, unreviewed, or invalid results: %j', (changes) => {
    expect(answerComparabilityKey(summary(changes))).toBeNull();
  });

  it.each<Partial<AnswerRunSummary>>([
    { benchmark_fingerprint: 'different-data' },
    { source_ids: ['source-a', 'source-c'] },
  ])('separates different datasets or candidate source sets: %j', (changes) => {
    expect(answerComparabilityKey(summary(changes))).not.toBe(answerComparabilityKey(summary()));
  });
});

describe('HotpotQA answer scoring contract', () => {
  it('accepts an unstarted run with omitted zero coverage and completed automatic scores', async () => {
    const initial = hotpotSummary({
      status: 'running',
      completed: 0,
      answered: 0,
      scored: undefined,
      automatic_scores: { exact_match: 0, f1: 0 },
      finished_at_ms: null,
    });
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(response(initial, 202))
      .mockResolvedValueOnce(response(hotpotDetail()));
    const api = createAnswerApi(fetcher);
    await expect(api.startRun(startRequest)).resolves.toEqual(initial);
    await expect(api.getRun('run-a')).resolves.toEqual(hotpotDetail());
  });

  it('keeps partial automatic means over the full dataset denominator', async () => {
    const partial = hotpotDetail({
      total: 2,
      status: 'running',
      automatic_scores: { exact_match: 0.5, f1: 0.5 },
      finished_at_ms: null,
    });
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(partial)));
    await expect(api.getRun('run-a')).resolves.toEqual(partial);
    expect(answerComparabilityKey(partial)).toBeNull();
  });

  it.each(['running', 'interrupted'] as const)(
    'accepts floating-point rounding at the partial %s scoring ceiling',
    async (status) => {
      const partial = hotpotSummary({
        status,
        total: 10,
        completed: 3,
        answered: 3,
        scored: 3,
        automatic_scores: { exact_match: 0.30000000000000004, f1: 0.30000000000000004 },
      });
      const fetcher = vi
        .fn<typeof fetch>()
        .mockResolvedValueOnce(response([partial]))
        .mockResolvedValueOnce(
          response([
            {
              ...partial,
              automatic_scores: { exact_match: 0.30001, f1: 0.3 },
            },
          ]),
        );
      const api = createAnswerApi(fetcher);
      await expect(api.listRuns()).resolves.toEqual([partial]);
      await expect(api.listRuns()).rejects.toBeInstanceOf(ApiError);
    },
  );

  it.each(['refused', 'evidence-only', 'not-ready', 'error'] as const)(
    'counts an attempted %s case with automatic zero scores',
    async (outcome) => {
      const failed = outcome === 'error';
      const run = hotpotDetail({
        status: failed ? 'failed' : 'completed',
        completed: failed ? 0 : 1,
        failed: failed ? 1 : 0,
        answered: 0,
        automatic_scores: { exact_match: 0, f1: 0 },
        cases: [
          hotpotCase({
            status: failed ? 'error' : 'ok',
            outcome,
            answer: null,
            model_receipt: null,
            automatic_scores: { exact_match: 0, f1: 0 },
          }),
        ],
      });
      const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(run)));
      await expect(api.getRun('run-a')).resolves.toEqual(run);
      if (failed) expect(answerComparabilityKey(run)).toBeNull();
      else expect(answerComparabilityKey(run)).toBe(answerComparabilityKey(hotpotSummary()));
    },
  );

  it('keeps human review metrics separate from automatic answer scores', async () => {
    const run = hotpotDetail({
      reviewed: 1,
      means: { correctness: 0, groundedness: 1, hallucination_rate: 0, citation_accuracy: 1 },
      cases: [hotpotCase({ review: { ...review, correctness: false, reviewed_at_ms: 12 } })],
    });
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response(run)));
    await expect(api.getRun('run-a')).resolves.toEqual(run);
    expect(answerComparabilityKey(run)).toBe(answerComparabilityKey(hotpotSummary()));
  });

  it.each([
    { ...hotpotSummary(), automatic_scores: undefined },
    { ...hotpotSummary(), automatic_scores: { exact_match: 1 } },
    { ...hotpotSummary(), automatic_scores: { exact_match: -0.1, f1: 1 } },
    { ...hotpotSummary(), automatic_scores: { exact_match: 1, f1: 1.01 } },
    { ...hotpotSummary(), automatic_scores: { exact_match: 1, f1: '1' } },
    { ...hotpotSummary(), scored: 0 },
    { ...hotpotSummary(), scored: undefined },
    { ...hotpotSummary(), scored: 2 },
    { ...hotpotSummary(), scored: 0.5 },
    { ...hotpotSummary(), scored: null },
    { ...hotpotSummary(), total: 2 },
    { ...summary(), scored: 1 },
    { ...summary(), automatic_scores: { exact_match: 1, f1: 1 } },
    { ...hotpotSummary(), evaluation: 'unknown_evaluator' },
  ])('rejects inconsistent automatic summary scores or evaluator fields: %j', async (value) => {
    const api = createAnswerApi(vi.fn<typeof fetch>().mockResolvedValue(response([value])));
    await expect(api.listRuns()).rejects.toBeInstanceOf(ApiError);
  });

  it.each([
    { ...hotpotCase(), reference_answer: undefined },
    { ...hotpotCase(), reference_answer: '' },
    { ...hotpotCase(), reference_answer: 1 },
    { ...hotpotCase(), automatic_scores: undefined },
    { ...hotpotCase(), automatic_scores: { exact_match: 0.5, f1: 1 } },
    { ...hotpotCase(), automatic_scores: { exact_match: 1, f1: -1 } },
    { ...hotpotCase(), automatic_scores: { exact_match: 0, f1: 0 } },
    { ...hotpotCase(), status: 'error', outcome: 'error' },
    { ...hotpotCase(), outcome: 'refused' },
  ])('rejects invalid case scores, references, or aggregate disagreement: %j', async (value) => {
    const api = createAnswerApi(
      vi.fn<typeof fetch>().mockResolvedValue(response({ ...hotpotDetail(), cases: [value] })),
    );
    await expect(api.getRun('run-a')).rejects.toBeInstanceOf(ApiError);
  });

  it('rejects automatic case fields in legacy manual runs', async () => {
    const api = createAnswerApi(
      vi.fn<typeof fetch>().mockResolvedValue(
        response(
          detail({
            cases: [
              answerCase({
                reference_answer: 'A reference',
                automatic_scores: { exact_match: 1, f1: 1 },
              }),
            ],
          }),
        ),
      ),
    );
    await expect(api.getRun('run-a')).rejects.toBeInstanceOf(ApiError);
  });

  it('separates automatic and manual evaluators while comparing fully scored refusals', () => {
    expect(answerComparabilityKey(hotpotSummary())).not.toBeNull();
    expect(answerComparabilityKey(hotpotSummary())).not.toBe(answerComparabilityKey(summary()));
    expect(answerComparabilityKey(hotpotSummary({ scored: undefined }))).toBeNull();
  });
});
