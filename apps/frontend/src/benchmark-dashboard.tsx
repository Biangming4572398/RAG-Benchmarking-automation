import React, { useEffect, useId, useMemo, useState } from 'react';
import { createBenchmarkApi, type BenchmarkApi } from './benchmark-api';
import { createAnswerApi, type AnswerApi } from './answer-api';
import { AnswerDashboard } from './answer-dashboard';
import { RetrievalDashboard } from './retrieval-dashboard';
import styles from './benchmark-dashboard.module.css';

export interface BenchmarkDashboardProps {
  initialState?: unknown;
  onStateChange?: (state: unknown) => void;
  api?: BenchmarkApi;
  answerApi?: AnswerApi;
  pollInterval?: number;
}

interface DashboardState {
  view: 'retrieval' | 'answers';
  retrieval: unknown;
  answers: unknown;
}

function restore(saved: unknown): DashboardState {
  const state = saved && typeof saved === 'object' ? (saved as Record<string, unknown>) : {};
  return {
    view: state.view === 'answers' ? 'answers' : 'retrieval',
    // Panels saved before tabs existed contain retrieval filters at the top level.
    retrieval: state.retrieval ?? state,
    answers: state.answers,
  };
}

export function BenchmarkDashboard({
  initialState,
  onStateChange,
  api,
  answerApi,
  pollInterval,
}: BenchmarkDashboardProps) {
  const id = useId();
  const [state, setState] = useState(() => restore(initialState));
  const benchmarkApi = useMemo(() => api ?? createBenchmarkApi(), [api]);
  const generatedApi = useMemo(() => answerApi ?? createAnswerApi(), [answerApi]);
  const retainRetrieval = React.useCallback(
    (retrieval: unknown) => setState((current) => ({ ...current, retrieval })),
    [],
  );
  const retainAnswers = React.useCallback(
    (answers: unknown) => setState((current) => ({ ...current, answers })),
    [],
  );
  useEffect(() => onStateChange?.(state), [state, onStateChange]);

  const views: { id: DashboardState['view']; label: string }[] = [
    { id: 'retrieval', label: 'Retrieval' },
    { id: 'answers', label: 'Generated answers' },
  ];
  function keyboard(event: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    if (!['ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? 1 : 1 - index;
    setState((current) => ({ ...current, view: views[next].id }));
    event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      [next]?.focus();
  }

  return (
    <div className={styles.workspace}>
      <div className={styles.tabs} role="tablist" aria-label="Benchmark evaluation">
        {views.map((view, index) => (
          <button
            key={view.id}
            id={`${id}-${view.id}-tab`}
            role="tab"
            aria-selected={state.view === view.id}
            aria-controls={`${id}-${view.id}-panel`}
            tabIndex={state.view === view.id ? 0 : -1}
            onClick={() => setState((current) => ({ ...current, view: view.id }))}
            onKeyDown={(event) => keyboard(event, index)}
          >
            {view.label}
          </button>
        ))}
      </div>
      <div
        className={styles.tabPanel}
        role="tabpanel"
        id={`${id}-${state.view}-panel`}
        aria-labelledby={`${id}-${state.view}-tab`}
      >
        {state.view === 'retrieval' ? (
          <RetrievalDashboard
            api={benchmarkApi}
            initialState={state.retrieval}
            onStateChange={retainRetrieval}
            pollInterval={pollInterval}
          />
        ) : (
          <AnswerDashboard
            api={generatedApi}
            benchmarkApi={benchmarkApi}
            initialState={state.answers}
            onStateChange={retainAnswers}
            pollInterval={pollInterval}
          />
        )}
      </div>
    </div>
  );
}
