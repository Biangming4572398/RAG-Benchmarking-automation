// @vitest-environment node
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { connectNebula } from '../../../scripts/nebula.mjs';

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), 'benchmark-nebula-'));
  directories.push(directory);
  const backendDirectory = join(directory, 'Modules/dev/RAG-Banchmarks/apps/backend');
  const nebulaDirectory = join(directory, 'Modules/native/Nebula');
  await mkdir(backendDirectory, { recursive: true });
  return {
    directory,
    backendDirectory,
    nebulaDirectory,
    env: { HOME: join(directory, 'home') } as NodeJS.ProcessEnv,
    log: vi.fn(),
  };
}

async function installLauncher(directory: string) {
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, 'package.json'), JSON.stringify({
    name: '@genesis/nebula',
    type: 'module',
    exports: { './benchmarking': './launch.mjs' },
  }));
  await writeFile(join(directory, 'launch.mjs'), `
import { writeFileSync } from 'node:fs';
export async function startBenchmarkNebula(options) {
  if (options.env.FAIL_NEBULA_STARTUP) throw new Error('fixture Nebula startup failed');
  writeFileSync(new URL('./started.json', import.meta.url), JSON.stringify({
    env: options.env,
    corpusPath: options.corpusPath,
    moduleStoragePath: options.moduleStoragePath,
    modelDirectory: options.modelDirectory,
  }));
  return {
    baseUrl: 'http://127.0.0.1:54321/api/nebula/v1',
    token: 'fixture-private-runtime-token',
    stop() { writeFileSync(new URL('./stopped', import.meta.url), 'stopped'); },
  };
}
`);
}

describe('benchmark Nebula connection', () => {
  it('keeps an explicit external connection without loading a local launcher', async () => {
    const options = await fixture();
    options.env.NEBULA_API_BASE = 'http://127.0.0.1:9000/api/nebula/v1';
    options.env.NEBULA_API_TOKEN = 'fixture-external-token';
    options.env.BENCHMARK_NEBULA_ROOT = join(options.directory, 'missing-nebula');
    const connection = await connectNebula(options);
    expect(connection.env).toBe(options.env);
    connection.stop();
    expect(options.log).not.toHaveBeenCalled();
  });

  it.each(['NEBULA_API_BASE', 'NEBULA_API_TOKEN'])(
    'rejects a partial external connection containing only %s',
    async (setting) => {
      const options = await fixture();
      options.env[setting] = setting === 'NEBULA_API_BASE'
        ? 'http://127.0.0.1:9000/api/nebula/v1'
        : 'fixture-external-token';
      await expect(connectNebula(options)).rejects.toThrow('Set both NEBULA_API_BASE and NEBULA_API_TOKEN');
    },
  );

  it('allows the standalone dashboard to run when no sibling Nebula exists', async () => {
    const options = await fixture();
    const connection = await connectNebula(options);
    expect(connection.env).toBe(options.env);
    connection.stop();
    expect(options.log).not.toHaveBeenCalled();
    await expect(stat(join(options.backendDirectory, 'benchmark-data'))).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('reports a missing explicitly requested checkout instead of silently disabling Nebula', async () => {
    const options = await fixture();
    options.env.BENCHMARK_NEBULA_ROOT = join(options.directory, 'missing-nebula');
    await expect(connectNebula(options)).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('explains how to update a Nebula checkout without a benchmarking export', async () => {
    const options = await fixture();
    await mkdir(options.nebulaDirectory, { recursive: true });
    await writeFile(join(options.nebulaDirectory, 'package.json'), JSON.stringify({ name: '@genesis/nebula' }));
    await expect(connectNebula(options)).rejects.toThrow('Update the Genesis benchmarking branch and its submodules');
  });

  it('discovers the sibling launcher and isolates its corpus and storage under benchmark data', async () => {
    const options = await fixture();
    await installLauncher(options.nebulaDirectory);
    const connection = await connectNebula(options);
    const started = JSON.parse(await readFile(join(options.nebulaDirectory, 'started.json'), 'utf8'));
    expect(started).toEqual({
      env: options.env,
      corpusPath: join(options.backendDirectory, 'benchmark-data/nebula/corpus'),
      moduleStoragePath: join(options.backendDirectory, 'benchmark-data/nebula/storage'),
      modelDirectory: join(options.env.HOME!, '.genesis/storage/.genesis/modules/nebula/models/intfloat-multilingual-e5-small'),
    });
    expect((await stat(started.corpusPath)).isDirectory()).toBe(true);
    expect((await stat(started.moduleStoragePath)).isDirectory()).toBe(true);
    expect(connection.env).toEqual({
      ...options.env,
      NEBULA_API_BASE: 'http://127.0.0.1:54321/api/nebula/v1',
      NEBULA_API_TOKEN: 'fixture-private-runtime-token',
      BENCHMARK_NEBULA_CORPUS: started.corpusPath,
    });
    expect(options.env.NEBULA_API_TOKEN).toBeUndefined();
    expect(JSON.stringify(options.log.mock.calls)).not.toContain('fixture-private-runtime-token');
    connection.stop();
    expect(await readFile(join(options.nebulaDirectory, 'stopped'), 'utf8')).toBe('stopped');
  });

  it('honors an explicit checkout, corpus, storage, and existing embedding bundle', async () => {
    const options = await fixture();
    const nebulaDirectory = join(options.directory, 'custom-nebula');
    await installLauncher(nebulaDirectory);
    Object.assign(options.env, {
      BENCHMARK_NEBULA_ROOT: nebulaDirectory,
      BENCHMARK_NEBULA_CORPUS: join(options.directory, 'custom-corpus'),
      BENCHMARK_NEBULA_STORAGE: join(options.directory, 'custom-storage'),
      BENCHMARK_NEBULA_MODEL_DIR: join(options.directory, 'existing-model'),
    });
    const connection = await connectNebula(options);
    const started = JSON.parse(await readFile(join(nebulaDirectory, 'started.json'), 'utf8'));
    expect(started.corpusPath).toBe(options.env.BENCHMARK_NEBULA_CORPUS);
    expect(started.moduleStoragePath).toBe(options.env.BENCHMARK_NEBULA_STORAGE);
    expect(started.modelDirectory).toBe(options.env.BENCHMARK_NEBULA_MODEL_DIR);
    connection.stop();
  });

  it('places default isolated storage in the configured benchmark data directory', async () => {
    const options = await fixture();
    await installLauncher(options.nebulaDirectory);
    options.env.BENCHMARK_DATA_DIR = '../shared-benchmark-data';
    const connection = await connectNebula(options);
    const started = JSON.parse(await readFile(join(options.nebulaDirectory, 'started.json'), 'utf8'));
    expect(started.corpusPath).toBe(join(options.backendDirectory, '../shared-benchmark-data/nebula/corpus'));
    expect(started.moduleStoragePath).toBe(join(options.backendDirectory, '../shared-benchmark-data/nebula/storage'));
    connection.stop();
  });

  it('propagates a launcher failure without claiming a working connection', async () => {
    const options = await fixture();
    await installLauncher(options.nebulaDirectory);
    options.env.FAIL_NEBULA_STARTUP = '1';
    await expect(connectNebula(options)).rejects.toThrow('fixture Nebula startup failed');
    expect(options.log).not.toHaveBeenCalled();
  });
});
