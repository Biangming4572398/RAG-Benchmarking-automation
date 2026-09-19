// @vitest-environment node
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { UserConfigFnObject } from 'vite';

import viteConfig from '../../../vite.config';
import { createBenchmarkProxy } from './proxy';

afterEach(() => vi.unstubAllEnvs());

describe('standalone benchmark backend connection', () => {
  it('uses the existing backend prefix and default port with a server-side bearer token', () => {
    expect(createBenchmarkProxy({ BENCHMARK_API_TOKEN: 'server-only-token' })).toEqual({
      '/api/benchmarks/v1': {
        target: 'http://127.0.0.1:4319',
        changeOrigin: true,
        headers: { Authorization: 'Bearer server-only-token' },
      },
    });
  });

  it('connects to a backend on a configured port', () => {
    const proxy = createBenchmarkProxy({
      BENCHMARK_API_TARGET: 'http://127.0.0.1:54321',
      BENCHMARK_API_TOKEN: 'server-only-token',
    });
    expect(proxy['/api/benchmarks/v1'].target).toBe('http://127.0.0.1:54321');
  });

  it('does not install a proxy until the server-side token is configured', () => {
    expect(createBenchmarkProxy({})).toEqual({});
    expect(() => createBenchmarkProxy({ BENCHMARK_API_TARGET: 'http://127.0.0.1:4319' })).toThrow(
      'BENCHMARK_API_TOKEN is required',
    );
  });

  it.each(['invalid token', 'line\nbreak', '☃', 'x'.repeat(1025)])(
    'rejects invalid bearer headers',
    (token) => {
      expect(() => createBenchmarkProxy({ BENCHMARK_API_TOKEN: token })).toThrow(
        'BENCHMARK_API_TOKEN is invalid',
      );
    },
  );

  it.each(['file:///private/data', 'http://user:password@127.0.0.1:4319'])(
    'rejects non-HTTP or credential-bearing backend URLs',
    (target) => {
      expect(() =>
        createBenchmarkProxy({ BENCHMARK_API_TARGET: target, BENCHMARK_API_TOKEN: 'token' }),
      ).toThrow('BENCHMARK_API_TARGET must be an HTTP URL');
    },
  );

  it('does not resolve backend configuration or expose credentials in a static build', () => {
    vi.stubEnv('BENCHMARK_API_TARGET', 'file:///invalid-target');
    vi.stubEnv('BENCHMARK_API_TOKEN', 'server-only-build-token');
    const config = (viteConfig as UserConfigFnObject)({ command: 'build', mode: 'production' });

    expect(config.server?.proxy).toBeUndefined();
    expect(JSON.stringify(config)).not.toContain('server-only-build-token');
  });
});
