interface BackendOptions {
  backendDirectory?: string;
  env?: NodeJS.ProcessEnv;
  log?: (message: string) => void;
}

export function ensureBackendBinary(options?: BackendOptions): Promise<string>;
export function startBackend(options?: BackendOptions & { target?: string }): Promise<() => void>;
