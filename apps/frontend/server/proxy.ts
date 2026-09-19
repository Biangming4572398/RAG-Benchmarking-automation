import type { ProxyOptions } from 'vite';

import packageJson from '../../../package.json';

/** The backend token is resolved only by Vite's Node process, never renderer code. */
export function createBenchmarkProxy(
  env: Readonly<Record<string, string | undefined>>,
): Record<string, ProxyOptions> {
  const { path, targetEnv, tokenEnv, defaultTarget } = packageJson.genesisDevelopment.proxy;
  const configuredTarget = env[targetEnv]?.trim();
  const token = env[tokenEnv];
  if (!configuredTarget && !token) return {};
  if (!token) throw new Error(`${tokenEnv} is required when ${targetEnv} is configured`);
  if (token.length > 1024 || /[^\x21-\x7e]/.test(token)) {
    throw new Error(`${tokenEnv} is invalid`);
  }
  const target = new URL(configuredTarget || defaultTarget);
  if (!['http:', 'https:'].includes(target.protocol) || target.username || target.password) {
    throw new Error(`${targetEnv} must be an HTTP URL without embedded credentials`);
  }
  return {
    [path]: {
      target: target.origin,
      changeOrigin: true,
      headers: { Authorization: `Bearer ${token}` },
    },
  };
}
