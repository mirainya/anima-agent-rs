import { useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchJson } from './client';

export interface QuestionAnswerPayload {
  question_id: string;
  source: 'agent' | 'user';
  answer_type: 'confirm' | 'choice' | 'text';
  answer: string;
}

interface QuestionAnswerResponse {
  ok: boolean;
}

async function answerQuestionApi(jobId: string, payload: QuestionAnswerPayload): Promise<QuestionAnswerResponse> {
  return fetchJson<QuestionAnswerResponse>(`/api/jobs/${encodeURIComponent(jobId)}/question-answer`, {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export function useQuestionAnswerMutation(jobId: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: QuestionAnswerPayload) => {
      if (!jobId) {
        return Promise.reject(new Error('no job id'));
      }
      return answerQuestionApi(jobId, payload);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      queryClient.invalidateQueries({ queryKey: ['status'] });
    },
  });
}
