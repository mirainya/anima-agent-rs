import type { JobView } from '@/shared/utils/types';
import type { JobListFilter, WorkbenchScope } from '@/shared/state/useUiStore';
import type { WorkbenchContext } from '@/shared/utils/workbench';
import { shortId } from '@/shared/utils/format';

const ACTIVE_JOB_STATUSES = new Set([
  'queued',
  'preparing_context',
  'creating_session',
  'planning',
  'executing',
  'waiting_user_input',
  'stalled',
]);

export function filterJobsByListFilter(jobs: JobView[], filter: JobListFilter): JobView[] {
  return jobs.filter((job) => {
    switch (filter) {
      case 'active':
        return ACTIVE_JOB_STATUSES.has(job.status);
      case 'review':
        return job.status === 'completed' && !job.review;
      case 'failed':
        return job.status === 'failed';
      default:
        return true;
    }
  });
}

export function sortJobsByUpdatedAtAsc(jobs: JobView[]): JobView[] {
  return jobs.slice().sort((a, b) => a.updated_at_ms - b.updated_at_ms);
}

export function sortJobsByUpdatedAtDesc(jobs: JobView[]): JobView[] {
  return jobs.slice().sort((a, b) => b.updated_at_ms - a.updated_at_ms);
}

export function buildScopeSummary(scope: WorkbenchScope, context: WorkbenchContext): string {
  if (scope === 'job') {
    return '当前任务';
  }
  if (scope === 'session') {
    return `当前会话 ${shortId(context.selectedSession?.chat_id ?? null)}`;
  }
  return '全部会话';
}

export function buildHierarchySummary(jobs: JobView[]): { main: number; subtasks: number } {
  return jobs.reduce(
    (summary, job) => {
      if (job.kind === 'subtask') {
        summary.subtasks += 1;
      } else {
        summary.main += 1;
      }
      return summary;
    },
    { main: 0, subtasks: 0 },
  );
}

export function pickDetailJob(selectedJob: JobView | null, visibleJobs: JobView[]): JobView | null {
  return selectedJob ?? visibleJobs[0] ?? null;
}
