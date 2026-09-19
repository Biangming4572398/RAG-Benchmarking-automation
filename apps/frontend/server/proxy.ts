import type { ProxyOptions } from 'vite';

import packageJson from '../../../package.json';

/** Connect the standalone dashboard to the local benchmark backend. */
export function createBenchmarkProxy(
  env: Readonly<Record<string, string | undefined>>,
): Record<string, ProxyOptions> {
  const { path, targetEnv, defaultTarget } = packageJson.genesisDevelopment.proxy;
  const configuredTarget = env[targetEnv]?.trim();
  const target = new URL(configuredTarget || defaultTarget);
  if (!['http:', 'https:'].includes(target.protocol) || target.username || target.password) {
    throw new Error(`${targetEnv} must be an HTTP URL without embedded credentials`);
  }
  if (
    target.hostname !== 'localhost' &&
    target.hostname !== '[::1]' &&
    !/^127\.\d+\.\d+\.\d+$/.test(target.hostname)
  ) {
    throw new Error(`${targetEnv} must use a loopback address`);
  }
  return {
    [path]: {
      target: target.origin,
      changeOrigin: true,
    },
  };
}
