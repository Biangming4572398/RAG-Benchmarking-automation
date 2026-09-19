import type {
  GenesisHost,
  GenesisManifest,
  GenesisModule,
  GenesisModuleInstance,
  SerializedInstanceState,
} from '@genesis/sdk';
import { BenchmarkDashboard } from './benchmark-dashboard';

export { BenchmarkDashboard } from './benchmark-dashboard';

class BenchmarkingInstance implements GenesisModuleInstance {
  #state: SerializedInstanceState;

  constructor(initialState?: SerializedInstanceState) {
    this.#state = initialState;
  }

  // Genesis may unmount a panel when changing spaces. Retain the latest local
  // filters in its instance. Server records are refreshed when the panel reopens.
  readonly #onStateChange = (state: unknown): void => {
    this.#state = state;
  };

  render() {
    return <BenchmarkDashboard initialState={this.#state} onStateChange={this.#onStateChange} />;
  }

  serialize(): SerializedInstanceState {
    return this.#state;
  }
}

export class BenchmarkingModule implements GenesisModule {
  declare readonly manifest: GenesisManifest;

  onRegister(host: GenesisHost): void {
    host.log.info('Benchmarking registered.');
  }

  createInstance(initialState?: SerializedInstanceState): GenesisModuleInstance {
    return new BenchmarkingInstance(initialState);
  }
}

export default new BenchmarkingModule();
