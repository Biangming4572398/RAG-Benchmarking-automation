import { ApiError } from './benchmark-api';

export interface ResultTable {
  benchmark: string;
  columns: string[];
  rows: Record<string, string>[];
}

export interface Results {
  benchmarks: ResultTable[];
}

export interface SuiteRequest {
  architecture_label: string;
  description?: string;
  profile_id?: string | null;
  top_k?: number | null;
}

export interface SuiteEntry {
  benchmark: string;
  benchmark_id: string | null;
  mode: 'retrieval' | 'generation' | null;
  status: 'queued' | 'running' | 'completed' | 'failed' | 'interrupted' | 'skipped';
  run_id: string | null;
  reason: string | null;
}

export interface SuitePreparation {
  benchmark: string;
  benchmark_id: string | null;
  configuration?: unknown;
  status: SuiteEntry['status'];
  reason: string | null;
}

export interface Initialization {
  status: 'initializing' | 'ready' | 'failed';
  phase: 'preparing' | 'indexing' | 'ready' | 'failed';
  preparations: SuitePreparation[];
  error: string | null;
}

export interface SuiteRun {
  id: string;
  run_number: number;
  request: SuiteRequest;
  status: 'running' | 'completed' | 'failed' | 'interrupted';
  started_at_ms: number;
  finished_at_ms: number | null;
  error: string | null;
  items: SuiteEntry[];
  phase?: 'preparing' | 'indexing' | 'running' | 'finished';
  preparations?: SuitePreparation[];
}

export interface ResultsApi {
  getInitialization(signal?: AbortSignal): Promise<Initialization>;
  getResults(signal?: AbortSignal): Promise<Results>;
  listSuites(signal?: AbortSignal): Promise<SuiteRun[]>;
  startSuite(request: SuiteRequest, signal?: AbortSignal): Promise<SuiteRun>;
  downloadResults(benchmark: string, signal?: AbortSignal): Promise<Blob>;
}

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === 'string';
const nullableText = (value: unknown) => value === null || text(value);
const count = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const entryStatus = (value: unknown) =>
  ['queued', 'running', 'completed', 'failed', 'interrupted', 'skipped'].includes(String(value));
function preparation(value: unknown): value is SuitePreparation {
  return (
    record(value) &&
    text(value.benchmark) &&
    nullableText(value.benchmark_id) &&
    entryStatus(value.status) &&
    nullableText(value.reason)
  );
}
function initialization(value: unknown): value is Initialization {
  return (
    record(value) &&
    ((value.status === 'initializing' && ['preparing', 'indexing'].includes(String(value.phase))) ||
      (value.status === 'ready' && value.phase === 'ready') ||
      (value.status === 'failed' && value.phase === 'failed')) &&
    Array.isArray(value.preparations) &&
    value.preparations.every(preparation) &&
    nullableText(value.error)
  );
}

function results(value: unknown): value is Results {
  return (
    record(value) &&
    Array.isArray(value.benchmarks) &&
    value.benchmarks.every(
      (table) =>
        record(table) &&
        text(table.benchmark) &&
        Array.isArray(table.columns) &&
        table.columns.every(text) &&
        Array.isArray(table.rows) &&
        table.rows.every(
          (row) =>
            record(row) &&
            Object.values(row).every(text) &&
            text(row.run_id) &&
            row.run_id.length > 0 &&
            text(row.run_number) &&
            /^[1-9]\d*$/.test(row.run_number) &&
            Number.isSafeInteger(Number(row.run_number)) &&
            ['retrieval', 'generation'].includes(String(row.mode)) &&
            ['running', 'completed', 'failed', 'interrupted'].includes(String(row.status)),
        ),
    )
  );
}

function suite(value: unknown): value is SuiteRun {
  return (
    record(value) &&
    text(value.id) &&
    count(value.run_number) &&
    value.run_number > 0 &&
    record(value.request) &&
    text(value.request.architecture_label) &&
    (value.request.description === undefined || text(value.request.description)) &&
    (value.request.profile_id === undefined || nullableText(value.request.profile_id)) &&
    ['running', 'completed', 'failed', 'interrupted'].includes(String(value.status)) &&
    count(value.started_at_ms) &&
    (value.finished_at_ms === null || count(value.finished_at_ms)) &&
    nullableText(value.error) &&
    (value.phase === undefined ||
      ['preparing', 'indexing', 'running', 'finished'].includes(String(value.phase))) &&
    (value.preparations === undefined ||
      (Array.isArray(value.preparations) && value.preparations.every(preparation))) &&
    Array.isArray(value.items) &&
    value.items.every(
      (entry) =>
        record(entry) &&
        text(entry.benchmark) &&
        nullableText(entry.benchmark_id) &&
        (entry.mode === null || ['retrieval', 'generation'].includes(String(entry.mode))) &&
        entryStatus(entry.status) &&
        nullableText(entry.run_id) &&
        nullableText(entry.reason),
    )
  );
}

async function error(response: Response): Promise<ApiError> {
  const body = await response.text();
  let message = `Benchmark server returned HTTP ${response.status}.`;
  try {
    const value: unknown = JSON.parse(body);
    if (record(value) && text(value.error)) message = value.error;
  } catch {
    if (body.trim() && !/^\s*</.test(body)) message = body;
  }
  return new ApiError(message.replace(/\s+/g, ' ').trim().slice(0, 500), response.status);
}

export function createResultsApi(fetchImpl: typeof fetch = globalThis.fetch): ResultsApi {
  const base = '/api/benchmarks/v1';
  async function request<T>(
    path: string,
    check: (value: unknown) => value is T,
    signal?: AbortSignal,
    body?: SuiteRequest,
  ): Promise<T> {
    const response = await fetchImpl(`${base}${path}`, {
      method: body ? 'POST' : 'GET',
      headers: {
        Accept: 'application/json',
        ...(body ? { 'Content-Type': 'application/json' } : {}),
      },
      cache: 'no-store',
      signal,
      ...(body ? { body: JSON.stringify(body) } : {}),
    });
    if (!response.ok) throw await error(response);
    let value: unknown;
    try {
      value = await response.json();
    } catch {
      /* Validated below. */
    }
    if (!check(value))
      throw new ApiError('Invalid benchmark response. Check the backend version.', response.status);
    return value;
  }
  return {
    async getInitialization(signal) {
      try {
        return await request('/initialization', initialization, signal);
      } catch (cause) {
        if (cause instanceof ApiError && cause.status === 404) {
          throw new ApiError(
            'This server does not support benchmark initialization. Update and restart the benchmark backend.',
            404,
          );
        }
        throw cause;
      }
    },
    getResults: (signal) => request('/results', results, signal),
    listSuites: (signal) =>
      request(
        '/suite-runs',
        (value): value is SuiteRun[] => Array.isArray(value) && value.every(suite),
        signal,
      ),
    startSuite: (body, signal) => request('/suite-runs', suite, signal, body),
    async downloadResults(benchmark, signal) {
      const response = await fetchImpl(
        `${base}/results/${encodeURIComponent(benchmark)}/scores.csv`,
        {
          headers: { Accept: 'text/csv' },
          cache: 'no-store',
          signal,
        },
      );
      if (!response.ok) throw await error(response);
      if (!response.headers.get('content-type')?.toLowerCase().startsWith('text/csv')) {
        throw new ApiError('Invalid CSV response. Check the server connection.', response.status);
      }
      return response.blob();
    },
  };
}
