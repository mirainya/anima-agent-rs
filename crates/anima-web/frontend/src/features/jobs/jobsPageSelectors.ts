import type { JobView } from '@/shared/utils/types';
import type { WorkbenchScope } from '@/shared/state/useUiStore';
import type { WorkbenchContext } from '@/shared/utils/workbench';
import { shortId } from '@/shared/utils/format';

export function sortJobsByUpdatedAtAsc(jobs: JobView[]): JobView[] {
  return jobs.slice().sort((a, b) => a.updated_at_ms - b.updated_at_ms);
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
