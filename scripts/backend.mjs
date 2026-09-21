import { spawn, execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { access, mkdtemp, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises';
import { constants, createReadStream } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { connectNebula } from './nebula.mjs';

const execute = promisify(execFile);
const backendRoot = fileURLToPath(new URL('../apps/backend/', import.meta.url));
const builds = new Map();

async function digestFile(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

async function sourceFingerprint(directory, env) {
  const hash = createHash('sha256').update(`${process.platform}/${process.arch}/debug/v1`);
  for (const name of [
    'RUSTFLAGS',
    'CARGO_ENCODED_RUSTFLAGS',
    'RUSTUP_TOOLCHAIN',
    'RUSTC',
    'CARGO_BUILD_TARGET',
  ]) {
    hash.update(name).update(env[name] || '');
  }
  async function include(path) {
    try {
      if ((await stat(path)).isDirectory()) {
        for (const name of (await readdir(path)).sort()) await include(join(path, name));
      } else {
        hash.update(path.slice(directory.length)).update(await readFile(path));
      }
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  for (const name of [
    'Cargo.toml',
    'Cargo.lock',
    'build.rs',
    'rust-toolchain',
    'rust-toolchain.toml',
    '.cargo/config',
    '.cargo/config.toml',
    'src',
  ])
    await include(join(directory, name));
  return hash.digest('hex');
}

async function validExecutable(binary, version, env) {
  let probeDirectory;
  try {
    await access(binary, constants.X_OK);
    probeDirectory = await mkdtemp(join(tmpdir(), 'benchmark-binary-probe-'));
    const result = await execute(binary, ['--version'], {
      // macOS can spend several seconds validating a newly linked large binary.
      timeout: 30_000,
      cwd: probeDirectory,
      // Older binaries ignored --version. A missing catalog prevents them from
      // opening the real data directory or changing saved runs during a probe.
      env: {
        ...env,
        BENCHMARK_CATALOG: join(probeDirectory, 'missing-catalog.yaml'),
        BENCHMARK_DATA_DIR: join(probeDirectory, 'data'),
        BENCHMARK_ADDR: '127.0.0.1:0',
      },
    });
    return result.stdout.trim() === `rag-benchmark-backend ${version}`;
  } catch {
    return false;
  } finally {
    if (probeDirectory) await rm(probeDirectory, { recursive: true, force: true });
  }
}

function runCargo(directory, env, log) {
  return new Promise((resolveBuild, reject) => {
    log('Compiling the Rust benchmark backend (the first build can take several minutes)…');
    const child = spawn(
      env.CARGO || 'cargo',
      [
        'build',
        '--locked',
        '--manifest-path',
        join(directory, 'Cargo.toml'),
        '--target-dir',
        join(directory, 'target'),
        '--bin',
        'backend',
      ],
      { cwd: directory, env, stdio: ['ignore', 'inherit', 'inherit'] },
    );
    child.once('error', (error) =>
      reject(
        new Error(
          `Cannot compile the benchmark backend: ${error.message}. Install Rust 1.95 or newer (rustup), then retry.`,
        ),
      ),
    );
    child.once('exit', (code, signal) => {
      if (code === 0) resolveBuild();
      else
        reject(
          new Error(
            `Benchmark backend compilation failed (${signal || `exit ${code}`}). See the Cargo output above.`,
          ),
        );
    });
  });
}

/** Verify both source freshness and executable identity before reusing a local build. */
export async function ensureBackendBinary({
  backendDirectory = backendRoot,
  env = process.env,
  log = console.info,
} = {}) {
  const directory = resolve(backendDirectory);
  if (builds.has(directory)) return builds.get(directory);
  const build = (async () => {
    const binary = join(
      directory,
      'target/debug',
      process.platform === 'win32' ? 'backend.exe' : 'backend',
    );
    const stampPath = join(dirname(binary), '.benchmark-backend-build.json');
    const manifest = await readFile(join(directory, 'Cargo.toml'), 'utf8');
    const version = /^version\s*=\s*"([^"]+)"/m.exec(manifest)?.[1];
    if (!version) throw new Error('Benchmark backend Cargo.toml must declare a package version.');
    const source = await sourceFingerprint(directory, env);
    const executable = await validExecutable(binary, version, env);
    let stamp;
    try {
      stamp = JSON.parse(await readFile(stampPath, 'utf8'));
    } catch {
      /* No validated build yet. */
    }
    if (executable && stamp?.source === source && stamp?.binary === (await digestFile(binary)))
      return binary;

    // Cargo must not reuse a damaged output whose own fingerprint still looks current.
    await rm(binary, { force: true });
    await rm(stampPath, { force: true });
    await runCargo(directory, env, log);
    if (!(await validExecutable(binary, version, env))) {
      throw new Error(
        'Cargo did not produce a valid benchmark backend for this computer. Check the Rust target/toolchain configuration.',
      );
    }
    const temporary = `${stampPath}.${process.pid}.tmp`;
    await writeFile(temporary, JSON.stringify({ source, binary: await digestFile(binary) }));
    await rename(temporary, stampPath);
    return binary;
  })();
  builds.set(directory, build);
  try {
    return await build;
  } finally {
    builds.delete(directory);
  }
}

async function backendState(target) {
  let reachable = false;
  try {
    async function read(path) {
      const response = await fetch(`${target}/api/benchmarks/v1/${path}`, {
        signal: AbortSignal.timeout(1500),
      });
      reachable = true;
      return response.ok ? response.json() : null;
    }
    if ((await read('health'))?.status !== 'ok') return 'incompatible';
    const benchmarks = (await read('catalog'))?.benchmarks;
    if (!benchmarks || typeof benchmarks !== 'object' || Array.isArray(benchmarks))
      return 'incompatible';
    if (!Array.isArray((await read('results'))?.benchmarks)) return 'incompatible';
    if (!Array.isArray(await read('suite-runs'))) return 'incompatible';
    return 'ready';
  } catch {
    return reachable ? 'incompatible' : 'absent';
  }
}

async function reuseExistingBackend(target) {
  const state = await backendState(target);
  if (state === 'incompatible') {
    throw new Error(
      `An incompatible server is already running at ${target}. Stop or restart that server with the rewritten benchmark backend; the dashboard requires the results and suite-runs APIs. The existing process was left running.`,
    );
  }
  return state === 'ready';
}

/** Start only when no existing local API owns the configured target. */
export async function startBackend({
  target = 'http://127.0.0.1:4319',
  backendDirectory = backendRoot,
  env = process.env,
  log = console.info,
} = {}) {
  const address = new URL(target);
  if (
    address.protocol !== 'http:' ||
    address.username ||
    address.password ||
    (!['localhost', '[::1]'].includes(address.hostname) &&
      !/^127\.\d+\.\d+\.\d+$/.test(address.hostname))
  ) {
    throw new Error('The managed benchmark backend requires an HTTP loopback target.');
  }
  target = address.origin;
  if (await reuseExistingBackend(target)) return () => {};
  const binary = await ensureBackendBinary({ backendDirectory, env, log });
  if (await reuseExistingBackend(target)) return () => {};
  const nebula = await connectNebula({ backendDirectory, env, log });
  let child;
  try {
    child = spawn(binary, [], {
      cwd: backendDirectory,
      env: {
        ...nebula.env,
        BENCHMARK_ADDR: `${address.hostname === 'localhost' ? '127.0.0.1' : address.hostname}:${address.port || '80'}`,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  } catch (error) {
    nebula.stop();
    throw error;
  }
  let failure;
  let stderr = '';
  let stdout = '';
  child.stdout.on('data', (chunk) => {
    process.stdout.write(chunk);
    if (address.port !== '0') return;
    stdout = `${stdout}${chunk}`.slice(-8000);
    const port = /BENCHMARK_BACKEND_PORT=(\d+)/.exec(stdout)?.[1];
    if (port) {
      address.port = port;
      target = address.origin;
    }
  });
  child.stderr.on('data', (chunk) => {
    stderr = `${stderr}${chunk}`.slice(-8000);
    process.stderr.write(chunk);
  });
  child.once('error', (error) => {
    failure = error;
  });
  child.once('exit', (code, signal) => {
    nebula.stop();
    failure = new Error(
      `Benchmark backend stopped (${signal || `exit ${code}`}). ${stderr.trim()}`,
    );
  });
  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    process.off('exit', stop);
    nebula.stop();
    child.kill('SIGINT');
    const force = setTimeout(() => child.kill('SIGKILL'), 5000);
    force.unref();
    child.once('exit', () => clearTimeout(force));
  };
  process.once('exit', stop);
  try {
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      if (failure) throw failure;
      if (await reuseExistingBackend(target)) {
        log(`Benchmark backend ready at ${target}`);
        return stop;
      }
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
    }
    throw new Error(
      `Benchmark backend did not become ready at ${target}. Check BENCHMARK_CATALOG and BENCHMARK_DATA_DIR.`,
    );
  } catch (error) {
    stop();
    throw error;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv[2] === 'build') {
      console.info(await ensureBackendBinary());
    } else if (process.argv[2] === 'start') {
      const stop = await startBackend({
        target: `http://${process.env.BENCHMARK_ADDR || '127.0.0.1:4319'}`,
      });
      for (const signal of ['SIGINT', 'SIGTERM'])
        process.once(signal, () => {
          stop();
          process.exitCode = 0;
        });
    } else {
      throw new Error('Usage: node scripts/backend.mjs <build|start>');
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
