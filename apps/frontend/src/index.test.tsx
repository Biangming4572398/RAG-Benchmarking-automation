import { isValidElement, type ReactElement } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { validateManifest } from '@genesis/sdk';
import packageJson from '../../../package.json';
import benchmarking, { BenchmarkDashboard, BenchmarkingModule } from './index';

type DashboardElement = ReactElement<{
  initialState?: unknown;
  onStateChange: (state: unknown) => void;
}>;

describe('Benchmarking Genesis adapter', () => {
  it('declares a discoverable dock item targeting its main panel', () => {
    expect(() => validateManifest(packageJson.genesis, packageJson.name)).not.toThrow();
    expect(packageJson.genesis.dockItem.defaultSanctumPanel).toBe(
      packageJson.genesis.sanctumPanels[0].id,
    );
    expect(packageJson.genesis.id).toBe('benchmarking');
    expect(packageJson.genesis.permissions).toEqual([]);

    const info = vi.fn();
    benchmarking.onRegister({ log: { info } } as never);
    expect(info).toHaveBeenCalledWith('Benchmarking registered.');
    const view = benchmarking.createInstance().render();
    expect(isValidElement(view) && view.type).toBe(BenchmarkDashboard);
  });

  it('retains each panel snapshot when Genesis unmounts and revisits its space', () => {
    const module = new BenchmarkingModule();
    const original = { query: 'transformer' };
    const first = module.createInstance(original);
    const second = module.createInstance({ query: 'recurrent' });
    const firstView = first.render() as DashboardElement;

    expect(firstView.props.initialState).toEqual(original);
    const edited = { query: 'attention' };
    firstView.props.onStateChange(edited);

    expect(first.serialize()).toEqual(edited);
    const restoredView = first.render() as DashboardElement;
    expect(restoredView.props.initialState).toEqual(edited);
    expect(restoredView.props.onStateChange).toBe(firstView.props.onStateChange);
    expect(second.serialize()).toEqual({ query: 'recurrent' });
    const restoredInstance = module.createInstance(first.serialize());
    expect((restoredInstance.render() as DashboardElement).props.initialState).toEqual(edited);
  });
});
