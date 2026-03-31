import { useStatusQuery } from '@/shared/api/status';
import type { SseConnectionState } from '@/shared/api/events';
import { useJobsQuery } from '@/shared/api/jobs';
import { useUiStore } from '@/shared/state/useUiStore';
import { shortId } from '@/shared/utils/format';
import { getWorkbenchContext } from '@/shared/utils/workbench';

interface RuntimeHeaderProps {
  sseState: SseConnectionState;
}

export function RuntimeHeader({ sseState }: RuntimeHeaderProps) {
  const { data: status } = useStatusQuery();
  const { data: jobs = [] } = useJobsQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const isInspectorOpen = useUiStore((state) => state.isInspectorOpen);
  const toggleInspector = useUiStore((state) => state.toggleInspector);
  const context = getWorkbenchContext(status, jobs, selectedSessionId, selectedJobId);

  const scopeLabel = context.scope === 'job'
    ? `当前 Job ${shortId(context.selectedJob?.job_id ?? null)}`
    : context.scope === 'session'
      ? `当前会话 ${shortId(context.selectedSession?.chat_id ?? null)}`
      : '全部会话';

  return (
    <div className="runtime-header">
      <div className="runtime-header-main">
        <div className="app-title">Anima Agent Workbench</div>
        <div className="runtime-header-subtitle">主工作流优先 · 运行态可解释 · Inspector 抽屉化</div>
      </div>
      <div className="runtime-header-meta">
        <span className={`header-status-dot ${sseState}`} />
        <span>SSE {sseState}</span>
        <span>Agent {status?.agent.status ?? 'loading'}</span>
        <span>Pool {status?.worker_pool.active ?? 0}/{status?.worker_pool.size ?? 0}</span>
        <span>{scopeLabel}</span>
        <button type="button" className="inspector-toggle-btn" onClick={toggleInspector}>
          {isInspectorOpen ? '收起 Inspector' : '展开 Inspector'}
        </button>
      </div>
    </div>
  );
}
