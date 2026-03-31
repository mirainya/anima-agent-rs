import { useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchJson } from './client';

export type ReviewVerdict = 'accepted' | 'rejected';

interface ReviewResponse {
  ok: boolean;
}

async function reviewJobApi(jobId: string, verdict: ReviewVerdict): Promise<ReviewResponse> {
  return fetchJson<ReviewResponse>(`/api/jobs/${encodeURIComponent(jobId)}/review`, {
    method: 'POST',
    body: JSON.stringify({ user_verdict: verdict }),
  });
}

export function useReviewJobMutation(jobId: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (verdict: ReviewVerdict) => {
      if (!jobId) {
        return Promise.reject(new Error('no job id'));
      }
      return reviewJobApi(jobId, verdict);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      queryClient.invalidateQueries({ queryKey: ['status'] });
    },
  });
}
