import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchJson } from './client';

export type ApprovalMode = 'auto' | 'supervised' | 'manual';

interface ApprovalModeResponse {
  mode: ApprovalMode;
}

interface PlanVerdictPayload {
  verdict: 'approved' | 'rejected';
  reason?: string;
}

interface PlanVerdictResponse {
  ok: boolean;
}

async function getApprovalMode(): Promise<ApprovalMode> {
  const data = await fetchJson<ApprovalModeResponse>('/api/approval-mode');
  return data.mode;
}

async function setApprovalMode(mode: ApprovalMode): Promise<ApprovalModeResponse> {
  return fetchJson<ApprovalModeResponse>('/api/approval-mode', {
    method: 'POST',
    body: JSON.stringify({ mode }),
  });
}

async function submitPlanVerdict(jobId: string, payload: PlanVerdictPayload): Promise<PlanVerdictResponse> {
  return fetchJson<PlanVerdictResponse>(`/api/jobs/${encodeURIComponent(jobId)}/plan-verdict`, {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export function useApprovalModeQuery() {
  return useQuery({
    queryKey: ['approval-mode'],
    queryFn: getApprovalMode,
  });
}

export function useApprovalModeMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setApprovalMode,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['approval-mode'] });
    },
  });
}

export function usePlanVerdictMutation(jobId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (payload: PlanVerdictPayload) => {
      if (!jobId) return Promise.reject(new Error('no job id'));
      return submitPlanVerdict(jobId, payload);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      queryClient.invalidateQueries({ queryKey: ['status'] });
    },
  });
}
