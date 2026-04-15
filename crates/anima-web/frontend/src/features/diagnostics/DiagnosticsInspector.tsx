import { useJobsQuery } from '@/shared/api/jobs';
import { useStatusQuery } from '@/shared/api/status';
import { useUiStore, type InspectorTab } from '@/shared/state/useUiStore';
import { formatDurationShort, formatTimestamp, shortId } from '@/shared/utils/format';
import { getWorkbenchContext } from '@/shared/utils/workbench';
import './diagnostics.css';

interface FailureView {
  error_code?: string;
  error_stage?: string;
  internal_message?: string;
  occurred_at_ms?: number;
  chat_id?: string | null;
}

interface ExecutionSummaryView {
  trace_id?: string;
  message_id?: string;
  plan_type?: string;
  status?: string;
  cache_hit?: boolean;
  worker_id?: string | null;
  error_code?: string | null;
  error_stage?: string | null;
  task_duration_ms?: number;
  stages?: {
    context_ms?: number;
    session_ms?: number;
    classify_ms?: number;
    execute_ms?: number;
    total_ms?: number;
  };
}

const tabs: Array<{ key: InspectorTab; label: string }> = [
  { key: 'overview', label: '概览' },
  { key: 'assignments', label: '分配' },
  { key: 'timeline', label: 'Timeline' },
  { key: 'failure', label: 'Failure' },
  { key: 'execution', label: 'Execution' },
];

export function DiagnosticsInspector() {
  const { data: status, isLoading, error } = useStatusQuery();
  const { data: jobs = [] } = useJobsQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const activeInspectorTab = useUiStore((state) => state.activeInspectorTab);
  const setActiveInspectorTab = useUiStore((state) => state.setActiveInspectorTab);
  const isInspectorOpen = useUiStore((state) => state.isInspectorOpen);
  const context = getWorkbenchContext(status?.recent_sessions, jobs, selectedSessionId, selectedJobId);

  if (!isInspectorOpen) {
    return (
      <div className="diagnostics-inspector diagnostics-inspector-collapsed">
        <button type="button" className="diagnostics-collapsed-label" onClick={() => useUiStore.getState().setIsInspectorOpen(true)}>
          Inspector
        </button>
      </div>
    );
  }

  if (isLoading) {
    return <div className="diagnostics-inspector"><div className="sidebar-placeholder">正在加载 Inspector...</div></div>;
  }

  if (error || !status) {
    return <div className="diagnostics-inspector"><div className="sidebar-placeholder">Inspector 加载失败</div></div>;
  }

  const contextFailure = (context.selectedJob?.failure ?? null) as FailureView | null
    ?? (context.selectedSession
      ? (status.failures.last_failure?.chat_id === context.selectedSession.chat_id ? status.failures.last_failure as FailureView : null)
      : status.failures.last_failure as FailureView | null);

  const timeline = (context.selectedJob
    ? status.runtime_timeline.filter((event) => event.message_id === context.selectedJob?.job_id || event.trace_id === context.selectedJob?.trace_id)
    : context.selectedSession
      ? status.runtime_timeline.filter((event) => event.chat_id === context.selectedSession?.chat_id)
      : status.runtime_timeline
  )
    .slice(-8)
    .reverse();

  const executionCandidates = context.selectedJob
    ? status.recent_execution_summaries.filter((summary) => summary.message_id === context.selectedJob?.job_id || summary.trace_id === context.selectedJob?.trace_id)
    : context.selectedSession
      ? status.recent_execution_summaries.filter((summary) => summary.chat_id === context.selectedSession?.chat_id)
      : status.recent_execution_summaries;
  const executionSummary = (executionCandidates[0] ?? context.selectedJob?.execution_summary ?? null) as ExecutionSummaryView | null;

  const selectedWorkerId = context.selectedJob?.worker?.worker_id ?? executionSummary?.worker_id ?? null;

  return (
    <div className="diagnostics-inspector">
      <div className="diagnostics-inspector-header">
        <div>
          <div className="pane-title">Inspector</div>
          <div className="pane-subtitle">次级诊断信息集中展示，不打断主工作流</div>
        </div>
      </div>

      <div className="diagnostics-tabs" role="tablist" aria-label="Inspector tabs">
        {tabs.map((tab) => (
          <button
            key={tab.key}
            type="button"
            className={`diagnostics-tab ${activeInspectorTab === tab.key ? 'active' : ''}`}
            onClick={() => setActiveInspectorTab(tab.key)}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div className="diagnostics-panel-body">
        {activeInspectorTab === 'overview' && (
          <div className="app-diagnostics">
            <div className="diag-card-react">
              <div className="diag-title-react">当前上下文</div>
              <div className="diag-line-react">scope={context.scope}</div>
              <div className="diag-line-react">session={context.selectedSession ? shortId(context.selectedSession.chat_id) : '全部会话'}</div>
              <div className="diag-line-react">job={context.selectedJob ? shortId(context.selectedJob.job_id) : '未选中'}</div>
            </div>
            <div className="diag-card-react">
              <div className="diag-title-react">Agent / Worker Pool</div>
              <div className="diag-line-react">agent={status.agent.status}</div>
              <div className="diag-line-react">context={status.agent.context_status}</div>
              <div className="diag-line-react">workers={status.worker_pool.active}/{status.worker_pool.size}</div>
              <div className="diag-line-react">idle={status.worker_pool.idle} · stopped={status.worker_pool.stopped}</div>
            </div>
            <div className="diag-card-react">
              <div className="diag-title-react">多主体扩展位</div>
              <div className="diag-line-react">actor_role / actor_id / subtask_id 预留中</div>
              <div className="diag-line-react">后续可接 planner / specialist / reviewer / subtask</div>
            </div>
          </div>
        )}

        {activeInspectorTab === 'assignments' && (
          <div className="assignment-list">
            {status.workers.length === 0 ? (
              <div className="sidebar-placeholder">暂无 worker</div>
            ) : (
              status.workers.map((worker) => {
                const busy = worker.status !== 'idle';
                const matchesJob = selectedWorkerId !== null && worker.id === selectedWorkerId;
                return (
                  <div key={worker.id} className={`assignment-card ${matchesJob ? 'focus' : ''}`}>
                    <div className="assignment-card-top">
                      <div className="diag-title-react">{worker.id}</div>
                      <span className={`job-status-pill ${busy ? 'busy' : 'neutral'}`}>{worker.status}</span>
                    </div>
                    <div className="diag-line-react">task={worker.current_task?.task_type ?? '-'}</div>
                    <div className="diag-line-react">trace={shortId(worker.current_task?.trace_id ?? null)}</div>
                    <div className="diag-line-react">task_id={shortId(worker.current_task?.task_id ?? null)}</div>
                    <div className="diag-line-react">elapsed={formatDurationShort(worker.current_task?.elapsed_ms ?? 0)}</div>
                    <div className="diag-line-react">preview={worker.current_task?.content_preview ?? 'idle'}</div>
                  </div>
                );
              })
            )}
          </div>
        )}

        {activeInspectorTab === 'timeline' && (
          <div className="app-diagnostics">
            <div className="diag-card-react">
              <div className="diag-title-react">相关 Timeline</div>
              {timeline.length === 0 ? (
                <div className="sidebar-placeholder">暂无相关事件</div>
              ) : (
                timeline.map((event) => (
                  <div key={`${event.event}-${event.recorded_at_ms}`} className="timeline-row">
                    <div>{event.event}</div>
                    <div className="diag-line-react">{shortId(event.trace_id)} · {formatTimestamp(event.recorded_at_ms)}</div>
                  </div>
                ))
              )}
            </div>
          </div>
        )}

        {activeInspectorTab === 'failure' && (
          <div className="app-diagnostics">
            <div className="diag-card-react">
              <div className="diag-title-react">相关失败</div>
              {contextFailure ? (
                <>
                  <div className="diag-line-react">code={contextFailure.error_code ?? '-'}</div>
                  <div className="diag-line-react">stage={contextFailure.error_stage ?? '-'}</div>
                  <div className="diag-line-react">time={contextFailure.occurred_at_ms ? formatTimestamp(contextFailure.occurred_at_ms) : '-'}</div>
                  <div className="diag-line-react diag-prewrap">{contextFailure.internal_message ?? '-'}</div>
                </>
              ) : (
                <div className="sidebar-placeholder">当前上下文暂无失败</div>
              )}
            </div>
          </div>
        )}

        {activeInspectorTab === 'execution' && (
          <div className="app-diagnostics">
            <div className="diag-card-react">
              <div className="diag-title-react">相关执行摘要</div>
              {executionSummary ? (
                <>
                  <div className="diag-line-react">plan={executionSummary.plan_type ?? '-'}</div>
                  <div className="diag-line-react">status={executionSummary.status ?? '-'}</div>
                  <div className="diag-line-react">cache={executionSummary.cache_hit ? 'hit' : 'miss'}</div>
                  <div className="diag-line-react">worker={executionSummary.worker_id ?? '-'}</div>
                  <div className="diag-line-react">task={formatDurationShort(executionSummary.task_duration_ms ?? 0)}</div>
                  <div className="diag-line-react">ctx {formatDurationShort(executionSummary.stages?.context_ms ?? 0)} · sess {formatDurationShort(executionSummary.stages?.session_ms ?? 0)} · cls {formatDurationShort(executionSummary.stages?.classify_ms ?? 0)} · exe {formatDurationShort(executionSummary.stages?.execute_ms ?? 0)}</div>
                </>
              ) : (
                <div className="sidebar-placeholder">暂无执行摘要</div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
