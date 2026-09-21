import type { Plugin } from 'vite';

import { startBackend } from '../../../scripts/backend.mjs';
import packageJson from '../../../package.json';

/** The renderer sees only the local proxy; the server owns compilation and credentials. */
export function benchmarkBackendPlugin(): Plugin {
  return {
    name: 'benchmark-backend',
    apply: 'serve',
    async configureServer(server) {
      if (!server.httpServer) return;
      if (process.env.BENCHMARK_API_TARGET?.trim()) return;
      const declaration = packageJson.genesisDevelopment.proxy;
      const proxy = server.config.server.proxy?.[declaration.path];
      const target = typeof proxy === 'string' ? proxy : proxy?.target;
      const stop = await startBackend({
        target: target ? String(target) : declaration.defaultTarget,
        log: (message) => server.config.logger.info(message),
      });
      server.httpServer?.once('close', stop);
    },
  };
}
