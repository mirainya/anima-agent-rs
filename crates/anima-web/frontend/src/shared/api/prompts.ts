import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchJson } from './client';

export interface PromptsConfig {
  task_decompose: string;
  context_infer: string;
  escalation_resolve: string;
  requirement_judge: string;
  question_followup: string;
  result_followup: string;
  agentic_loop_system: string;
  tool_resume_system: string;
  message_classifier: string;
  identity: string;
}

interface PromptsResponse {
  ok: boolean;
  prompts: PromptsConfig;
}

export const PROMPT_LABELS: Record<keyof PromptsConfig, string> = {
  identity: '身份提示词',
  agentic_loop_system: 'Agentic Loop 系统提示词',
  tool_resume_system: '工具恢复系统提示词',
  task_decompose: '任务分解提示词',
  context_infer: '上下文推断提示词',
  escalation_resolve: '升级解决提示词',
  requirement_judge: '需求判断提示词',
  question_followup: '问题追问提示词',
  result_followup: '结果追问提示词',
  message_classifier: '消息分类提示词',
};

async function getPrompts(): Promise<PromptsConfig> {
  const data = await fetchJson<PromptsResponse>('/api/prompts');
  return data.prompts;
}

async function setPrompts(prompts: PromptsConfig): Promise<PromptsConfig> {
  const data = await fetchJson<PromptsResponse>('/api/prompts', {
    method: 'PUT',
    body: JSON.stringify(prompts),
  });
  return data.prompts;
}

export function usePromptsQuery() {
  return useQuery({
    queryKey: ['prompts'],
    queryFn: getPrompts,
  });
}

export function usePromptsMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setPrompts,
    onSuccess: (data) => {
      queryClient.setQueryData(['prompts'], data);
    },
  });
}
