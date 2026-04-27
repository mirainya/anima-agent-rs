import { usePlanVerdictMutation } from '@/shared/api/approval';
import type { JobView } from '@/shared/utils/types';

interface PlanApprovalPanelProps {
  job: JobView;
}

interface PlanTask {
  id: string;
  task_type: string;
  payload: Record<string, unknown>;
}

export function PlanApprovalPanel({ job }: PlanApprovalPanelProps) {
  const mutation = usePlanVerdictMutation(job.job_id);
  const pendingPlan = job.pending_plan as { summary?: string; tasks?: PlanTask[] } | undefined;

  if (!pendingPlan) return null;

  const tasks = pendingPlan.tasks ?? [];

  return (
    <div className="plan-approval-panel">
      <div className="plan-approval-header">
        <div className="plan-approval-title">执行计划待审批</div>
        <div className="job-message-meta">{pendingPlan.summary}</div>
      </div>

      {tasks.length > 0 && (
        <div className="plan-approval-tasks">
          {tasks.map((task, idx) => (
            <div key={task.id} className="plan-approval-task-row">
              <span className="plan-approval-task-idx">{idx + 1}</span>
              <span className="plan-approval-task-type">{task.task_type}</span>
              <span className="plan-approval-task-preview">
                {(task.payload?.content as string)?.slice(0, 80) ?? '-'}
              </span>
            </div>
          ))}
        </div>
      )}

      <div className="plan-approval-actions">
        <button
          type="button"
          className="plan-approval-btn approve"
          disabled={mutation.isPending}
          onClick={() => mutation.mutate({ verdict: 'approved' })}
        >
          批准执行
        </button>
        <button
          type="button"
          className="plan-approval-btn reject"
          disabled={mutation.isPending}
          onClick={() => mutation.mutate({ verdict: 'rejected', reason: '用户拒绝' })}
        >
          拒绝
        </button>
      </div>
    </div>
  );
}
