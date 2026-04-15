import { useJobsQuery } from '@/shared/api/jobs';
import { useStatusQuery } from '@/shared/api/status';
import type { SseConnectionState } from '@/shared/api/events';
import { useUiStore } from '@/shared/state/useUiStore';
import { shortId } from '@/shared/utils/format';
import { getWorkbenchContext } from '@/shared/utils/workbench';

interface RuntimeHeaderProps {
  sseState: SseConnectionState;
}

interface HeaderStep {
  id: string;
  label: string;
  tone: 'done' | 'active' | 'pending';
}

function buildHeaderSteps(currentStep: string | null | undefined, jobStatus: string | undefined): HeaderStep[] {
  const status = jobStatus ?? 'queued';
  const isCompleted = status === 'completed';
  const isFailed = status === 'failed';
  const isWaiting = status === 'waiting_user_input';
  const current = currentStep || (isCompleted ? '任务完成' : isFailed ? '任务失败' : isWaiting ? '等待输入' : '正在执行');

  return [
    { id: 'context', label: '上下文', tone: ['planning', 'executing', 'waiting_user_input', 'completed', 'failed', 'stalled'].includes(status) ? 'done' : 'active' },
    { id: 'session', label: '会话', tone: ['creating_session', 'planning', 'executing', 'waiting_user_input', 'completed', 'failed', 'stalled'].includes(status) ? 'done' : status === 'preparing_context' || status === 'queued' ? 'pending' : 'active' },
    { id: 'execute', label: current, tone: isCompleted || isFailed ? 'done' : 'active' },
    { id: 'finish', label: isFailed ? '失败' : isCompleted ? '完成' : isWaiting ? '待确认' : '收口', tone: isCompleted || isFailed ? 'done' : 'pending' },
  ];
}

export function RuntimeHeader({ sseState }: RuntimeHeaderProps) {
  const { data: status } = useStatusQuery();
  const { data: jobs = [] } = useJobsQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const isInspectorOpen = useUiStore((state) => state.isInspectorOpen);
  const toggleInspector = useUiStore((state) => state.toggleInspector);
  const context = getWorkbenchContext(status?.recent_sessions, jobs, selectedSessionId, selectedJobId);
  const selectedJob = context.selectedJob;
  const steps = buildHeaderSteps(selectedJob?.current_step, selectedJob?.status);
  const workers = status?.workers ?? [];

  const scopeLabel = context.scope === 'job'
    ? `当前 Job ${shortId(selectedJob?.job_id ?? null)}`
    : context.scope === 'session'
      ? `当前会话 ${shortId(context.selectedSession?.chat_id ?? null)}`
      : '全部会话';

  return (
    <div className="runtime-header">
      <div className="runtime-header-cluster">
        <div className="runtime-header-main">
          <div className="app-title">anima-agent-rs <span className="app-title-version">workbench</span></div>
          <div className="runtime-header-subtitle">
            <span className={`header-status-dot ${sseState}`} />
            <span>{sseState === 'connected' ? '运行时就绪 - 多智能体协作模式' : sseState === 'connecting' ? '正在连接运行时事件流' : '事件流已断开'}</span>
          </div>
        </div>

        <div className="runtime-steps" aria-label="runtime steps">
          {steps.map((step, idx) => (
            <div key={step.id} className="runtime-step-item">
              <div className={`runtime-step-dot ${step.tone}`}>{step.tone === 'done' ? '✓' : idx + 1}</div>
              <span className={`runtime-step-label ${step.tone}`}>{step.label}</span>
              {idx < steps.length - 1 ? <span className="runtime-step-separator">›</span> : null}
            </div>
          ))}
        </div>
      </div>

      <div className="runtime-header-side">
        <div className="runtime-worker-stack" aria-label="workers">
          {workers.slice(0, 4).map((worker) => (
            <div key={worker.id} className={`runtime-worker-chip ${worker.status !== 'idle' ? 'active' : ''}`} title={`${worker.id} · ${worker.status}`}>
              {worker.id.slice(0, 1).toUpperCase()}
            </div>
          ))}
        </div>
        <div className="runtime-header-meta">
          <span>Agent {status?.agent.status ?? 'loading'}</span>
          <span>Pool {status?.worker_pool.active ?? 0}/{status?.worker_pool.size ?? 0}</span>
          <span>{scopeLabel}</span>
        </div>
        <button type="button" className="inspector-toggle-btn" onClick={toggleInspector}>
          {isInspectorOpen ? '收起 Inspector' : '展开 Inspector'}
        </button>
      </div>
    </div>
  );
}
