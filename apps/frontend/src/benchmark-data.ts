export interface BenchmarkDefinition {
  id: string;
  name: string;
  suite: string;
  metric: string;
  unit: string;
  direction: 'higher' | 'lower';
}

export interface Architecture {
  id: string;
  name: string;
  description: string;
}

export interface BenchmarkResult {
  benchmarkId: string;
  architectureId: string;
  status: 'completed' | 'running' | 'queued' | 'failed';
  value?: number;
  note?: string;
}

export interface BenchmarkReport {
  schemaVersion: 1;
  name: string;
  sample: boolean;
  architectures: Architecture[];
  benchmarks: BenchmarkDefinition[];
  results: BenchmarkResult[];
}

const SAMPLE_BENCHMARKS: BenchmarkDefinition[] = [
  ['exact-match', 'Exact match', 'Correctness', 'Accuracy'],
  ['structured-output', 'Structured output', 'Correctness', 'Valid responses'],
  ['instruction-following', 'Instruction following', 'Correctness', 'Pass rate'],
  ['document-recall', 'Document recall', 'Retrieval', 'Recall@10'],
  ['source-attribution', 'Source attribution', 'Retrieval', 'Citation accuracy'],
  ['context-relevance', 'Context relevance', 'Retrieval', 'Relevant results'],
  ['multi-step', 'Multi-step reasoning', 'Reasoning', 'Pass rate'],
  ['numerical-reasoning', 'Numerical reasoning', 'Reasoning', 'Accuracy'],
  ['constraint-solving', 'Constraint solving', 'Reasoning', 'Pass rate'],
  ['tool-selection', 'Tool selection', 'Tool use', 'Accuracy'],
  ['argument-validity', 'Argument validity', 'Tool use', 'Valid calls'],
  ['task-completion', 'Task completion', 'Tool use', 'Success rate'],
].map(([id, name, suite, metric]) => ({
  id,
  name,
  suite,
  metric,
  unit: '%',
  direction: 'higher',
}));

const SAMPLE_ARCHITECTURES: Architecture[] = [
  { id: 'baseline', name: 'Baseline', description: 'Single-stage reference architecture' },
  { id: 'pipeline', name: 'Pipeline', description: 'Sequential stages with shared context' },
  {
    id: 'parallel',
    name: 'Parallel workers',
    description: 'Concurrent workers with an aggregation stage',
  },
  {
    id: 'event-driven',
    name: 'Event-driven',
    description: 'Independent stages coordinated through events',
  },
];

// Illustrative scores only: these are not measurements of Genesis or a real run.
const SAMPLE_SCORES = [
  [78.4, 85.1, 87.6, 86.2],
  [88.2, 97.4, 96.8, 98.1],
  [81.6, 89.8, 90.3, 88.7],
  [65.9, 82.7, 91.2, 87.5],
  [72.3, 89.6, 88.4, 92.1],
  [76.1, 85.2, 89.9, 88.3],
  [59.8, 76.4, 84.7, 79.2],
  [68.5, 82.1, 88.6, 83.7],
  [61.2, 79.3, 83.9, 81.8],
  [77.4, 91.8, 89.6, 93.2],
  [84.6, 96.3, 95.7, 97.1],
  [63.8, 81.5, 86.4, 85.9],
];

export const SAMPLE_REPORT: BenchmarkReport = {
  schemaVersion: 1,
  name: 'Architecture comparison',
  sample: true,
  architectures: SAMPLE_ARCHITECTURES,
  benchmarks: SAMPLE_BENCHMARKS,
  results: SAMPLE_BENCHMARKS.flatMap((benchmark, benchmarkIndex) =>
    SAMPLE_ARCHITECTURES.flatMap((architecture, architectureIndex): BenchmarkResult[] => {
      const coordinate = `${benchmark.id}/${architecture.id}`;
      if (coordinate === 'task-completion/event-driven') return [];
      if (coordinate === 'constraint-solving/parallel') {
        return [{ benchmarkId: benchmark.id, architectureId: architecture.id, status: 'running' }];
      }
      if (coordinate === 'task-completion/parallel') {
        return [{ benchmarkId: benchmark.id, architectureId: architecture.id, status: 'queued' }];
      }
      if (coordinate === 'argument-validity/event-driven') {
        return [
          {
            benchmarkId: benchmark.id,
            architectureId: architecture.id,
            status: 'failed',
            note: 'Illustrative failed run: invalid response format.',
          },
        ];
      }
      return [
        {
          benchmarkId: benchmark.id,
          architectureId: architecture.id,
          status: 'completed',
          value: SAMPLE_SCORES[benchmarkIndex][architectureIndex],
        },
      ];
    }),
  ),
};

function object(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object.`);
  }
  return value as Record<string, unknown>;
}

function string(value: unknown, label: string, allowEmpty = false): string {
  if (typeof value !== 'string' || (!allowEmpty && !value.trim())) {
    throw new Error(`${label} must be ${allowEmpty ? 'a string' : 'a non-empty string'}.`);
  }
  return value.trim();
}

function array(value: unknown, label: string, maximum: number, allowEmpty = false): unknown[] {
  if (!Array.isArray(value) || (!allowEmpty && value.length === 0)) {
    throw new Error(`${label} must be ${allowEmpty ? 'an array' : 'a non-empty array'}.`);
  }
  if (value.length > maximum) {
    throw new Error(`${label} must contain at most ${maximum.toLocaleString('en-GB')} entries.`);
  }
  return value;
}

function uniqueId(value: unknown, ids: Set<string>, label: string): string {
  const id = string(value, `${label}.id`);
  if (ids.has(id)) throw new Error(`${label}.id duplicates "${id}".`);
  ids.add(id);
  return id;
}

/** Read an architecture matrix. Each row declares how its scores are compared. */
export function parseBenchmarkReport(text: string): BenchmarkReport {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error('This file is not valid JSON. Choose a benchmark comparison report.');
  }
  const report = object(parsed, 'Report');
  if (report.schemaVersion !== 1) {
    throw new Error('Expected a benchmark comparison report with schemaVersion: 1.');
  }
  const name = string(report.name, 'Report.name');
  const architectureIds = new Set<string>();
  const benchmarkIds = new Set<string>();
  const architectures = array(report.architectures, 'architectures', 50).map((value, index) => {
    const label = `architectures[${index}]`;
    const item = object(value, label);
    return {
      id: uniqueId(item.id, architectureIds, label),
      name: string(item.name, `${label}.name`),
      description: string(item.description, `${label}.description`, true),
    };
  });
  const benchmarks = array(report.benchmarks, 'benchmarks', 2_000).map(
    (value, index): BenchmarkDefinition => {
      const label = `benchmarks[${index}]`;
      const item = object(value, label);
      if (item.direction !== 'higher' && item.direction !== 'lower') {
        throw new Error(`${label}.direction must be "higher" or "lower".`);
      }
      return {
        id: uniqueId(item.id, benchmarkIds, label),
        name: string(item.name, `${label}.name`),
        suite: string(item.suite, `${label}.suite`),
        metric: string(item.metric, `${label}.metric`),
        unit: string(item.unit, `${label}.unit`),
        direction: item.direction,
      };
    },
  );
  const seenCoordinates = new Map<string, Set<string>>();
  const results = array(report.results, 'results', 100_000, true).map(
    (value, index): BenchmarkResult => {
      const label = `results[${index}]`;
      const item = object(value, label);
      const benchmarkId = string(item.benchmarkId, `${label}.benchmarkId`);
      const architectureId = string(item.architectureId, `${label}.architectureId`);
      if (!benchmarkIds.has(benchmarkId)) {
        throw new Error(`${label} refers to unknown benchmark "${benchmarkId}".`);
      }
      if (!architectureIds.has(architectureId)) {
        throw new Error(`${label} refers to unknown architecture "${architectureId}".`);
      }
      const architecturesForBenchmark = seenCoordinates.get(benchmarkId) ?? new Set<string>();
      if (architecturesForBenchmark.has(architectureId)) {
        throw new Error(`${label} duplicates a benchmark and architecture pair.`);
      }
      architecturesForBenchmark.add(architectureId);
      seenCoordinates.set(benchmarkId, architecturesForBenchmark);
      const status = item.status;
      if (
        status !== 'completed' &&
        status !== 'running' &&
        status !== 'queued' &&
        status !== 'failed'
      ) {
        throw new Error(`${label}.status must be "completed", "running", "queued", or "failed".`);
      }
      const result: BenchmarkResult = { benchmarkId, architectureId, status };
      if (status === 'completed') {
        if (typeof item.value !== 'number' || !Number.isFinite(item.value)) {
          throw new Error(`${label}.value must be a finite number for a completed result.`);
        }
        result.value = item.value;
      } else if ('value' in item) {
        throw new Error(`${label} must not have a value until its status is "completed".`);
      }
      if ('note' in item) result.note = string(item.note, `${label}.note`, true);
      return result;
    },
  );

  return { schemaVersion: 1, name, sample: false, architectures, benchmarks, results };
}

export function formatResult(value: number, unit: string): string {
  const score = value.toLocaleString('en-GB', { maximumFractionDigits: 2 });
  return unit === '%' ? `${score}%` : unit ? `${score} ${unit}` : score;
}

/** Missing and unfinished cells never participate in a row's best score. */
export function getBestValue(report: BenchmarkReport, benchmarkId: string): number | null {
  const benchmark = report.benchmarks.find((item) => item.id === benchmarkId);
  if (!benchmark) return null;
  let best: number | null = null;
  for (const result of report.results) {
    if (
      result.benchmarkId !== benchmarkId ||
      result.status !== 'completed' ||
      result.value === undefined ||
      !Number.isFinite(result.value)
    ) {
      continue;
    }
    if (
      best === null ||
      (benchmark.direction === 'higher' ? result.value > best : result.value < best)
    ) {
      best = result.value;
    }
  }
  return best;
}

function csvCell(value: string | number): string {
  // Treat imported labels as text when opened in spreadsheet applications.
  const safeValue =
    typeof value === 'string' && (/^\s*[=+\-@]/u.test(value) || /^[\t\r\n]/u.test(value))
      ? `'${value}`
      : String(value);
  return `"${safeValue.replace(/"/g, '""')}"`;
}

export function exportComparisonCsv(
  report: BenchmarkReport,
  benchmarks: BenchmarkDefinition[] = report.benchmarks,
  architectureIds: string[] = report.architectures.map((architecture) => architecture.id),
): string {
  const architectures = architectureIds.flatMap((id) => {
    const architecture = report.architectures.find((item) => item.id === id);
    return architecture ? [architecture] : [];
  });
  const results = new Map<string, Map<string, BenchmarkResult>>();
  for (const result of report.results) {
    const row = results.get(result.benchmarkId) ?? new Map<string, BenchmarkResult>();
    row.set(result.architectureId, result);
    results.set(result.benchmarkId, row);
  }
  const rows: (string | number)[][] = [
    ['Benchmark', 'Suite', 'Metric', 'Unit', 'Direction', ...architectures.map(({ name }) => name)],
    ...benchmarks.map((benchmark) => [
      benchmark.name,
      benchmark.suite,
      benchmark.metric,
      benchmark.unit,
      `${benchmark.direction} is better`,
      ...architectures.map((architecture) => {
        const result = results.get(benchmark.id)?.get(architecture.id);
        return result?.status === 'completed' && result.value !== undefined
          ? result.value
          : (result?.status ?? 'not run');
      }),
    ]),
  ];
  return rows.map((row) => row.map(csvCell).join(',')).join('\r\n');
}
