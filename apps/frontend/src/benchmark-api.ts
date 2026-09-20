/** The server contract in apps/backend/src/{server,catalog,trials,storage}.rs. */
export interface BenchmarkDefinition {
  name: string;
  source: string;
  split: string;
  adapter: string;
  evaluation: string;
  defaults: { limit: number; top_k: number };
}

export interface Catalog {
  benchmarks: Record<string, BenchmarkDefinition>;
}

export interface BenchmarkConfiguration {
  key: string;
  definition: BenchmarkDefinition;
}

export interface BenchmarkInfo {
  id: string;
  source: string;
  split: string;
  metric_kind: string;
  case_count: number;
  document_count: number;
  corpus_path: string;
  fingerprint: string;
  configuration?: BenchmarkConfiguration;
}

export interface BenchmarkSnapshot {
  id: string;
  source: string;
  split: string;
  metric_kind: string;
  configuration?: BenchmarkConfiguration;
  cases: Array<{
    id: string;
    query: string;
    document_id: string;
    answer_reference?: {
      answer: string;
      candidate_document_ids: string[];
      supporting_facts: Array<{ title: string; sentence_index: number }>;
    };
    reference_outputs: Array<{
      id: string;
      output: string;
      model: string;
      quality: string;
      hallucination_labels: unknown;
    }>;
  }>;
  documents: Array<{ id: string; filename: string; text: string; revision: string }>;
}

export interface Scores {
  context_hit_at_k: number;
  reciprocal_rank_at_k: number;
  ndcg_at_k: number;
}

export interface BenchmarkRun {
  id: string;
  request: { benchmark_id: string; top_k: number; label: string };
  metric_kind: string;
  benchmark_fingerprint: string;
  status: 'running' | 'completed' | 'failed' | 'interrupted';
  started_at_ms: number;
  finished_at_ms: number | null;
  total: number;
  completed: number;
  failed: number;
  means: Scores | null;
  error: string | null;
  scope: unknown;
  watermark: unknown;
  source_ids: string[];
}

export interface LoadBenchmarkRequest {
  benchmark: string;
  limit?: number;
}

export interface StartRunRequest {
  benchmark_id: string;
  top_k?: number;
  label: string;
}

export interface BenchmarkApi {
  getCatalog(signal?: AbortSignal): Promise<Catalog>;
  listBenchmarks(signal?: AbortSignal): Promise<BenchmarkInfo[]>;
  getBenchmark(id: string, signal?: AbortSignal): Promise<BenchmarkSnapshot>;
  listRuns(signal?: AbortSignal): Promise<BenchmarkRun[]>;
  getRun(id: string, signal?: AbortSignal): Promise<BenchmarkRun>;
  loadBenchmark(request: LoadBenchmarkRequest, signal?: AbortSignal): Promise<BenchmarkInfo>;
  startRun(request: StartRunRequest, signal?: AbortSignal): Promise<BenchmarkRun>;
  downloadScores(id: string, signal?: AbortSignal): Promise<Blob>;
}

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

type RecordValue = Record<string, unknown>;
type Check<T> = (value: unknown) => value is T;

const isRecord = (value: unknown): value is RecordValue =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const isText = (value: unknown): value is string => typeof value === 'string';
const isCount = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const hasStrings = (value: RecordValue, keys: string[]): boolean =>
  keys.every((key) => isText(value[key]));
const isList =
  <T>(check: Check<T>): Check<T[]> =>
  (value: unknown): value is T[] =>
    Array.isArray(value) && value.every(check);

function isDefinition(value: unknown): value is BenchmarkDefinition {
  return (
    isRecord(value) &&
    hasStrings(value, ['name', 'source', 'split', 'adapter', 'evaluation']) &&
    isRecord(value.defaults) &&
    isCount(value.defaults.limit) &&
    value.defaults.limit >= 1 &&
    value.defaults.limit <= 10_000 &&
    isCount(value.defaults.top_k) &&
    value.defaults.top_k >= 1 &&
    value.defaults.top_k <= 100
  );
}

function hasConfiguration(value: RecordValue): boolean {
  return (
    value.configuration === undefined ||
    (isRecord(value.configuration) &&
      isText(value.configuration.key) &&
      isDefinition(value.configuration.definition))
  );
}

function isCatalog(value: unknown): value is Catalog {
  return (
    isRecord(value) &&
    isRecord(value.benchmarks) &&
    Object.values(value.benchmarks).every(isDefinition)
  );
}

function isBenchmarkInfo(value: unknown): value is BenchmarkInfo {
  return (
    isRecord(value) &&
    hasStrings(value, ['id', 'source', 'split', 'metric_kind', 'corpus_path', 'fingerprint']) &&
    isCount(value.case_count) &&
    isCount(value.document_count) &&
    hasConfiguration(value)
  );
}

function isSnapshot(value: unknown): value is BenchmarkSnapshot {
  return (
    isRecord(value) &&
    hasStrings(value, ['id', 'source', 'split', 'metric_kind']) &&
    hasConfiguration(value) &&
    Array.isArray(value.cases) &&
    value.cases.every(
      (item) =>
        isRecord(item) &&
        hasStrings(item, ['id', 'query', 'document_id']) &&
        (item.answer_reference === undefined ||
          (isRecord(item.answer_reference) &&
            isText(item.answer_reference.answer) &&
            isList(isText)(item.answer_reference.candidate_document_ids) &&
            Array.isArray(item.answer_reference.supporting_facts) &&
            item.answer_reference.supporting_facts.every(
              (fact) => isRecord(fact) && isText(fact.title) && isCount(fact.sentence_index),
            ))) &&
        Array.isArray(item.reference_outputs) &&
        item.reference_outputs.every(
          (output) =>
            isRecord(output) &&
            hasStrings(output, ['id', 'output', 'model', 'quality']) &&
            Array.isArray(output.hallucination_labels),
        ),
    ) &&
    Array.isArray(value.documents) &&
    value.documents.every(
      (item) => isRecord(item) && hasStrings(item, ['id', 'filename', 'text', 'revision']),
    )
  );
}

function isScores(value: unknown): value is Scores {
  return (
    isRecord(value) &&
    ['context_hit_at_k', 'reciprocal_rank_at_k', 'ndcg_at_k'].every(
      (key) => typeof value[key] === 'number' && Number.isFinite(value[key]),
    )
  );
}

function isRun(value: unknown): value is BenchmarkRun {
  return (
    isRecord(value) &&
    hasStrings(value, ['id', 'metric_kind', 'benchmark_fingerprint']) &&
    isRecord(value.request) &&
    hasStrings(value.request, ['benchmark_id', 'label']) &&
    isCount(value.request.top_k) &&
    value.request.top_k >= 1 &&
    value.request.top_k <= 100 &&
    ['running', 'completed', 'failed', 'interrupted'].includes(String(value.status)) &&
    isCount(value.started_at_ms) &&
    (value.finished_at_ms === null || isCount(value.finished_at_ms)) &&
    isCount(value.total) &&
    isCount(value.completed) &&
    isCount(value.failed) &&
    value.completed + value.failed <= value.total &&
    (value.means === null || isScores(value.means)) &&
    (value.error === null || isText(value.error)) &&
    'scope' in value &&
    'watermark' in value &&
    isList(isText)(value.source_ids)
  );
}

async function responseError(response: Response): Promise<ApiError> {
  const body = await response.text();
  let message = `Benchmark server returned HTTP ${response.status}.`;
  try {
    const parsed: unknown = JSON.parse(body);
    if (isRecord(parsed) && isText(parsed.error) && parsed.error.trim()) message = parsed.error;
  } catch {
    // Axum uses plain text for malformed requests, while a wrong proxy may return HTML.
    if (body.trim() && !/^\s*</.test(body)) message = body;
  }
  return new ApiError(message.replace(/\s+/g, ' ').trim().slice(0, 300), response.status);
}

/** Authentication is supplied by the module's server-side bridge, never by the renderer. */
export function createBenchmarkApi(fetchImpl: typeof fetch = globalThis.fetch): BenchmarkApi {
  const base = '/api/benchmarks/v1';
  async function request<T>(
    path: string,
    check: Check<T>,
    signal?: AbortSignal,
    body?: LoadBenchmarkRequest | StartRunRequest,
  ): Promise<T> {
    const response = await fetchImpl(`${base}${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: {
        Accept: 'application/json',
        ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      },
      cache: 'no-store',
      signal,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    if (!response.ok) throw await responseError(response);
    let value: unknown;
    try {
      value = await response.json();
    } catch {
      throw new ApiError(
        'Invalid benchmark response. Check the server connection.',
        response.status,
      );
    }
    if (!check(value)) {
      throw new ApiError(
        'Invalid benchmark response. Check the server connection.',
        response.status,
      );
    }
    return value;
  }

  return {
    getCatalog: (signal) => request('/catalog', isCatalog, signal),
    listBenchmarks: (signal) => request('/benchmarks', isList(isBenchmarkInfo), signal),
    getBenchmark: (id, signal) =>
      request(`/benchmarks/${encodeURIComponent(id)}`, isSnapshot, signal),
    listRuns: (signal) => request('/runs', isList(isRun), signal),
    getRun: (id, signal) => request(`/runs/${encodeURIComponent(id)}`, isRun, signal),
    loadBenchmark: (body, signal) => request('/benchmarks', isBenchmarkInfo, signal, body),
    startRun: (body, signal) => request('/runs', isRun, signal, body),
    async downloadScores(id, signal) {
      const response = await fetchImpl(`${base}/runs/${encodeURIComponent(id)}/scores.csv`, {
        headers: { Accept: 'text/csv' },
        cache: 'no-store',
        signal,
      });
      if (!response.ok) throw await responseError(response);
      if (!response.headers.get('content-type')?.toLowerCase().startsWith('text/csv')) {
        throw new ApiError('Invalid CSV response. Check the server connection.', response.status);
      }
      return response.blob();
    },
  };
}

/** Partial means are useful for diagnosis but are never eligible for full comparisons. */
export function comparabilityKey(run: BenchmarkRun): string | null {
  if (
    run.status !== 'completed' ||
    run.total < 1 ||
    run.completed !== run.total ||
    run.failed !== 0 ||
    run.means === null ||
    !run.metric_kind.trim() ||
    !run.benchmark_fingerprint.trim() ||
    !Number.isInteger(run.request.top_k) ||
    run.request.top_k < 1 ||
    run.request.top_k > 100 ||
    run.source_ids.length === 0 ||
    run.source_ids.some((id) => !id.trim())
  ) {
    return null;
  }
  return JSON.stringify([
    run.metric_kind,
    run.benchmark_fingerprint,
    run.request.top_k,
    [...new Set(run.source_ids)].sort(),
  ]);
}
