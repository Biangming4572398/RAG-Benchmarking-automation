import { defineConfig } from 'vite';

import { createBenchmarkProxy } from './apps/frontend/server/proxy';
import { benchmarkBackendPlugin } from './apps/frontend/server/backend';

export default defineConfig(({ command, mode }) => ({
  root: 'apps/frontend',
  base: './',
  esbuild: { jsx: 'automatic' },
  plugins: command === 'serve' && mode !== 'test' ? [benchmarkBackendPlugin()] : [],
  server: {
    host: '127.0.0.1',
    open: '/benchmarking.html',
    ...(command === 'serve' ? { proxy: createBenchmarkProxy(process.env) } : {}),
  },
  build: {
    outDir: '../../dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        index: 'apps/frontend/index.html',
        benchmarking: 'apps/frontend/benchmarking.html',
      },
    },
  },
}));
