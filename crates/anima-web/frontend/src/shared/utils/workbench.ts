import type { JobView, SessionSummary } from './types';
import type { WorkbenchScope } from '@/shared/state/useUiStore';

export interface WorkbenchContext {
  scope: WorkbenchScope;
  selectedSession: SessionSummary | null;
  selectedJob: JobView | null;
  sessionJobs: JobView[];
}

export function getWorkbenchContext(sessions: SessionSummary[] | undefined, jobs: JobView[], selectedSessionId: string | null, selectedJobId: string | null): WorkbenchContext {
  const sessionList = sessions ?? [];
  const selectedSession = sessionList.find((session) => session.chat_id === selectedSessionId) ?? null;
  const sessionJobs = selectedSession
    ? jobs.filter((job) => job.chat_id && job.chat_id === selectedSession.chat_id)
    : jobs;
  const selectedJob = sessionJobs.find((job) => job.job_id === selectedJobId) ?? null;
  const scope: WorkbenchScope = selectedJob ? 'job' : selectedSession ? 'session' : 'global';

  return {
    scope,
    selectedSession,
    selectedJob,
    sessionJobs,
  };
}
