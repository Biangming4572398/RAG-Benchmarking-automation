interface NebulaConnectionOptions {
  backendDirectory: string;
  env: NodeJS.ProcessEnv;
  log: (message: string) => void;
}

export function connectNebula(options: NebulaConnectionOptions): Promise<{
  env: NodeJS.ProcessEnv;
  stop: () => void;
}>;
