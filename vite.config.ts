import { defineConfig } from 'vite';

import { createBenchmarkProxy } from './apps/frontend/server/proxy';

export default defineConfig(({ command }) => ({
  root: 'apps/frontend',
  base: './',
  esbuild: { jsx: 'automatic' },
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
