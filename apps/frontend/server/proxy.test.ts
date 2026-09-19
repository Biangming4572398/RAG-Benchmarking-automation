// @vitest-environment node
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { UserConfigFnObject } from 'vite';

import viteConfig from '../../../vite.config';
import { createBenchmarkProxy } from './proxy';

afterEach(() => vi.unstubAllEnvs());

describe('standalone benchmark backend connection', () => {
  it('connects to the default local backend without credentials', () => {
    expect(createBenchmarkProxy({})).toEqual({
      '/api/benchmarks/v1': {
        target: 'http://127.0.0.1:4319',
        changeOrigin: true,
      },
    });
  });

  it('connects to a backend on a configured port', () => {
    const proxy = createBenchmarkProxy({
      BENCHMARK_API_TARGET: 'http://127.0.0.1:54321',
    });
    expect(proxy['/api/benchmarks/v1'].target).toBe('http://127.0.0.1:54321');
  });

  it.each(['http://localhost:4319', 'http://[::1]:4319', 'http://127.0.0.2:4319'])(
    'supports loopback targets',
    (target) => {
      expect(
        createBenchmarkProxy({ BENCHMARK_API_TARGET: target })['/api/benchmarks/v1'].target,
      ).toBe(target);
    },
  );

  it.each(['http://example.com:4319', 'http://192.168.1.2:4319', 'http://0.0.0.0:4319'])(
    'rejects non-loopback targets',
    (target) => {
      expect(() => createBenchmarkProxy({ BENCHMARK_API_TARGET: target })).toThrow(
        'BENCHMARK_API_TARGET must use a loopback address',
      );
    },
  );

  it.each(['file:///private/data', 'http://user:password@127.0.0.1:4319'])(
    'rejects non-HTTP or credential-bearing backend URLs',
    (target) => {
      expect(() => createBenchmarkProxy({ BENCHMARK_API_TARGET: target })).toThrow(
        'BENCHMARK_API_TARGET must be an HTTP URL',
      );
    },
  );

  it('does not resolve backend configuration in a static build', () => {
    vi.stubEnv('BENCHMARK_API_TARGET', 'file:///invalid-target');
    const config = (viteConfig as UserConfigFnObject)({ command: 'build', mode: 'production' });

    expect(config.server?.proxy).toBeUndefined();
  });
});
