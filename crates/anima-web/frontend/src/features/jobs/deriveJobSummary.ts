import type { JobView } from '@/shared/utils/types';
import { statusLabel, statusTone } from '@/shared/utils/jobStatus';

interface JobEventPayloadView {
  response_preview?: string;
  response_text?: string;
}

export interface DerivedJobSummary {
  statusLabelText: string;
  statusTone: ReturnType<typeof statusTone>;
  headline: string;
  detail: string;
  latestResponseText: string;
  hasResponse: boolean;
  needsUserAction: boolean;
  isAutoContinuing: boolean;
  isStalledWithoutQuestion: boolean;
  hierarchyText: string;
}

export function getLatestMessage(job: JobView): JobEventPayloadView | null {
  const event = job.recent_events
    .slice()
    .reverse()
    .find((item) => item.event === 'message_completed');

  if (!event || typeof event.payload !== 'object' || event.payload === null) {
    return null;
  }

  return event.payload as JobEventPayloadView;
}

export function explainJobStatus(job: {
  status: string;
  worker?: { task_type?: string; worker_id?: string; phase?: string | null } | null;
  tool_state?: { phase?: string; tool_name?: string | null; result_preview?: string | null; error?: string | null } | null;
  review?: { verdict?: string } | null;
  failure?: JobView['failure'] | null;
  execution_summary?: JobView['execution_summary'] | null;
  recent_events: Array<{ event: string }>;
}): string {
  const lastEvent = job.recent_events[job.recent_events.length - 1]?.event;

  switch (job.status) {
    case 'completed':
      if (job.review?.verdict === 'accepted') {
        return '主 agent 已判定结果满足需求，且用户已接受这次结果。';
      }
      if (job.review?.verdict === 'rejected') {
        return '主 agent 已判定结果满足需求，但用户拒绝了这次结果；这属于反馈，不会改变主任务状态。';
      }
      return '主 agent 已判定结果满足需求，当前可以继续提交结果反馈。';
    case 'failed':
      if (job.failure?.error_code || job.failure?.error_stage) {
        return `因为运行时记录了失败快照（${job.failure.error_stage ?? '-'} / ${job.failure.error_code ?? '-'}），所以该 Job 已失败。`;
      }
      if (lastEvent === 'session_create_failed') {
        return '因为上游会话创建失败，所以该 Job 被判定为失败。';
      }
      if (lastEvent === 'message_failed') {
        return '因为消息执行阶段返回失败事件，所以该 Job 被判定为失败。';
      }
      return '该 Job 遇到了失败信号，当前需要排查失败原因。';
    case 'executing':
      if (lastEvent === 'requirement_followup_scheduled') {
        return '主 agent 判断上一轮结果还未满足原始需求，正在自动继续补充处理。';
      }
      if (job.tool_state?.phase) {
        const toolName = job.tool_state.tool_name ?? '当前工具';
        if (job.tool_state.phase === 'permission_resolved') {
          return `工具 ${toolName} 的权限已经确认，系统正准备继续执行。`;
        }
        if (job.tool_state.phase === 'executing' || job.tool_state.phase === 'execution_started') {
          return `当前正在执行工具 ${toolName}，因此该 Job 仍处于执行中。`;
        }
        if (job.tool_state.phase === 'result_recorded') {
          return job.tool_state.result_preview
            ? `工具 ${toolName} 的结果已记录：${job.tool_state.result_preview}`
            : `工具 ${toolName} 的结果已记录，等待后续步骤继续推进。`;
        }
        if (job.tool_state.phase === 'execution_failed' || job.tool_state.phase === 'failed') {
          return job.tool_state.error
            ? `工具 ${toolName} 执行失败：${job.tool_state.error}`
            : `工具 ${toolName} 执行失败，系统正在处理失败结果。`;
        }
      }
      if (job.worker?.task_type) {
        return job.worker.phase
          ? `因为 worker 正在执行 ${job.worker.task_type}（${job.worker.phase}），所以该 Job 仍处于执行中。`
          : `因为 worker 正在执行 ${job.worker.task_type}，所以该 Job 仍处于执行中。`;
      }
      if (lastEvent === 'cache_hit' || lastEvent === 'cache_miss') {
        return `因为最近事件是 ${lastEvent}，说明任务已经进入执行阶段，所以当前仍显示为执行中。`;
      }
      return '该 Job 已经过了规划阶段，正在执行实际任务。';
    case 'creating_session':
      return '因为当前 worker 正在执行 session-create，所以系统判断它正在创建上游会话。';
    case 'planning':
      if (lastEvent === 'plan_built') {
        return `因为规划刚完成，正在准备执行 ${job.execution_summary?.plan_type ?? '当前计划'}。`;
      }
      if (lastEvent === 'session_ready') {
        return '因为上游会话已经就绪，系统正在构建后续执行计划。';
      }
      return '该 Job 正在进行规划与执行前准备。';
    case 'waiting_user_input':
      return '只有当主 agent 判定确实缺少外部用户信息时，系统才会展示这个状态，并允许提交回答。';
    case 'stalled':
      return '当前只是缺少新的运行时进展提示，不代表存在可以立即回答的问题。';
    case 'preparing_context':
      return '因为已经收到消息，但还没进入会话或规划阶段，所以当前显示为准备上下文。';
    case 'queued':
      return '因为 Job 已被接收，但还没有出现更进一步的运行时事件，所以当前仍在队列中。';
    default:
      return '当前状态由最近的运行时事件、worker 情况和 review / failure 信息共同推导得出。';
  }
}

export function deriveJobSummary(job: JobView): DerivedJobSummary {
  const failure = (job.failure ?? null) as FailureView | null;
  const executionSummary = (job.execution_summary ?? null) as ExecutionSummaryView | null;
  const latestMessage = getLatestMessage(job);
  const latestResponseText = latestMessage?.response_text ?? latestMessage?.response_preview ?? '';
  const hasResponse = Boolean(latestResponseText);
  const isAutoContinuing = job.status === 'executing' && job.recent_events.some((event) => event.event === 'requirement_followup_scheduled');
  const toolPermissionQuestion = job.pending_question?.raw_question && typeof job.pending_question.raw_question === 'object' && job.pending_question.raw_question !== null && 'type' in job.pending_question.raw_question && job.pending_question.raw_question.type === 'tool_permission'
    ? job.pending_question.raw_question as { type: 'tool_permission'; tool_name?: string; tool_use_id?: string; input_preview?: string }
    : null;
  const needsUserAction = job.status === 'waiting_user_input' && Boolean(job.pending_question);
  const isStalledWithoutQuestion = job.status === 'stalled' && !job.pending_question;
  const hierarchyText = job.parent_job_id
    ? `该任务当前归属于主任务 ${job.parent_job_id.slice(0, 8)}。`
    : job.kind === 'main'
      ? '该任务当前作为主任务展示。'
      : '该任务已标记为子任务，但暂未识别到父任务。';

  let headline = job.current_step || statusLabel(job.status);
  let detail = explainJobStatus({
    status: job.status,
    worker: job.worker,
    tool_state: job.tool_state,
    review: job.review,
    failure,
    execution_summary: executionSummary,
    recent_events: job.recent_events,
  });

  if (job.status === 'failed') {
    headline = failure?.error_stage ? `失败于 ${failure.error_stage}` : statusLabel(job.status);
    detail = failure?.error_code
      ? `失败快照：${failure.error_stage ?? '未知阶段'} / ${failure.error_code}`
      : detail;
  } else if (job.status === 'completed') {
    headline = job.review?.verdict === 'accepted'
      ? '结果已被接受'
      : job.review?.verdict === 'rejected'
        ? '结果已被拒绝'
        : '任务已完成，等待反馈';
  } else if (needsUserAction) {
    headline = toolPermissionQuestion
      ? `等待确认工具权限：${toolPermissionQuestion.tool_name ?? 'unknown'}`
      : job.pending_question?.prompt || '等待用户提供补充信息';
    detail = job.pending_question?.answer_summary
      ? `最近已提交回答：${job.pending_question.answer_summary}`
      : toolPermissionQuestion
        ? `工具调用 ${toolPermissionQuestion.tool_use_id ?? '-'} 需要用户确认后才能继续执行。`
        : '存在真实结构化问题，等待用户提供外部输入。';
  } else if (job.tool_state?.phase) {
    headline = job.current_step || `工具阶段：${job.tool_state.phase}`;
    detail = job.tool_state.error
      ? job.tool_state.error
      : job.tool_state.result_preview
        ? job.tool_state.result_preview
        : job.tool_state.input_preview
          ? `工具输入摘要：${job.tool_state.input_preview}`
          : detail;
  } else if (isAutoContinuing) {
    headline = job.current_step || '系统正在自动继续';
    detail = '主 agent 正在自动继续补充处理，当前不需要用户回答。';
  } else if (isStalledWithoutQuestion) {
    headline = job.current_step || '当前暂无新进展';
    detail = '当前没有结构化问题可提交，只是暂时缺少新的运行时进展。';
  } else if (hasResponse) {
    detail = latestResponseText;
  }

  return {
    statusLabelText: statusLabel(job.status),
    statusTone: statusTone(job.status),
    headline,
    detail,
    latestResponseText,
    hasResponse,
    needsUserAction,
    isAutoContinuing,
    isStalledWithoutQuestion,
    hierarchyText,
  };
}
