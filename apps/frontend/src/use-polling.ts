import { useCallback, useEffect, useRef, useState } from 'react';

/** Coalesce refreshes and abort reads when the owning tab is closed. */
export function usePolling<T>(
  load: (signal: AbortSignal) => Promise<T>,
  initial: T,
  interval: number,
) {
  const [data, setData] = useState(initial);
  const [connected, setConnected] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [updatedAt, setUpdatedAt] = useState<number>();
  const request = useRef<AbortController | null>(null);
  const inFlight = useRef<Promise<void> | null>(null);
  const mounted = useRef(false);
  const refresh = useCallback((): Promise<void> => {
    if (inFlight.current) return inFlight.current;
    const controller = new AbortController();
    request.current = controller;
    setRefreshing(true);
    const work = (async () => {
      try {
        const next = await load(controller.signal);
        if (controller.signal.aborted) return;
        setData(next);
        setConnected(true);
        setError('');
        setUpdatedAt(Date.now());
      } catch (cause) {
        if (controller.signal.aborted) return;
        setError(cause instanceof Error ? cause.message : 'Cannot read benchmark results.');
        setConnected(false);
        setRefreshing(false);
        controller.abort();
      } finally {
        if (request.current === controller) {
          inFlight.current = null;
          if (!controller.signal.aborted) setRefreshing(false);
        }
      }
    })();
    inFlight.current = work;
    return work;
  }, [load]);
  const reload = useCallback(async () => {
    if (inFlight.current) await inFlight.current;
    if (mounted.current) await refresh();
  }, [refresh]);
  useEffect(() => {
    mounted.current = true;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await refresh();
      if (active) timer = setTimeout(() => void poll(), interval);
    };
    void poll();
    return () => {
      mounted.current = false;
      active = false;
      clearTimeout(timer);
      request.current?.abort();
      request.current = null;
      inFlight.current = null;
    };
  }, [refresh, interval]);
  return { data, connected, refreshing, error, updatedAt, refresh, reload };
}
