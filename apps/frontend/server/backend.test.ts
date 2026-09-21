// @vitest-environment node
import { createServer } from 'node:http';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { ensureBackendBinary, startBackend } from '../../../scripts/backend.mjs';

const directories: string[] = [];
const stops: Array<() => void> = [];

afterEach(async () => {
  stops.splice(0).forEach((stop) => stop());
  await Promise.all(
    directories.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), 'benchmark-build-'));
  directories.push(directory);
  await mkdir(join(directory, 'src'));
  await writeFile(
    join(directory, 'Cargo.toml'),
    '[package]\nname = "backend"\nversion = "0.1.0"\n',
  );
  await writeFile(join(directory, 'Cargo.lock'), 'locked dependencies');
  await writeFile(join(directory, 'src/main.rs'), 'original source');
  const executable = `#!${process.execPath}
if (process.argv[2] === '--version') {
  console.log('rag-benchmark-backend 0.1.0');
} else {
  if (process.env.RECORD_NEBULA_CONNECTION) {
    require('node:fs').writeFileSync(process.env.RECORD_NEBULA_CONNECTION, JSON.stringify({
      base: process.env.NEBULA_API_BASE,
      token: process.env.NEBULA_API_TOKEN,
    }));
  }
  if (process.env.FAIL_BENCHMARK_STARTUP) process.exit(23);
  const http = require('node:http');
  const [host, port] = process.env.BENCHMARK_ADDR.split(':');
  const server = http.createServer((request, response) => {
    response.setHeader('Content-Type', 'application/json');
    const path = request.url.split('/').at(-1);
    response.end(JSON.stringify(path === 'health' ? {status: 'ok'} : path === 'results' ? {benchmarks: []} : path === 'suite-runs' ? [] : {benchmarks: {}}));
  });
  server.listen(Number(port), host, () => console.log('BENCHMARK_BACKEND_PORT=' + server.address().port));
  process.once('SIGINT', () => server.close());
}
`;
  const cargo = join(directory, 'fixture-cargo');
  await writeFile(
    cargo,
    `#!${process.execPath}
const fs = require('node:fs');
const path = require('node:path');
fs.appendFileSync(path.join(process.cwd(), 'builds'), JSON.stringify(process.argv.slice(2)) + '\\n');
fs.mkdirSync(path.join(process.cwd(), 'target/debug'), { recursive: true });
fs.writeFileSync(path.join(process.cwd(), 'target/debug/backend'), ${JSON.stringify(executable)}, { mode: 0o755 });
`,
    { mode: 0o755 },
  );
  return { backendDirectory: directory, env: { ...process.env, CARGO: cargo } as NodeJS.ProcessEnv, log: vi.fn() };
}

async function withNebula(options: Awaited<ReturnType<typeof fixture>>) {
  const directory = join(options.backendDirectory, 'fixture-nebula');
  await mkdir(directory);
  await writeFile(join(directory, 'package.json'), JSON.stringify({
    name: '@genesis/nebula',
    type: 'module',
    exports: { './benchmarking': './launch.mjs' },
  }));
  await writeFile(join(directory, 'launch.mjs'), `
import { appendFileSync } from 'node:fs';
export async function startBenchmarkNebula() {
  const events = new URL('./events', import.meta.url);
  appendFileSync(events, 'start\\n');
  let stopped = false;
  return {
    baseUrl: 'http://127.0.0.1:54321/api/nebula/v1',
    token: 'fixture-nebula-runtime-token',
    stop() {
      if (stopped) return;
      stopped = true;
      appendFileSync(events, 'stop\\n');
    },
  };
}
`);
  options.env.BENCHMARK_NEBULA_ROOT = directory;
  delete options.env.NEBULA_API_BASE;
  delete options.env.NEBULA_API_TOKEN;
  return join(directory, 'events');
}

async function availableTarget() {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Expected a TCP listener');
  const target = `http://127.0.0.1:${address.port}`;
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return target;
}

describe('automatic Rust backend build', () => {
  it('builds a missing binary with locked dependencies and reuses a verified current build', async () => {
    const options = await fixture();
    const binary = await ensureBackendBinary(options);
    expect(binary).toBe(join(options.backendDirectory, 'target/debug/backend'));
    expect(await ensureBackendBinary(options)).toBe(binary);
    const builds = (await readFile(join(options.backendDirectory, 'builds'), 'utf8'))
      .trim()
      .split('\n');
    expect(builds).toHaveLength(1);
    expect(JSON.parse(builds[0])).toEqual([
      'build',
      '--locked',
      '--manifest-path',
      join(options.backendDirectory, 'Cargo.toml'),
      '--target-dir',
      join(options.backendDirectory, 'target'),
      '--bin',
      'backend',
    ]);
  });

  it('rebuilds when Rust sources or the dependency lock change', async () => {
    const options = await fixture();
    await ensureBackendBinary(options);
    await writeFile(join(options.backendDirectory, 'src/main.rs'), 'rewritten source');
    await ensureBackendBinary(options);
    await writeFile(join(options.backendDirectory, 'Cargo.lock'), 'changed dependencies');
    await ensureBackendBinary(options);
    expect(options.log).toHaveBeenCalledTimes(3);
  });

  it('rebuilds an executable with the wrong identity even when its build stamp remains', async () => {
    const options = await fixture();
    const binary = await ensureBackendBinary(options);
    await writeFile(binary, `#!${process.execPath}\nconsole.log('another application');\n`, {
      mode: 0o755,
    });
    await ensureBackendBinary(options);
    expect(options.log).toHaveBeenCalledTimes(2);
  });

  it('shares compilation for simultaneous callers', async () => {
    const options = await fixture();
    const binaries = await Promise.all([
      ensureBackendBinary(options),
      ensureBackendBinary(options),
    ]);
    expect(binaries[0]).toBe(binaries[1]);
    expect(options.log).toHaveBeenCalledTimes(1);
  });

  it('explains how to recover when Rust is not installed', async () => {
    const options = await fixture();
    options.env.CARGO = join(options.backendDirectory, 'missing-cargo');
    await expect(ensureBackendBinary(options)).rejects.toThrow('Install Rust 1.95 or newer');
  });

  it('rejects a failed compilation and retries when the toolchain is repaired', async () => {
    const options = await fixture();
    const failingCargo = join(options.backendDirectory, 'failing-cargo');
    await writeFile(failingCargo, `#!${process.execPath}\nprocess.exit(1);\n`, { mode: 0o755 });
    await expect(
      ensureBackendBinary({ ...options, env: { ...options.env, CARGO: failingCargo } }),
    ).rejects.toThrow('compilation failed (exit 1)');
    await expect(ensureBackendBinary(options)).resolves.toContain('/target/debug/backend');
  });

  it('starts the compiled API and stops the owned child', async () => {
    const options = await fixture();
    const target = await availableTarget();
    const stop = await startBackend({ ...options, target });
    stops.push(stop);
    expect(await (await fetch(`${target}/api/benchmarks/v1/health`)).json()).toEqual({
      status: 'ok',
    });
    stop();
    await vi.waitFor(async () => {
      await expect(fetch(`${target}/api/benchmarks/v1/health`)).rejects.toThrow();
    });
  });

  it('reuses an already healthy server without compiling or taking ownership', async () => {
    const options = await fixture();
    const target = await availableTarget();
    stops.push(await startBackend({ ...options, target }));
    const stopOther = await startBackend({
      ...options,
      target,
      backendDirectory: '/does-not-exist',
    });
    stopOther();
    expect((await fetch(`${target}/api/benchmarks/v1/health`)).ok).toBe(true);
    expect(options.log).toHaveBeenCalledTimes(2);
  });

  it('passes the managed Nebula connection to the backend and stops both on shutdown', async () => {
    const options = await fixture();
    const events = await withNebula(options);
    const connectionFile = join(options.backendDirectory, 'connection.json');
    options.env.RECORD_NEBULA_CONNECTION = connectionFile;
    const target = await availableTarget();
    const stop = await startBackend({ ...options, target });
    stops.push(stop);
    expect(JSON.parse(await readFile(connectionFile, 'utf8'))).toEqual({
      base: 'http://127.0.0.1:54321/api/nebula/v1',
      token: 'fixture-nebula-runtime-token',
    });
    expect(await readFile(events, 'utf8')).toBe('start\n');
    stop();
    stop();
    await vi.waitFor(async () => {
      await expect(fetch(`${target}/api/benchmarks/v1/health`)).rejects.toThrow();
    });
    expect(await readFile(events, 'utf8')).toBe('start\nstop\n');
  });

  it('stops its Nebula when the benchmark backend fails during startup', async () => {
    const options = await fixture();
    const events = await withNebula(options);
    options.env.FAIL_BENCHMARK_STARTUP = '1';
    await expect(startBackend({ ...options, target: await availableTarget() }))
      .rejects.toThrow('exit 23');
    expect(await readFile(events, 'utf8')).toBe('start\nstop\n');
  });

  it('does not start Nebula when reusing an existing compatible benchmark backend', async () => {
    const options = await fixture();
    const target = await availableTarget();
    stops.push(await startBackend({ ...options, target }));
    const events = await withNebula(options);
    const stopOther = await startBackend({ ...options, target });
    stopOther();
    await expect(readFile(events, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
    expect((await fetch(`${target}/api/benchmarks/v1/health`)).ok).toBe(true);
  });

  it('supports an ephemeral backend port reported by the Rust startup handshake', async () => {
    const options = await fixture();
    stops.push(await startBackend({ ...options, target: 'http://127.0.0.1:0' }));
    const message = options.log.mock.calls.at(-1)?.[0] as string;
    const target = message.replace('Benchmark backend ready at ', '');
    expect(new URL(target).port).not.toBe('0');
    expect((await fetch(`${target}/api/benchmarks/v1/health`)).ok).toBe(true);
  });

  it('rejects a legacy API without compiling or stopping the existing server', async () => {
    const options = await fixture();
    const server = createServer((request, response) => {
      response.setHeader('Content-Type', 'application/json');
      if (request.url?.endsWith('/health')) response.end(JSON.stringify({ status: 'ok' }));
      else if (request.url?.endsWith('/catalog')) response.end(JSON.stringify({ benchmarks: {} }));
      else {
        response.statusCode = 404;
        response.end('{}');
      }
    });
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    stops.push(() => server.close());
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('Expected TCP listener');
    const target = `http://127.0.0.1:${address.port}`;
    await expect(startBackend({ ...options, target })).rejects.toThrow(
      'requires the results and suite-runs APIs',
    );
    expect(options.log).not.toHaveBeenCalled();
    expect((await fetch(`${target}/api/benchmarks/v1/health`)).ok).toBe(true);
  });
});
