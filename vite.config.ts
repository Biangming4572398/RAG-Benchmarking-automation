import { defineConfig } from 'vite';

export default defineConfig({
  root: 'apps/frontend',
  base: './',
  esbuild: { jsx: 'automatic' },
  server: { host: '127.0.0.1', open: '/benchmarking.html' },
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
});
