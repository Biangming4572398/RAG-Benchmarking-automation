import { useCallback, useEffect, useRef, useState } from 'react';
import type { BenchmarkApi, BenchmarkInfo, BenchmarkRun, Catalog } from './benchmark-api';

export function useBenchmarkWorkspace(api: BenchmarkApi, pollInterval: number) {
  const [catalog, setCatalog] = useState<Catalog>({ benchmarks: {} });
  const [benchmarks, setBenchmarks] = useState<BenchmarkInfo[]>([]);
  const [runs, setRuns] = useState<BenchmarkRun[]>([]);
  const [error, setError] = useState('');
  const [connected, setConnected] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<number>();
  const inFlight = useRef<Promise<void> | null>(null);
  const controller = useRef<AbortController | null>(null);
  const mounted = useRef(false);

  const refresh = useCallback((): Promise<void> => {
    if (inFlight.current) return inFlight.current;
    const request = new AbortController();
    controller.current = request;
    setRefreshing(true);
    const work = (async () => {
      try {
        const [nextCatalog, nextBenchmarks, nextRuns] = await Promise.all([
          api.getCatalog(request.signal),
          api.listBenchmarks(request.signal),
          api.listRuns(request.signal),
        ]);
        if (request.signal.aborted) return;
        setCatalog(nextCatalog);
        setBenchmarks(nextBenchmarks);
        setRuns(nextRuns);
        setError('');
        setConnected(true);
        setUpdatedAt(Date.now());
      } catch (cause) {
        if (request.signal.aborted) return;
        setError(cause instanceof Error ? cause.message : 'Cannot reach the benchmark server.');
        setConnected(false);
        setRefreshing(false);
        request.abort();
      } finally {
        if (controller.current === request) {
          inFlight.current = null;
          if (!request.signal.aborted) setRefreshing(false);
        }
      }
    })();
    inFlight.current = work;
    return work;
  }, [api]);

  // A mutation may finish while a poll that started before it is still running.
  // Wait for that read, then request a fresh server snapshot before enabling actions.
  const reload = useCallback(async () => {
    if (inFlight.current) await inFlight.current;
    if (!mounted.current) return;
    await refresh();
  }, [refresh]);

  useEffect(() => {
    mounted.current = true;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await refresh();
      if (active) timer = setTimeout(() => void poll(), pollInterval);
    };
    void poll();
    return () => {
      mounted.current = false;
      active = false;
      clearTimeout(timer);
      controller.current?.abort();
      controller.current = null;
      inFlight.current = null;
    };
  }, [refresh, pollInterval]);

  return { catalog, benchmarks, runs, error, connected, refreshing, updatedAt, refresh, reload };
}
