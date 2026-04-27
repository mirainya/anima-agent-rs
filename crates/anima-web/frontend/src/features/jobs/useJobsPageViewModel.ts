import { useJobsQuery } from '@/shared/api/jobs';
import { useSessionsQuery } from '@/shared/api/sessions';
import { useUiStore } from '@/shared/state/useUiStore';
import { getWorkbenchContext } from '@/shared/utils/workbench';
import { useParams } from 'react-router-dom';
import { buildScopeSummary, sortJobsByUpdatedAtAsc } from './jobsPageSelectors';

export function useJobsPageViewModel() {
  const { jobId: routeJobId } = useParams<{ jobId?: string }>();
  const { data: jobs = [] } = useJobsQuery();
  const { data: sessions = [] } = useSessionsQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);

  const routeSelectedJob = routeJobId ? jobs.find((job) => job.job_id === routeJobId) ?? null : null;
  const effectiveSelectedJobId = routeSelectedJob?.job_id ?? selectedJobId;
  const effectiveSelectedSessionId = routeSelectedJob?.chat_id ?? selectedSessionId;

  const context = getWorkbenchContext(sessions, jobs, effectiveSelectedSessionId, effectiveSelectedJobId);
  const visibleJobs = sortJobsByUpdatedAtAsc(context.sessionJobs);
  const scopeSummary = buildScopeSummary(context.scope, context);

  return {
    selectedSessionId: effectiveSelectedSessionId,
    selectedSession: context.selectedSession,
    visibleJobs,
    scopeSummary,
  };
}
