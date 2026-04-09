import { useQuery } from '@tanstack/react-query';
import { fetchJson } from './client';
import { sessionHistoryResponseSchema, sessionsResponseSchema, type SessionHistoryItem, type SessionSummary } from '@/shared/utils/types';

async function getSessions(): Promise<SessionSummary[]> {
  const data = await fetchJson<unknown>('/api/sessions');
  return sessionsResponseSchema.parse(data).sessions;
}

async function getSessionHistory(sessionId: string): Promise<SessionHistoryItem[]> {
  const data = await fetchJson<unknown>(`/api/sessions/${sessionId}/history`);
  return sessionHistoryResponseSchema.parse(data).history;
}

export function useSessionsQuery() {
  return useQuery({
    queryKey: ['sessions'],
    queryFn: getSessions,
    refetchInterval: 15_000,
  });
}

export function useSessionHistoryQuery(sessionId: string | null) {
  return useQuery({
    queryKey: ['sessions', sessionId, 'history'],
    queryFn: () => getSessionHistory(sessionId!),
    enabled: Boolean(sessionId),
    refetchInterval: 15_000,
  });
}
