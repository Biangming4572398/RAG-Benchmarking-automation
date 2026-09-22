import { mkdir, readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

/** Connect through Nebula's public development launcher; credentials stay in that module. */
export async function connectNebula({ backendDirectory, env, log }) {
  const base = env.NEBULA_API_BASE?.trim();
  const token = env.NEBULA_API_TOKEN?.trim();
  if (base || token) {
    if (!base || !token) {
      throw new Error('Set both NEBULA_API_BASE and NEBULA_API_TOKEN for an external Nebula, or neither for automatic startup.');
    }
    return { env, stop: () => {} };
  }

  const directory = resolve(
    env.BENCHMARK_NEBULA_ROOT || resolve(backendDirectory, '../../../../native/Nebula'),
  );
  const manifest = join(directory, 'package.json');
  let pkg;
  try {
    pkg = JSON.parse(await readFile(manifest, 'utf8'));
  } catch (error) {
    if (error.code !== 'ENOENT' || env.BENCHMARK_NEBULA_ROOT) throw error;
    // A standalone benchmark checkout may intentionally have no Nebula installed.
    return { env, stop: () => {} };
  }
  if (pkg.name !== '@genesis/nebula' || !pkg.exports?.['./benchmarking']) {
    throw new Error('Nebula is missing its benchmarking launcher. Update the Genesis benchmarking branch and its submodules, or set an external NEBULA_API_BASE and NEBULA_API_TOKEN.');
  }
  const entry = createRequire(manifest).resolve('@genesis/nebula/benchmarking');
  const { startBenchmarkNebula } = await import(pathToFileURL(entry).href);
  const dataDirectory = resolve(backendDirectory, env.BENCHMARK_DATA_DIR || 'benchmark-data');
  const corpusPath = resolve(env.BENCHMARK_NEBULA_CORPUS || join(dataDirectory, 'nebula/corpus'));
  const moduleStoragePath = resolve(env.BENCHMARK_NEBULA_STORAGE || join(dataDirectory, 'nebula/storage'));
  const modelDirectory = resolve(
    env.BENCHMARK_NEBULA_MODEL_DIR ||
      join(env.HOME || homedir(), '.genesis/storage/.genesis/modules/nebula/models/intfloat-multilingual-e5-small'),
  );
  await mkdir(corpusPath, { recursive: true });
  await mkdir(moduleStoragePath, { recursive: true });
  const runtime = await startBenchmarkNebula({ env, log, corpusPath, moduleStoragePath, modelDirectory });
  log(`Benchmark Nebula corpus: ${corpusPath}`);
  return {
    env: {
      ...env,
      NEBULA_API_BASE: runtime.baseUrl,
      NEBULA_API_TOKEN: runtime.token,
      BENCHMARK_NEBULA_CORPUS: corpusPath,
    },
    stop: runtime.stop,
  };
}
