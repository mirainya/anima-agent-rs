import { useJobsQuery } from '@/shared/api/jobs';
import { useSessionsQuery } from '@/shared/api/sessions';
import { useStatusQuery } from '@/shared/api/status';
import { useUiStore } from '@/shared/state/useUiStore';
import { getWorkbenchContext } from '@/shared/utils/workbench';
import { useNavigate, useParams } from 'react-router-dom';
import {
  buildHierarchySummary,
  buildScopeSummary,
  filterJobsByListFilter,
  pickDetailJob,
  sortJobsByUpdatedAtAsc,
  sortJobsByUpdatedAtDesc,
} from './jobsPageSelectors';

export function useJobsPageViewModel() {
  const navigate = useNavigate();
  const { jobId: routeJobId } = useParams<{ jobId?: string }>();
  const { data: jobs = [] } = useJobsQuery();
  const { data: sessions = [] } = useSessionsQuery();
  const { data: status } = useStatusQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const jobListFilter = useUiStore((state) => state.jobListFilter);
  const setJobListFilter = useUiStore((state) => state.setJobListFilter);
  const setSelectedJobId = useUiStore((state) => state.setSelectedJobId);
  const isJobsDrawerOpen = useUiStore((state) => state.isJobsDrawerOpen);
  const setIsJobsDrawerOpen = useUiStore((state) => state.setIsJobsDrawerOpen);

  const routeSelectedJob = routeJobId ? jobs.find((job) => job.job_id === routeJobId) ?? null : null;
  const effectiveSelectedJobId = routeSelectedJob?.job_id ?? selectedJobId;
  const effectiveSelectedSessionId = routeSelectedJob?.chat_id ?? selectedSessionId;

  const context = getWorkbenchContext(
    sessions,
    jobs,
    effectiveSelectedSessionId,
    effectiveSelectedJobId,
  );
  const visibleJobs = sortJobsByUpdatedAtAsc(
    filterJobsByListFilter(context.sessionJobs, jobListFilter),
  );
  const scopeSummary = buildScopeSummary(context.scope, context);
  const hierarchySummary = buildHierarchySummary(visibleJobs);
  const detailJob = pickDetailJob(context.selectedJob, visibleJobs);
  const drawerJobs = sortJobsByUpdatedAtDesc(visibleJobs);

  const openJobDetails = (jobId: string) => {
    setSelectedJobId(jobId);
    setIsJobsDrawerOpen(true);
    navigate(`/jobs/${jobId}`);
  };

  return {
    status,
    selectedSessionId: effectiveSelectedSessionId,
    selectedSession: context.selectedSession,
    selectedJob: context.selectedJob,
    visibleJobs,
    detailJob,
    drawerJobs,
    scopeSummary,
    hierarchySummary,
    jobListFilter,
    setJobListFilter,
    isJobsDrawerOpen,
    setIsJobsDrawerOpen,
    openJobDetails,
    hasRouteSelection: Boolean(routeSelectedJob),
    sessionJobsCount: context.sessionJobs.length,
  };
}
