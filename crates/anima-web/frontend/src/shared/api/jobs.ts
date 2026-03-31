import { useQuery } from '@tanstack/react-query';
import { fetchJson } from './client';
import { jobsResponseSchema, type JobView } from '@/shared/utils/types';

async function getJobs(): Promise<JobView[]> {
  const data = await fetchJson<unknown>('/api/jobs');
  return jobsResponseSchema.parse(data).jobs;
}

export function useJobsQuery() {
  return useQuery({
    queryKey: ['jobs'],
    queryFn: getJobs,
    refetchInterval: 15_000,
  });
}
