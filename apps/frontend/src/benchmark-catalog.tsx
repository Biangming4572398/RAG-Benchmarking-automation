import React, { useCallback, useState } from 'react';
import type { BenchmarkApi, BenchmarkDefinition, BenchmarkInfo, Catalog } from './benchmark-api';
import { usePolling } from './use-polling';
import styles from './benchmark-dashboard.module.css';

const EMPTY: Catalog = { benchmarks: {} };

function evaluationLabel(evaluation: string): string {
  switch (evaluation) {
    case 'paired_context_recovery_v1':
      return 'Retrieval · paired context';
    case 'hotpotqa_answer_v1':
      return 'Answer exact match / token F1';
    case 'manual_review_v1':
      return 'Human review';
    case 'external_evaluation':
      return 'Dataset-specific evaluation';
    default:
      return evaluation;
  }
}

/** Catalog links open public websites only; local paths, credentials and local hosts stay text. */
function publicLink(value: string | undefined): string | undefined {
  if (!value) return undefined;
  try {
    const url = new URL(value);
    const host = url.hostname.toLowerCase().replace(/\.$/, '');
    if (
      !['http:', 'https:'].includes(url.protocol) ||
      url.username ||
      url.password ||
      !host.includes('.') ||
      /^[\d.]+$/.test(host) ||
      host.includes(':') ||
      /(^|\.)(localhost|local|internal|test)$/.test(host)
    ) {
      return undefined;
    }
    return url.href;
  } catch {
    return undefined;
  }
}

interface CatalogRow {
  key: string;
  definition: BenchmarkDefinition;
  snapshots: BenchmarkInfo[];
  savedOnly: boolean;
}

function rowsFor(catalog: Catalog, snapshots: BenchmarkInfo[]): CatalogRow[] {
  const rows = new Map<string, CatalogRow>(
    Object.entries(catalog.benchmarks).map(([key, definition]): [string, CatalogRow] => [
      key,
      { key, definition, snapshots: [], savedOnly: false },
    ]),
  );
  for (const snapshot of snapshots) {
    const key = snapshot.configuration?.key ?? `saved:${snapshot.id}`;
    let row = rows.get(key);
    if (!row) {
      row = {
        key,
        definition: snapshot.configuration?.definition ?? {
          name: snapshot.source,
          source: snapshot.source,
          split: snapshot.split,
          adapter: '',
          evaluation: snapshot.metric_kind,
          defaults: { limit: snapshot.case_count, top_k: 8 },
        },
        snapshots: [],
        savedOnly: true,
      };
      rows.set(key, row);
    }
    row.snapshots.push(snapshot);
  }
  return [...rows.values()];
}

function CatalogEntry({
  row,
  onOpenAnswers,
}: {
  row: CatalogRow;
  onOpenAnswers?: (benchmarkId: string) => void;
}) {
  const { definition, snapshots } = row;
  const [selection, setSelection] = useState('');
  const external = definition.adapter === 'external_suite';
  const supported = external
    ? []
    : snapshots.filter((snapshot) =>
        ['paired_context_recovery_v1', 'hotpotqa_answer_v1'].includes(snapshot.metric_kind),
      );
  const selected = supported.find((snapshot) => snapshot.id === selection) ?? supported[0];
  const source = publicLink(definition.source);
  const homepage = publicLink(definition.homepage);
  return (
    <tr>
      <th scope="row">
        {definition.name}
        {definition.description && <small>{definition.description}</small>}
        {definition.preparation && (
          <details className={styles.catalogRequirements}>
            <summary>Setup requirements</summary>
            <small>{definition.preparation}</small>
          </details>
        )}
        {row.savedOnly && <small>Saved snapshot definition</small>}
      </th>
      <td>
        <span className={styles.badge}>
          {external
            ? 'Integration required'
            : snapshots.length
              ? `${snapshots.length} prepared snapshot${snapshots.length === 1 ? '' : 's'}`
              : 'Not prepared'}
        </span>
        {supported.length > 1 && (
          <label>
            <span className={styles.catalogSnapshotLabel}>Snapshot for {definition.name}</span>
            <select
              value={selected?.id ?? ''}
              onChange={(event) => setSelection(event.target.value)}
            >
              {supported.map((snapshot) => (
                <option value={snapshot.id} key={snapshot.id}>
                  {snapshot.case_count} questions · {snapshot.id.slice(0, 8)}
                </option>
              ))}
            </select>
          </label>
        )}
        {selected && (
          <small>
            {selected.case_count} questions · {selected.document_count} documents
          </small>
        )}
        {selected && onOpenAnswers && (
          <button
            className={styles.button}
            aria-label={`Open ${definition.name} in Generated answers`}
            onClick={() => onOpenAnswers(selected.id)}
          >
            Generated answers →
          </button>
        )}
        {selected && !onOpenAnswers && <small>Available in Generated answers</small>}
      </td>
      <td>
        {evaluationLabel(definition.evaluation)}
        <small>Split: {definition.split}</small>
      </td>
      <td>
        {source ? (
          <a href={source} target="_blank" rel="noopener noreferrer">
            Dataset source ↗
          </a>
        ) : (
          <span className={styles.muted}>Source configured in YAML</span>
        )}
        {homepage && homepage !== source && (
          <a href={homepage} target="_blank" rel="noopener noreferrer">
            Project page ↗
          </a>
        )}
      </td>
    </tr>
  );
}

export function BenchmarkCatalog({
  api,
  snapshots,
  onOpenAnswers,
  pollInterval = 3000,
}: {
  api: BenchmarkApi;
  snapshots: BenchmarkInfo[];
  onOpenAnswers?: (benchmarkId: string) => void;
  pollInterval?: number;
}) {
  const load = useCallback((signal: AbortSignal) => api.getCatalog(signal), [api]);
  const catalog = usePolling(load, EMPTY, pollInterval);
  const rows = rowsFor(catalog.data, snapshots);
  return (
    <section className={styles.catalog} aria-label="Configured benchmarks">
      <div className={styles.panelHeading}>
        <div>
          <h2>Benchmark catalog</h2>
          <span>YAML / Git</span>
        </div>
        <button
          className={styles.button}
          onClick={() => void catalog.refresh()}
          disabled={catalog.refreshing}
        >
          Refresh catalog
        </button>
      </div>
      <p className={styles.muted}>
        Team benchmark definitions, including datasets that have not been prepared. Setup and
        evaluation integrations are managed in YAML and the backend.
      </p>
      {catalog.error && (
        <p className={styles.error} role="alert">
          Could not refresh the benchmark catalog. {catalog.error}
          {catalog.updatedAt ? ' Previously loaded definitions remain visible.' : ''}
        </p>
      )}
      <div
        className={`${styles.tableScroll} ${styles.catalogTable}`}
        role="region"
        tabIndex={0}
        aria-label="Benchmark catalog table, scroll for all columns"
      >
        <table aria-label="Benchmark catalog">
          <thead>
            <tr>
              <th scope="col">Benchmark</th>
              <th scope="col">Preparation</th>
              <th scope="col">Evaluation</th>
              <th scope="col">Links</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <CatalogEntry key={row.key} row={row} onOpenAnswers={onOpenAnswers} />
            ))}
          </tbody>
        </table>
        {!rows.length && (
          <p className={styles.catalogEmpty}>
            {catalog.refreshing
              ? 'Reading the YAML catalog…'
              : catalog.connected
                ? 'No benchmarks are configured in the YAML catalog.'
                : 'Connect the benchmark server to read its YAML catalog.'}
          </p>
        )}
      </div>
    </section>
  );
}
