import { describe, expect, it } from 'vitest';
import {
  exportComparisonCsv,
  formatResult,
  getBestValue,
  parseBenchmarkReport,
  SAMPLE_REPORT,
  type BenchmarkReport,
} from './benchmark-data';

function fixture(): BenchmarkReport {
  return {
    schemaVersion: 1,
    name: 'Comparison',
    sample: false,
    architectures: [
      { id: 'a', name: 'Architecture A', description: 'Reference' },
      { id: 'b', name: 'Architecture B', description: 'Candidate' },
    ],
    benchmarks: [
      {
        id: 'quality',
        name: 'Quality',
        suite: 'Core',
        metric: 'Accuracy',
        unit: '%',
        direction: 'higher',
      },
      {
        id: 'errors',
        name: 'Errors',
        suite: 'Core',
        metric: 'Error rate',
        unit: '%',
        direction: 'lower',
      },
    ],
    results: [
      { benchmarkId: 'quality', architectureId: 'a', status: 'completed', value: 80 },
      { benchmarkId: 'quality', architectureId: 'b', status: 'completed', value: 90 },
      { benchmarkId: 'errors', architectureId: 'a', status: 'completed', value: 8 },
      { benchmarkId: 'errors', architectureId: 'b', status: 'running' },
    ],
  };
}

describe('parseBenchmarkReport', () => {
  it('imports arbitrary suites, metrics, architectures, and partial results', () => {
    const report = fixture();
    report.results.pop();
    report.sample = true;
    expect(parseBenchmarkReport(JSON.stringify(report))).toEqual({ ...report, sample: false });
  });

  it('keeps zero and negative completed scores and permits an empty queue', () => {
    const report = fixture();
    report.results[0].value = 0;
    report.results[1].value = -1.5;
    expect(parseBenchmarkReport(JSON.stringify(report)).results.slice(0, 2)).toEqual(
      report.results.slice(0, 2),
    );
    report.results = [];
    expect(parseBenchmarkReport(JSON.stringify(report)).results).toEqual([]);
  });

  it('imports all supported statuses and their notes', () => {
    const report = fixture();
    report.results = ['completed', 'running', 'queued', 'failed'].map((status, index) => ({
      benchmarkId: index < 2 ? 'quality' : 'errors',
      architectureId: index % 2 === 0 ? 'a' : 'b',
      status: status as BenchmarkReport['results'][number]['status'],
      ...(status === 'completed' ? { value: 12 } : {}),
      note: `Status: ${status}`,
    }));
    expect(parseBenchmarkReport(JSON.stringify(report))).toEqual(report);
  });

  it.each(['not json', 'null', '[]', '{"schemaVersion":2}'])(
    'rejects malformed report %s',
    (text) => {
      expect(() => parseBenchmarkReport(text)).toThrow();
    },
  );

  it.each(['metric', 'unit', 'direction'] as const)('requires each row to declare %s', (key) => {
    const report = fixture();
    const invalidRow: Record<string, unknown> = { ...report.benchmarks[0] };
    delete invalidRow[key];
    expect(() =>
      parseBenchmarkReport(JSON.stringify({ ...report, benchmarks: [invalidRow] })),
    ).toThrow(key);
  });

  it.each(['architectures', 'benchmarks'] as const)('rejects duplicate %s IDs', (key) => {
    const report = fixture();
    report[key][1].id = report[key][0].id;
    expect(() => parseBenchmarkReport(JSON.stringify(report))).toThrow('duplicates');
  });

  it('rejects duplicate result coordinates', () => {
    const report = fixture();
    report.results.push({ ...report.results[0] });
    expect(() => parseBenchmarkReport(JSON.stringify(report))).toThrow('duplicates');
  });

  it.each(['benchmarkId', 'architectureId'] as const)('rejects unknown %s references', (key) => {
    const report = fixture();
    report.results[0][key] = 'unknown';
    expect(() => parseBenchmarkReport(JSON.stringify(report))).toThrow('unknown');
  });

  it.each([undefined, null, '90'])('rejects invalid completed score %s', (value) => {
    const report = fixture();
    expect(() =>
      parseBenchmarkReport(
        JSON.stringify({
          ...report,
          results: [{ ...report.results[0], value }],
        }),
      ),
    ).toThrow('finite number');
  });

  it('rejects numeric overflow from JSON', () => {
    const text = JSON.stringify(fixture()).replace('"value":80', '"value":1e400');
    expect(() => parseBenchmarkReport(text)).toThrow('finite number');
  });

  it.each(['running', 'queued', 'failed'])('rejects a score for a %s cell', (status) => {
    const report = fixture();
    expect(() =>
      parseBenchmarkReport(
        JSON.stringify({
          ...report,
          results: [{ ...report.results[0], status }],
        }),
      ),
    ).toThrow('must not have a value');
  });

  it.each([
    ['architectures', 51],
    ['benchmarks', 2_001],
    ['results', 100_001],
  ] as const)('caps the number of %s', (key, length) => {
    const report = fixture();
    expect(() =>
      parseBenchmarkReport(
        JSON.stringify({
          ...report,
          [key]: Array.from({ length }, () => report[key][0]),
        }),
      ),
    ).toThrow('at most');
  });

  it('ships a valid, explicitly illustrative comparison with all cell states', () => {
    expect(SAMPLE_REPORT.sample).toBe(true);
    expect(SAMPLE_REPORT.benchmarks).toHaveLength(12);
    expect(SAMPLE_REPORT.architectures).toHaveLength(4);
    expect(parseBenchmarkReport(JSON.stringify(SAMPLE_REPORT)).results).toEqual(
      SAMPLE_REPORT.results,
    );
    expect(new Set(SAMPLE_REPORT.results.map(({ status }) => status))).toEqual(
      new Set(['completed', 'running', 'queued', 'failed']),
    );
  });
});

describe('score comparison', () => {
  it('respects row direction and ignores unfinished cells', () => {
    const report = fixture();
    expect(getBestValue(report, 'quality')).toBe(90);
    expect(getBestValue(report, 'errors')).toBe(8);
    report.results[3] = { ...report.results[3], status: 'completed', value: 2 };
    expect(getBestValue(report, 'errors')).toBe(2);
  });

  it('returns one shared best value for tied architectures', () => {
    const report = fixture();
    report.results[0].value = 90;
    expect(getBestValue(report, 'quality')).toBe(90);
  });

  it('returns no best score for missing or unmeasured rows', () => {
    const report = fixture();
    report.results = [];
    expect(getBestValue(report, 'quality')).toBeNull();
    expect(getBestValue(report, 'unknown')).toBeNull();
  });

  it('formats arbitrary score units without turning zeros into missing values', () => {
    expect(formatResult(0, '%')).toBe('0%');
    expect(formatResult(1234.567, 'points')).toBe('1,234.57 points');
    expect(formatResult(0.25, '')).toBe('0.25');
  });
});

describe('exportComparisonCsv', () => {
  it('exports scores, row units, directions, and explicit unfinished or missing cells', () => {
    const report = fixture();
    report.results.pop();
    expect(exportComparisonCsv(report)).toBe(
      [
        '"Benchmark","Suite","Metric","Unit","Direction","Architecture A","Architecture B"',
        '"Quality","Core","Accuracy","%","higher is better","80","90"',
        '"Errors","Core","Error rate","%","lower is better","8","not run"',
      ].join('\r\n'),
    );
    report.results.push({ benchmarkId: 'errors', architectureId: 'b', status: 'queued' });
    expect(exportComparisonCsv(report)).toContain('"8","queued"');
  });

  it('exports only requested rows and architectures in their selected order', () => {
    const report = fixture();
    expect(exportComparisonCsv(report, [report.benchmarks[1]], ['b'])).toBe(
      [
        '"Benchmark","Suite","Metric","Unit","Direction","Architecture B"',
        '"Errors","Core","Error rate","%","lower is better","running"',
      ].join('\r\n'),
    );
  });

  it('escapes quotes, delimiters, newlines, and spreadsheet formulas without changing numeric scores', () => {
    const report = fixture();
    report.architectures[0].name = '=HYPERLINK("https://example.com")';
    report.benchmarks[0].name = 'A, "quoted"\nbenchmark';
    report.benchmarks[0].suite = '  +SUM(1,2)';
    report.results[0].value = -1;
    const csv = exportComparisonCsv(report);
    expect(csv).toContain('"\'=HYPERLINK(""https://example.com"")"');
    expect(csv).toContain('"A, ""quoted""\nbenchmark"');
    expect(csv).toContain('"\'  +SUM(1,2)"');
    expect(csv).toContain('"-1","90"');
  });
});
