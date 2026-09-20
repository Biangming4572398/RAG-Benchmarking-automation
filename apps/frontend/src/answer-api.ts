import { ApiError } from './benchmark-api';

export interface AnswerEmbeddingModel {
  id: string;
  revision: string;
}

export interface AnswerProfile {
  id: string;
  label: string;
  enabled: boolean;
  disabled_reason: string | null;
}

export interface AnswerRuntime {
  available: boolean;
  reason: string | null;
  embedding_model: AnswerEmbeddingModel | null;
  profiles: AnswerProfile[];
}

export interface StartAnswerRunRequest {
  benchmark_id: string;
  architecture_label: string;
  profile_id: string;
}

export interface AnswerMeans {
  correctness: number;
  groundedness: number;
  hallucination_rate: number;
  citation_accuracy: number;
}

export interface AutomaticAnswerScores {
  exact_match: number;
  f1: number;
}

export interface AnswerRunSummary {
  id: string;
  request: StartAnswerRunRequest;
  benchmark_fingerprint: string;
  evaluation: 'manual_review_v1' | 'hotpotqa_answer_v1';
  embedding_model: AnswerEmbeddingModel;
  generation_model: { profile_id: string; label: string };
  top_k: 8;
  status: 'running' | 'completed' | 'failed' | 'interrupted';
  started_at_ms: number;
  finished_at_ms: number | null;
  total: number;
  completed: number;
  failed: number;
  answered: number;
  reviewed: number;
  means: AnswerMeans | null;
  scored?: number;
  automatic_scores?: AutomaticAnswerScores;
  error: string | null;
  scope: unknown;
  watermark: unknown;
  source_ids: string[];
}

export interface AnswerEvidence {
  id: string;
  ordinal: number;
  sourceId: string;
  sourceTitle: string;
  sourceRevision: string;
  location: string;
  excerpt: string;
}

export interface AnswerLineage {
  id: string;
  claim: string;
  evidenceIds: string[];
}

export interface AnswerModelReceipt {
  profileId: string;
  route: string;
  modelLabel: string;
}

export interface AnswerReviewRequest {
  reviewer: string;
  correctness: boolean;
  groundedness: boolean;
  hallucination: boolean;
  citation_accuracy: boolean;
  notes: string;
}

export interface AnswerReview extends AnswerReviewRequest {
  reviewed_at_ms: number;
}

export interface AnswerCase {
  case_id: string;
  query: string;
  status: 'ok' | 'error';
  outcome: 'answered' | 'refused' | 'evidence-only' | 'not-ready' | 'error';
  answer: string | null;
  reason: string | null;
  evidence: AnswerEvidence[];
  lineage: AnswerLineage[];
  model_receipt: AnswerModelReceipt | null;
  latency_ms: number;
  error: string | null;
  review: AnswerReview | null;
  reference_answer?: string;
  automatic_scores?: AutomaticAnswerScores;
}

export interface AnswerRun extends AnswerRunSummary {
  cases: AnswerCase[];
}

export interface AnswerApi {
  getRuntime(signal?: AbortSignal): Promise<AnswerRuntime>;
  listRuns(signal?: AbortSignal): Promise<AnswerRunSummary[]>;
  getRun(id: string, signal?: AbortSignal): Promise<AnswerRun>;
  startRun(body: StartAnswerRunRequest, signal?: AbortSignal): Promise<AnswerRunSummary>;
  saveReview(
    runId: string,
    caseId: string,
    body: AnswerReviewRequest,
    signal?: AbortSignal,
  ): Promise<AnswerRun>;
  downloadScores(id: string, signal?: AbortSignal): Promise<Blob>;
}

type ValueRecord = Record<string, unknown>;
type Check<T> = (value: unknown) => value is T;

const isRecord = (value: unknown): value is ValueRecord =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const isText = (value: unknown): value is string => typeof value === 'string';
const isNonemptyText = (value: unknown): value is string =>
  isText(value) && value.trim().length > 0;
const isNullableText = (value: unknown): value is string | null => value === null || isText(value);
const isCount = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const isList =
  <T>(check: Check<T>): Check<T[]> =>
  (value: unknown): value is T[] =>
    Array.isArray(value) && value.every(check);
const textFits = (value: unknown, bytes: number): value is string =>
  isText(value) && new TextEncoder().encode(value).length <= bytes;
const isLabel = (value: unknown, bytes: number): value is string =>
  isNonemptyText(value) && value.trim() === value && textFits(value, bytes);

function isEmbeddingModel(value: unknown): value is AnswerEmbeddingModel {
  return isRecord(value) && isNonemptyText(value.id) && isNonemptyText(value.revision);
}

function isProfile(value: unknown): value is AnswerProfile {
  return (
    isRecord(value) &&
    isNonemptyText(value.id) &&
    isNonemptyText(value.label) &&
    typeof value.enabled === 'boolean' &&
    isNullableText(value.disabled_reason)
  );
}

function isRuntime(value: unknown): value is AnswerRuntime {
  return (
    isRecord(value) &&
    typeof value.available === 'boolean' &&
    isNullableText(value.reason) &&
    (value.embedding_model === null || isEmbeddingModel(value.embedding_model)) &&
    isList(isProfile)(value.profiles) &&
    new Set(value.profiles.map((profile) => profile.id)).size === value.profiles.length &&
    (!value.available ||
      (value.embedding_model !== null && value.profiles.some((profile) => profile.enabled)))
  );
}

function isStartRequest(value: unknown): value is StartAnswerRunRequest {
  return (
    isRecord(value) &&
    isNonemptyText(value.benchmark_id) &&
    isLabel(value.architecture_label, 256) &&
    isNonemptyText(value.profile_id)
  );
}

function isMeans(value: unknown): value is AnswerMeans {
  return (
    isRecord(value) &&
    ['correctness', 'groundedness', 'hallucination_rate', 'citation_accuracy'].every(
      (key) =>
        typeof value[key] === 'number' &&
        Number.isFinite(value[key]) &&
        value[key] >= 0 &&
        value[key] <= 1,
    )
  );
}

function isAutomaticScores(value: unknown): value is AutomaticAnswerScores {
  return (
    isRecord(value) &&
    ['exact_match', 'f1'].every(
      (key) =>
        typeof value[key] === 'number' &&
        Number.isFinite(value[key]) &&
        value[key] >= 0 &&
        value[key] <= 1,
    )
  );
}

function hasEvaluationScores(value: ValueRecord): boolean {
  if (value.evaluation === 'manual_review_v1') {
    return (
      value.automatic_scores === undefined && (value.scored === undefined || value.scored === 0)
    );
  }
  if (
    value.evaluation !== 'hotpotqa_answer_v1' ||
    !isCount(value.completed) ||
    !isCount(value.failed) ||
    !isCount(value.total) ||
    !isAutomaticScores(value.automatic_scores)
  ) {
    return false;
  }
  const scored = value.scored === undefined ? 0 : value.scored;
  const ceiling = value.total === 0 ? 0 : (value.completed + value.failed) / value.total;
  return (
    isCount(scored) &&
    scored === value.completed + value.failed &&
    value.automatic_scores.exact_match <= ceiling + 1e-9 &&
    value.automatic_scores.f1 <= ceiling + 1e-9
  );
}

function isSummary(value: unknown): value is AnswerRunSummary {
  return (
    isRecord(value) &&
    isNonemptyText(value.id) &&
    isStartRequest(value.request) &&
    isText(value.benchmark_fingerprint) &&
    hasEvaluationScores(value) &&
    isEmbeddingModel(value.embedding_model) &&
    isRecord(value.generation_model) &&
    isNonemptyText(value.generation_model.profile_id) &&
    isNonemptyText(value.generation_model.label) &&
    value.top_k === 8 &&
    ['running', 'completed', 'failed', 'interrupted'].includes(String(value.status)) &&
    isCount(value.started_at_ms) &&
    (value.finished_at_ms === null || isCount(value.finished_at_ms)) &&
    isCount(value.total) &&
    isCount(value.completed) &&
    isCount(value.failed) &&
    value.completed + value.failed <= value.total &&
    isCount(value.answered) &&
    value.answered <= value.completed &&
    isCount(value.reviewed) &&
    value.reviewed <= value.answered &&
    (value.reviewed === 0 ? value.means === null : isMeans(value.means)) &&
    isNullableText(value.error) &&
    'scope' in value &&
    'watermark' in value &&
    isList(isText)(value.source_ids)
  );
}

function isEvidence(value: unknown): value is AnswerEvidence {
  return (
    isRecord(value) &&
    ['id', 'sourceId', 'sourceRevision', 'sourceTitle', 'location', 'excerpt'].every((key) =>
      isText(value[key]),
    ) &&
    isCount(value.ordinal)
  );
}

function isLineage(value: unknown): value is AnswerLineage {
  return (
    isRecord(value) && isText(value.id) && isText(value.claim) && isList(isText)(value.evidenceIds)
  );
}

function isReceipt(value: unknown): value is AnswerModelReceipt {
  return isRecord(value) && ['profileId', 'route', 'modelLabel'].every((key) => isText(value[key]));
}

function isReviewRequest(value: unknown): value is AnswerReviewRequest {
  return (
    isRecord(value) &&
    isLabel(value.reviewer, 128) &&
    ['correctness', 'groundedness', 'hallucination', 'citation_accuracy'].every(
      (key) => typeof value[key] === 'boolean',
    ) &&
    textFits(value.notes, 4000)
  );
}

function isReview(value: unknown): value is AnswerReview {
  return isReviewRequest(value) && 'reviewed_at_ms' in value && isCount(value.reviewed_at_ms);
}

function isCase(value: unknown): value is AnswerCase {
  if (
    !isRecord(value) ||
    !isNonemptyText(value.case_id) ||
    !isText(value.query) ||
    !['ok', 'error'].includes(String(value.status)) ||
    !['answered', 'refused', 'evidence-only', 'not-ready', 'error'].includes(
      String(value.outcome),
    ) ||
    !isNullableText(value.answer) ||
    !isNullableText(value.reason) ||
    !isList(isEvidence)(value.evidence) ||
    !isList(isLineage)(value.lineage) ||
    !(value.model_receipt === null || isReceipt(value.model_receipt)) ||
    !isCount(value.latency_ms) ||
    !isNullableText(value.error) ||
    !(value.review === null || isReview(value.review))
  ) {
    return false;
  }
  if (value.status === 'ok' && value.outcome === 'answered') {
    const evidenceIds = new Set(value.evidence.map((evidence) => evidence.id));
    return (
      isNonemptyText(value.answer) &&
      value.model_receipt !== null &&
      Object.values(value.model_receipt).every(isNonemptyText) &&
      evidenceIds.size === value.evidence.length &&
      value.evidence.every((item) =>
        [item.id, item.sourceId, item.sourceRevision].every(isNonemptyText),
      ) &&
      value.lineage.every(
        (item) => isNonemptyText(item.id) && item.evidenceIds.every((id) => evidenceIds.has(id)),
      )
    );
  }
  // Failed pinning checks deliberately preserve returned evidence and receipts
  // for diagnosis. They remain structurally typed and cannot carry a review.
  return value.review === null;
}

function isRun(value: unknown): value is AnswerRun {
  if (!isSummary(value) || !('cases' in value) || !isList(isCase)(value.cases)) return false;
  if (value.evaluation === 'hotpotqa_answer_v1') {
    if (
      value.cases.length !== (value.scored ?? 0) ||
      !value.cases.every(
        (item) =>
          isNonemptyText(item.reference_answer) &&
          isAutomaticScores(item.automatic_scores) &&
          (item.automatic_scores.exact_match === 0 || item.automatic_scores.exact_match === 1) &&
          ((item.status === 'ok' && item.outcome === 'answered') ||
            (item.automatic_scores.exact_match === 0 && item.automatic_scores.f1 === 0)),
      )
    ) {
      return false;
    }
    for (const metric of ['exact_match', 'f1'] as const) {
      const mean =
        value.total === 0
          ? 0
          : value.cases.reduce((sum, item) => sum + item.automatic_scores![metric], 0) /
            value.total;
      if (Math.abs(mean - value.automatic_scores![metric]) > 1e-9) return false;
    }
  } else if (
    value.cases.some(
      (item) => item.automatic_scores !== undefined || item.reference_answer !== undefined,
    )
  ) {
    return false;
  }
  return (
    value.cases.length === value.completed + value.failed &&
    new Set(value.cases.map((item) => item.case_id)).size === value.cases.length &&
    value.cases.filter((item) => item.status === 'ok').length === value.completed &&
    value.cases.filter((item) => item.status === 'ok' && item.outcome === 'answered').length ===
      value.answered &&
    value.cases.filter((item) => item.review !== null).length === value.reviewed
  );
}

async function responseError(response: Response): Promise<ApiError> {
  const body = await response.text();
  let message =
    response.status === 404
      ? 'This benchmark server does not support answer-quality runs. Restart it with the updated backend.'
      : `Benchmark server returned HTTP ${response.status}.`;
  try {
    const value: unknown = JSON.parse(body);
    if (isRecord(value) && isNonemptyText(value.error)) message = value.error;
  } catch {
    if (response.status !== 404 && body.trim() && !/^\s*</.test(body)) message = body;
  }
  return new ApiError(message.replace(/\s+/g, ' ').trim().slice(0, 300), response.status);
}

/** Backend/model configuration stays outside the renderer; this transport is same-origin only. */
export function createAnswerApi(fetchImpl: typeof fetch = globalThis.fetch): AnswerApi {
  const base = '/api/benchmarks/v1';
  async function request<T>(
    path: string,
    check: Check<T>,
    signal?: AbortSignal,
    body?: StartAnswerRunRequest | AnswerReviewRequest,
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
    } catch (cause) {
      if (signal?.aborted || (cause instanceof Error && cause.name === 'AbortError')) throw cause;
      throw new ApiError(
        'Invalid answer-quality response. Check the server connection.',
        response.status,
      );
    }
    if (!check(value)) {
      throw new ApiError(
        'Invalid answer-quality response. Check the server connection.',
        response.status,
      );
    }
    return value;
  }

  return {
    getRuntime: (signal) => request('/answer-runtime', isRuntime, signal),
    listRuns: (signal) => request('/answer-runs', isList(isSummary), signal),
    getRun: (id, signal) => request(`/answer-runs/${encodeURIComponent(id)}`, isRun, signal),
    async startRun(body, signal) {
      if (!isStartRequest(body)) {
        throw new ApiError(
          'Choose a benchmark and model profile and provide a trimmed architecture label of at most 256 UTF-8 bytes.',
          400,
        );
      }
      return request('/answer-runs', isSummary, signal, body);
    },
    async saveReview(runId, caseId, body, signal) {
      if (!isReviewRequest(body)) {
        throw new ApiError(
          'Provide a trimmed reviewer name of at most 128 UTF-8 bytes, four review decisions, and notes of at most 4000 UTF-8 bytes.',
          400,
        );
      }
      return request(
        `/answer-runs/${encodeURIComponent(runId)}/cases/${encodeURIComponent(caseId)}/review`,
        isRun,
        signal,
        body,
      );
    },
    async downloadScores(id, signal) {
      const response = await fetchImpl(`${base}/answer-runs/${encodeURIComponent(id)}/scores.csv`, {
        method: 'GET',
        headers: { Accept: 'text/csv' },
        cache: 'no-store',
        signal,
      });
      if (!response.ok) throw await responseError(response);
      if (response.headers.get('content-type')?.split(';')[0].trim().toLowerCase() !== 'text/csv') {
        throw new ApiError('Invalid CSV response. Check the server connection.', response.status);
      }
      return response.blob();
    },
  };
}

/** Compare complete runs under the same evaluator; model/architecture choices may differ. */
export function answerComparabilityKey(run: AnswerRunSummary): string | null {
  if (
    !isSummary(run) ||
    run.status !== 'completed' ||
    run.total < 1 ||
    run.completed !== run.total ||
    run.failed !== 0 ||
    (run.evaluation === 'manual_review_v1'
      ? run.answered !== run.total || run.reviewed !== run.answered || run.means === null
      : run.scored !== run.total || run.automatic_scores === undefined) ||
    !run.benchmark_fingerprint.trim() ||
    run.source_ids.length === 0 ||
    run.source_ids.some((id) => !id.trim())
  ) {
    return null;
  }
  return JSON.stringify([
    run.benchmark_fingerprint,
    run.top_k,
    run.evaluation,
    [...new Set(run.source_ids)].sort(),
  ]);
}
