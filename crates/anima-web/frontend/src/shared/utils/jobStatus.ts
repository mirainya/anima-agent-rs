import type { JobStatus } from './types';

export function statusTone(status: JobStatus): 'success' | 'failed' | 'busy' | 'neutral' {
  if (status === 'completed') return 'success';
  if (status === 'failed') return 'failed';
  if (
    status === 'queued' ||
    status === 'preparing_context' ||
    status === 'creating_session' ||
    status === 'planning' ||
    status === 'executing' ||
    status === 'waiting_user_input'
  ) {
    return 'busy';
  }
  return 'neutral';
}

export function statusLabel(status: JobStatus): string {
  switch (status) {
    case 'queued':
      return '已排队';
    case 'preparing_context':
      return '准备上下文';
    case 'creating_session':
      return '创建会话';
    case 'planning':
      return '规划中';
    case 'executing':
      return '执行中';
    case 'waiting_user_input':
      return '等待用户输入';
    case 'stalled':
      return '暂时停滞';
    case 'completed':
      return '已完成';
    case 'failed':
      return '失败';
    case 'awaiting_plan_approval':
      return '等待审批';
  }
}
