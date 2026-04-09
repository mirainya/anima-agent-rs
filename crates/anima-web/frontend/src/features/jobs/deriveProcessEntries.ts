import type { JobView, StatusSnapshot } from '@/shared/utils/types';
import { formatActiveSubtask } from './formatOrchestration';

export type TimelineEvent = StatusSnapshot['runtime_timeline'][number];
export type ProcessSource = 'job' | 'runtime';

export interface ProcessEntry {
  id: string;
  kind:
    | 'assignment'
    | 'api_start'
    | 'upstream_response'
    | 'question_asked'
    | 'question_answer'
    | 'question_resolved'
    | 'followup'
    | 'tool_detected'
    | 'tool_permission'
    | 'tool_execution'
    | 'tool_result'
    | 'completed'
    | 'failed'
    | 'other';
  event: string;
  timestamp: number;
  title: string;
  detail: string;
  preview?: string;
  raw?: unknown;
  source: ProcessSource;
}

export interface AgentDecisionSummary {
  title: string;
  reason: string;
  nextAction: string;
  timestamp?: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function getString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined;
}

function createEntry(
  source: ProcessSource,
  event: string,
  timestamp: number,
  payload: unknown,
  index: number,
): ProcessEntry {
  const view = isRecord(payload) ? payload : {};
  const taskType = getString(view.task_type) ?? 'unknown';
  const workerId = getString(view.worker_id);
  const executionKind = getString(view.execution_kind);
  const questionId = getString(view.question_id);
  const opencodeSessionId = getString(view.opencode_session_id);
  const responsePreview = getString(view.response_preview);
  const taskSummary = getString(view.task_summary);
  const taskPreview = getString(view.task_preview);
  const requestPreview = getString(view.request_preview);
  const provider = getString(view.provider);
  const operation = getString(view.operation);
  const prompt = getString(view.prompt);
  const answer = getString(view.answer);
  const answerSummary = getString(view.answer_summary);
  const resolutionSource = getString(view.resolution_source);
  const responseText = getString(view.response_text);
  const errorCode = getString(view.error_code);
  const error = getString(view.error);
  const reason = getString(view.reason);
  const rawQuestion = view.raw_question;
  const rawResult = view.raw_result;
  const subtaskName = getString(view.subtask_name);
  const loweredTaskType = getString(view.lowered_task_type);
  const originalTaskType = getString(view.original_task_type);
  const executionMode = getString(view.execution_mode);
  const resultKind = getString(view.result_kind);
  const parallelSafe = typeof view.parallel_safe === 'boolean' ? view.parallel_safe : undefined;
  const parallelGroupIndex = typeof view.parallel_group_index === 'number' ? view.parallel_group_index : undefined;
  const parallelGroupSize = typeof view.parallel_group_size === 'number' ? view.parallel_group_size : undefined;
  const phase = getString(view.phase);
  const deltaKind = getString(view.delta_kind);
  const textDelta = getString(view.text_delta);
  const accumulatedTextPreview = getString(view.accumulated_text_preview);
  const contentBlockKind = getString(view.content_block_kind);
  const toolName = getString(view.tool_name);
  const toolUseId = getString(view.tool_use_id);
  const invocationId = getString(view.invocation_id);
  const permissionState = getString(view.permission_state);
  const resultSummary = getString(view.result_summary);
  const errorSummary = getString(view.error_summary);
  const details = isRecord(view.details) ? view.details : {};
  const toolInput = details.tool_input ?? view.tool_input;
  const inputPreview = getString(view.input_preview)
    ?? (toolInput !== undefined ? JSON.stringify(toolInput) : undefined);
  const decision = getString(details.decision);
  const partialJson = getString(view.partial_json);
  const stopReason = getString(view.stop_reason);

  switch (event) {
    case 'worker_task_assigned':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'assignment',
        event,
        timestamp,
        title: '主 agent 已派发任务给 worker',
        detail: taskSummary ?? `任务类型：${taskType}${executionKind ? ` · ${executionKind}` : ''}`,
        preview: taskPreview ?? opencodeSessionId,
        raw: view,
        source,
      };
    case 'api_call_started':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'api_start',
        event,
        timestamp,
        title: 'worker 开始调用上游',
        detail: `任务类型：${taskType}${executionKind ? ` · ${executionKind}` : ''}`,
        preview: requestPreview ?? opencodeSessionId,
        raw: view,
        source,
      };
    case 'upstream_response_observed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: '已收到上游响应',
        detail: [workerId, taskType, provider, operation].filter(Boolean).join(' · ') || '收到上游返回结果',
        preview: responsePreview ?? responseText ?? opencodeSessionId,
        raw: rawResult ?? view,
        source,
      };
    case 'question_asked':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'question_asked',
        event,
        timestamp,
        title: '触发结构化问题',
        detail: questionId ? `question=${questionId}` : '上游要求补充信息',
        preview: prompt,
        raw: rawQuestion ?? view,
        source,
      };
    case 'question_answer_submitted':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'question_answer',
        event,
        timestamp,
        title: '已提交问题回答',
        detail: resolutionSource ? `来源：${resolutionSource}` : '用户已提交回答',
        preview: answerSummary ?? answer,
        raw: view,
        source,
      };
    case 'question_resolved':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'question_resolved',
        event,
        timestamp,
        title: '问题已解决',
        detail: questionId ? `question=${questionId}` : '结构化问题已结束',
        preview: answerSummary ?? resolutionSource ?? opencodeSessionId,
        raw: view,
        source,
      };
    case 'requirement_followup_scheduled':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'followup',
        event,
        timestamp,
        title: '主 agent 安排自动 followup',
        detail: reason ?? '当前结果尚未满足需求，系统准备继续推进',
        preview: getString(view.followup_prompt),
        raw: view,
        source,
      };
    case 'orchestration_subtask_started': {
      const groupLabel = parallelGroupIndex !== undefined
        ? `第 ${parallelGroupIndex + 1} 组${parallelGroupSize ? ` / 共 ${parallelGroupSize} 个` : ''}`
        : '编排子任务';
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'assignment',
        event,
        timestamp,
        title: `开始执行子任务：${subtaskName ?? 'unknown'}`,
        detail: [
          groupLabel,
          originalTaskType,
          loweredTaskType ? `lower 为 ${loweredTaskType}` : undefined,
          executionMode,
          parallelSafe === undefined ? undefined : parallelSafe ? '允许白名单并行' : '仅串行执行',
        ].filter(Boolean).join(' · '),
        preview: resultKind ?? undefined,
        raw: view,
        source,
      };
    }
    case 'orchestration_subtask_completed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: `子任务完成：${subtaskName ?? 'unknown'}`,
        detail: [
          originalTaskType,
          loweredTaskType ? `结果来自 ${loweredTaskType}` : undefined,
          resultKind,
          executionMode,
        ].filter(Boolean).join(' · ') || '编排子任务执行完成',
        preview: responsePreview ?? getString(view.result_preview),
        raw: view,
        source,
      };
    case 'orchestration_subtask_failed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'failed',
        event,
        timestamp,
        title: `子任务失败：${subtaskName ?? 'unknown'}`,
        detail: [
          originalTaskType,
          loweredTaskType,
          executionMode,
          resultKind,
        ].filter(Boolean).join(' · ') || '编排子任务执行失败',
        preview: error ?? getString(view.result_preview),
        raw: view,
        source,
      };
    case 'worker_api_call_streaming_started':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'api_start',
        event,
        timestamp,
        title: 'worker 已切换到流式上游调用',
        detail: [subtaskName, taskType, phase].filter(Boolean).join(' · ') || '已进入流式调用阶段',
        preview: opencodeSessionId,
        raw: view,
        source,
      };
    case 'sdk_stream_message_started':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: '上游开始返回消息',
        detail: [subtaskName, taskType, phase].filter(Boolean).join(' · ') || '已收到流式消息开头',
        preview: opencodeSessionId,
        raw: view,
        source,
      };
    case 'sdk_stream_content_block_started':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: contentBlockKind === 'tool_use' ? '上游开始返回工具调用块' : '上游开始返回内容块',
        detail: [subtaskName, contentBlockKind, toolName, phase].filter(Boolean).join(' · ') || '流式内容块开始',
        preview: accumulatedTextPreview ?? partialJson ?? opencodeSessionId,
        raw: view,
        source,
      };
    case 'sdk_stream_content_block_delta':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: deltaKind === 'input_json_delta' ? '正在接收流式工具输入' : '正在接收流式文本',
        detail: [subtaskName, deltaKind, toolName, phase].filter(Boolean).join(' · ') || '流式内容持续更新中',
        preview: textDelta ?? partialJson ?? accumulatedTextPreview,
        raw: view,
        source,
      };
    case 'sdk_stream_message_delta':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: '流式消息状态更新',
        detail: [subtaskName, stopReason, phase].filter(Boolean).join(' · ') || '消息增量状态已刷新',
        preview: accumulatedTextPreview ?? opencodeSessionId,
        raw: view,
        source,
      };
    case 'sdk_stream_content_block_stopped':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: '流式内容块接收完成',
        detail: [subtaskName, contentBlockKind, toolName, phase].filter(Boolean).join(' · ') || '当前流式块已结束',
        preview: accumulatedTextPreview ?? partialJson,
        raw: view,
        source,
      };
    case 'sdk_stream_message_stopped':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: '上游流式消息结束',
        detail: [subtaskName, stopReason, phase].filter(Boolean).join(' · ') || '流式消息已结束',
        preview: accumulatedTextPreview ?? responsePreview,
        raw: view,
        source,
      };
    case 'worker_api_call_streaming_finished':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'upstream_response',
        event,
        timestamp,
        title: '流式响应接收完成',
        detail: [subtaskName, taskType, phase].filter(Boolean).join(' · ') || 'worker 已完成流式响应接收',
        preview: opencodeSessionId,
        raw: view,
        source,
      };
    case 'worker_api_call_streaming_failed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'failed',
        event,
        timestamp,
        title: '流式响应接收失败',
        detail: [subtaskName, taskType, phase].filter(Boolean).join(' · ') || 'worker 在流式接收阶段失败',
        preview: error ?? reason,
        raw: view,
        source,
      };
    case 'tool_invocation_detected':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'tool_detected',
        event,
        timestamp,
        title: `检测到工具调用：${toolName ?? 'unknown'}`,
        detail: [toolUseId, invocationId, permissionState, phase].filter(Boolean).join(' · ') || '主 agent 已识别到工具调用',
        preview: inputPreview,
        raw: view,
        source,
      };
    case 'tool_permission_requested':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'tool_permission',
        event,
        timestamp,
        title: `等待工具权限确认：${toolName ?? 'unknown'}`,
        detail: [toolUseId, invocationId, permissionState ?? 'requested', phase].filter(Boolean).join(' · ') || '工具调用需要用户确认',
        preview: prompt ?? inputPreview,
        raw: view,
        source,
      };
    case 'tool_permission_resolved':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'tool_permission',
        event,
        timestamp,
        title: `工具权限已确认：${toolName ?? 'unknown'}`,
        detail: [decision ? `decision=${decision}` : undefined, toolUseId, permissionState, phase].filter(Boolean).join(' · ') || '工具权限状态已更新',
        preview: errorSummary ?? resultSummary ?? inputPreview,
        raw: view,
        source,
      };
    case 'tool_execution_started':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'tool_execution',
        event,
        timestamp,
        title: `开始执行工具：${toolName ?? 'unknown'}`,
        detail: [toolUseId, invocationId, permissionState, phase].filter(Boolean).join(' · ') || '工具开始执行',
        preview: inputPreview,
        raw: view,
        source,
      };
    case 'tool_execution_finished':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'tool_execution',
        event,
        timestamp,
        title: `工具执行完成：${toolName ?? 'unknown'}`,
        detail: [toolUseId, invocationId, permissionState, phase].filter(Boolean).join(' · ') || '工具执行已完成',
        preview: resultSummary,
        raw: view,
        source,
      };
    case 'tool_execution_failed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'failed',
        event,
        timestamp,
        title: `工具执行失败：${toolName ?? 'unknown'}`,
        detail: [toolUseId, invocationId, permissionState, phase].filter(Boolean).join(' · ') || '工具执行失败',
        preview: errorSummary ?? error,
        raw: view,
        source,
      };
    case 'tool_result_recorded':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'tool_result',
        event,
        timestamp,
        title: `工具结果已记录：${toolName ?? 'unknown'}`,
        detail: [toolUseId, invocationId, permissionState, phase].filter(Boolean).join(' · ') || '工具结果已写回消息链路',
        preview: resultSummary ?? errorSummary,
        raw: view,
        source,
      };
    case 'message_completed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'completed',
        event,
        timestamp,
        title: '任务完成',
        detail: getString(view.status) ?? '执行成功结束',
        preview: responseText ?? responsePreview,
        raw: view,
        source,
      };
    case 'message_failed':
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'failed',
        event,
        timestamp,
        title: '任务失败',
        detail: errorCode ?? '执行失败',
        preview: error,
        raw: view,
        source,
      };
    default:
      return {
        id: `${source}-${event}-${timestamp}-${index}`,
        kind: 'other',
        event,
        timestamp,
        title: event,
        detail: '运行时事件',
        preview: responsePreview ?? responseText ?? taskPreview ?? requestPreview,
        raw: view,
        source,
      };
  }
}

export function deriveProcessEntriesFromEvents(events: JobView['recent_events'], source: ProcessSource = 'job'): ProcessEntry[] {
  return events
    .map((event, index) => createEntry(source, event.event, event.recorded_at_ms, event.payload, index))
    .sort((left, right) => left.timestamp - right.timestamp);
}

function createProjectionEntry(
  source: ProcessSource,
  kind: ProcessEntry['kind'],
  event: string,
  timestamp: number,
  title: string,
  detail: string,
  preview?: string,
  raw?: unknown,
): ProcessEntry {
  return {
    id: `${source}-${event}-${timestamp}-projection`,
    kind,
    event,
    timestamp,
    title,
    detail,
    preview,
    raw,
    source,
  };
}

export function deriveProcessEntriesFromJob(job: JobView, source: ProcessSource = 'job'): ProcessEntry[] {
  const entries = deriveProcessEntriesFromEvents(job.recent_events, source);
  const hasKind = (kind: ProcessEntry['kind']) => entries.some((entry) => entry.kind === kind);
  const pushProjectionEntry = (entry: ProcessEntry) => {
    if (!entries.some((candidate) => candidate.kind === entry.kind && candidate.title === entry.title && candidate.timestamp === entry.timestamp)) {
      entries.push(entry);
    }
  };

  if (job.orchestration && job.orchestration.total_subtasks > 0 && !hasKind('assignment')) {
    pushProjectionEntry(createProjectionEntry(
      source,
      'assignment',
      'projection_orchestration_state',
      job.updated_at_ms,
      '当前存在 orchestration 计划',
      `总子任务 ${job.orchestration.total_subtasks} · 运行中 ${job.orchestration.active_subtasks} · 已完成 ${job.orchestration.completed_subtasks} · 失败 ${job.orchestration.failed_subtasks}`,
      formatActiveSubtask(job.orchestration.active_subtask_name, job.orchestration.active_subtask_type) ?? job.orchestration.plan_id ?? undefined,
      job.orchestration,
    ));
  }

  if (job.pending_question && !hasKind('question_asked')) {
    const rawQuestion = job.pending_question.raw_question;
    const toolPermissionQuestion = rawQuestion && typeof rawQuestion === 'object' && rawQuestion !== null && 'type' in rawQuestion && rawQuestion.type === 'tool_permission'
      ? rawQuestion as { tool_name?: string; tool_use_id?: string; input_preview?: string; prompt?: string }
      : null;
    pushProjectionEntry(createProjectionEntry(
      source,
      toolPermissionQuestion ? 'tool_permission' : 'question_asked',
      'projection_pending_question',
      job.updated_at_ms,
      toolPermissionQuestion ? `等待工具权限确认：${toolPermissionQuestion.tool_name ?? 'unknown'}` : '存在待回答问题',
      toolPermissionQuestion
        ? [toolPermissionQuestion.tool_use_id, job.pending_question.risk_level, job.pending_question.decision_mode].filter(Boolean).join(' · ') || '工具调用需要用户确认'
        : [job.pending_question.question_kind, job.pending_question.decision_mode, job.pending_question.risk_level].filter(Boolean).join(' · ') || '等待用户补充信息',
      toolPermissionQuestion?.input_preview ?? toolPermissionQuestion?.prompt ?? job.pending_question.prompt,
      job.pending_question,
    ));
  }

  if (job.tool_state) {
    const toolState = job.tool_state;
    const kind = toolState.phase === 'result_recorded'
      ? 'tool_result'
      : toolState.phase === 'execution_failed' || toolState.phase === 'failed'
        ? 'failed'
        : toolState.awaits_user_confirmation
          ? 'tool_permission'
          : 'tool_execution';
    if (!hasKind(kind)) {
      pushProjectionEntry(createProjectionEntry(
        source,
        kind,
        'projection_tool_state',
        job.updated_at_ms,
        kind === 'tool_result'
          ? `工具结果已记录：${toolState.tool_name ?? 'unknown'}`
          : kind === 'failed'
            ? `工具执行失败：${toolState.tool_name ?? 'unknown'}`
            : kind === 'tool_permission'
              ? `等待工具权限确认：${toolState.tool_name ?? 'unknown'}`
              : `工具执行中：${toolState.tool_name ?? 'unknown'}`,
        [toolState.tool_use_id, toolState.invocation_id, toolState.permission_state, toolState.phase].filter(Boolean).join(' · ') || '运行时工具状态',
        toolState.result_preview ?? toolState.error ?? toolState.input_preview ?? undefined,
        toolState,
      ));
    }
  }

  if (job.failure && !hasKind('failed')) {
    pushProjectionEntry(createProjectionEntry(
      source,
      'failed',
      'projection_failure',
      job.failure.occurred_at_ms,
      '任务失败',
      [job.failure.error_stage, job.failure.error_code].filter(Boolean).join(' / ') || '执行失败',
      job.failure.internal_message,
      job.failure,
    ));
  }

  if (job.status === 'completed' && !hasKind('completed')) {
    pushProjectionEntry(createProjectionEntry(
      source,
      'completed',
      'projection_completed',
      job.updated_at_ms,
      '任务完成',
      job.execution_summary?.status ?? job.status,
      undefined,
      job.execution_summary ?? { status: job.status },
    ));
  }

  return entries.sort((left, right) => left.timestamp - right.timestamp);
}

export function deriveProcessEntriesFromRuntimeTimeline(events: TimelineEvent[], source: ProcessSource = 'runtime'): ProcessEntry[] {
  return events
    .map((event, index) => createEntry(source, event.event, event.recorded_at_ms, event.payload, index))
    .sort((left, right) => left.timestamp - right.timestamp);
}

function getLatestEntry(entries: ProcessEntry[], kinds: ProcessEntry['kind'][]): ProcessEntry | undefined {
  return entries
    .slice()
    .reverse()
    .find((entry) => kinds.includes(entry.kind));
}

export function deriveAgentDecisionSummary(entries: ProcessEntry[]): AgentDecisionSummary | null {
  if (entries.length === 0) {
    return null;
  }

  const latestQuestion = getLatestEntry(entries, ['question_asked']);
  const latestFollowup = getLatestEntry(entries, ['followup']);
  const latestFailure = getLatestEntry(entries, ['failed']);
  const latestCompletion = getLatestEntry(entries, ['completed']);
  const latestAssignment = getLatestEntry(entries, ['assignment']);
  const latestApiStart = getLatestEntry(entries, ['api_start']);

  if (latestQuestion && (!latestCompletion || latestQuestion.timestamp >= latestCompletion.timestamp)) {
    return {
      title: '先澄清需求再继续实现',
      reason: latestQuestion.preview ?? latestQuestion.detail,
      nextAction: '等待用户补充信息或回答结构化问题。',
      timestamp: latestQuestion.timestamp,
    };
  }

  if (latestFollowup && (!latestCompletion || latestFollowup.timestamp >= latestCompletion.timestamp)) {
    return {
      title: '继续自动推进后续处理',
      reason: latestFollowup.detail,
      nextAction: '系统会自动发起 follow-up，本轮暂时不需要用户介入。',
      timestamp: latestFollowup.timestamp,
    };
  }

  if (latestFailure) {
    return {
      title: '停止执行并等待排查',
      reason: latestFailure.preview ?? latestFailure.detail,
      nextAction: '查看失败阶段与错误信息，确认是否需要人工介入。',
      timestamp: latestFailure.timestamp,
    };
  }

  if (latestCompletion) {
    return {
      title: '当前轮次结果已可提交反馈',
      reason: latestCompletion.preview ?? latestCompletion.detail,
      nextAction: '可以接受/拒绝结果，或基于当前输出继续提出下一步需求。',
      timestamp: latestCompletion.timestamp,
    };
  }

  if (latestAssignment || latestApiStart) {
    const latestExecution = [latestApiStart, latestAssignment]
      .filter((entry): entry is ProcessEntry => Boolean(entry))
      .sort((left, right) => right.timestamp - left.timestamp)[0];

    return {
      title: '开始执行当前计划',
      reason: latestExecution.preview ?? latestExecution.detail,
      nextAction: '等待 worker 完成上游调用并返回结果。',
      timestamp: latestExecution.timestamp,
    };
  }

  const latestEntry = entries[entries.length - 1];
  return {
    title: '根据最近事件继续推进',
    reason: latestEntry.preview ?? latestEntry.detail,
    nextAction: '可结合详细过程时间线继续观察后续进展。',
    timestamp: latestEntry.timestamp,
  };
}
