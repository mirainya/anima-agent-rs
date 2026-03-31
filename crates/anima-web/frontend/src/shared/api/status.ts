import { useQuery } from '@tanstack/react-query';
import { fetchJson } from './client';
import { statusSnapshotSchema, type StatusSnapshot } from '@/shared/utils/types';

async function getStatus(): Promise<StatusSnapshot> {
  const data = await fetchJson<unknown>('/api/status');
  return statusSnapshotSchema.parse(data);
}

export function useStatusQuery() {
  return useQuery({
    queryKey: ['status'],
    queryFn: getStatus,
    refetchInterval: 15_000,
  });
}
