import { useCallback } from 'react';
import type { BenchmarkApi, BenchmarkInfo, BenchmarkRun } from './benchmark-api';
import { usePolling } from './use-polling';

const EMPTY: { benchmarks: BenchmarkInfo[]; runs: BenchmarkRun[] } = { benchmarks: [], runs: [] };

export function useBenchmarkWorkspace(api: BenchmarkApi, pollInterval: number) {
  const load = useCallback(
    async (signal: AbortSignal) => {
      const [benchmarks, runs] = await Promise.all([
        api.listBenchmarks(signal),
        api.listRuns(signal),
      ]);
      return { benchmarks, runs };
    },
    [api],
  );
  const { data, ...state } = usePolling(load, EMPTY, pollInterval);
  return { ...data, ...state };
}
