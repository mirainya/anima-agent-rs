import { useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchJson } from './client';
import { useUiStore } from '@/shared/state/useUiStore';

interface SendRequest {
  content: string;
  session_id?: string | null;
}

interface SendResponse {
  ok: boolean;
  accepted: boolean;
  job_id?: string;
  chat_id?: string;
  session_id?: string | null;
  error?: string;
}

async function sendMessageApi(payload: SendRequest): Promise<SendResponse> {
  const endpoint = payload.session_id ? `/api/sessions/${payload.session_id}/send` : '/api/send';
  const body = payload.session_id
    ? JSON.stringify({ content: payload.content })
    : JSON.stringify(payload);

  return fetchJson<SendResponse>(endpoint, {
    method: 'POST',
    body,
  });
}

export function useSendMessageMutation() {
  const queryClient = useQueryClient();
  const setSelectedSessionId = useUiStore((state) => state.setSelectedSessionId);
  const setSelectedJobId = useUiStore((state) => state.setSelectedJobId);
  const setSelectNewestJob = useUiStore((state) => state.setSelectNewestJob);

  return useMutation({
    mutationFn: (payload: SendRequest) => sendMessageApi(payload),
    onSuccess: (data, variables) => {
      const nextSessionId = variables.session_id ?? data.session_id ?? data.chat_id ?? null;
      if (nextSessionId) {
        setSelectedSessionId(nextSessionId);
      }
      if (data.job_id) {
        setSelectedJobId(data.job_id);
      }
      setSelectNewestJob(true);
      queryClient.invalidateQueries({ queryKey: ['status'] });
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      queryClient.invalidateQueries({ queryKey: ['sessions'] });
      queryClient.invalidateQueries({ queryKey: ['sessions', nextSessionId, 'history'] });
    },
  });
}
