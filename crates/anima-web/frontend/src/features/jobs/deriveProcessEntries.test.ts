import { describe, expect, it } from 'vitest';
import { deriveProcessEntriesFromEvents, deriveProcessEntriesFromJob, deriveProcessEntriesFromRuntimeTimeline } from './deriveProcessEntries';

describe('deriveProcessEntries', () => {
  it('sorts and maps detailed job events into readable process entries', () => {
    const entries = deriveProcessEntriesFromEvents([
      {
        event: 'upstream_response_observed',
        recorded_at_ms: 30,
        payload: {
          worker_id: 'worker-1',
          task_type: 'api-call',
          provider: 'opencode',
          operation: 'send_prompt',
          response_preview: '收到回复',
          raw_result: { content: '收到回复' },
        },
      },
      {
        event: 'worker_task_assigned',
        recorded_at_ms: 10,
        payload: {
          task_type: 'api-call',
          task_summary: '主 agent 已将任务派发给 worker',
          task_preview: '检查发布状态',
        },
      },
      {
        event: 'api_call_started',
        recorded_at_ms: 20,
        payload: {
          task_type: 'api-call',
          request_preview: '检查发布状态',
        },
      },
    ]);

    expect(entries.map((entry) => entry.kind)).toEqual(['assignment', 'api_start', 'upstream_response']);
    expect(entries[0].title).toContain('派发任务');
    expect(entries[0].preview).toBe('检查发布状态');
    expect(entries[2].detail).toContain('worker-1');
    expect(entries[2].raw).toEqual({ content: '收到回复' });
  });

  it('falls back gracefully for legacy upstream payloads and question chain events', () => {
    const runtimeEntries = deriveProcessEntriesFromRuntimeTimeline([
      {
        event: 'question_asked',
        trace_id: 'trace-1',
        message_id: 'job-1',
        channel: 'web',
        chat_id: 'chat-1',
        sender_id: 'user-1',
        recorded_at_ms: 10,
        payload: {
          question_id: 'q-1',
          prompt: '请选择环境',
          raw_question: { prompt: '请选择环境' },
        },
      },
      {
        event: 'question_answer_submitted',
        trace_id: 'trace-1',
        message_id: 'job-1',
        channel: 'web',
        chat_id: 'chat-1',
        sender_id: 'user-1',
        recorded_at_ms: 20,
        payload: {
          question_id: 'q-1',
          answer_summary: 'staging',
        },
      },
      {
        event: 'upstream_response_observed',
        trace_id: 'trace-1',
        message_id: 'job-1',
        channel: 'web',
        chat_id: 'chat-1',
        sender_id: 'user-1',
        recorded_at_ms: 30,
        payload: {
          opencode_session_id: 'legacy-session',
        },
      },
    ]);

    expect(runtimeEntries.map((entry) => entry.kind)).toEqual(['question_asked', 'question_answer', 'upstream_response']);
    expect(runtimeEntries[0].preview).toBe('请选择环境');
    expect(runtimeEntries[1].preview).toBe('staging');
    expect(runtimeEntries[2].preview).toBe('legacy-session');
    expect(runtimeEntries[2].title).toContain('已收到上游响应');
  });

  it('maps tool lifecycle events into dedicated process entries', () => {
    const entries = deriveProcessEntriesFromRuntimeTimeline([
      {
        event: 'tool_invocation_detected',
        trace_id: 'trace-tool',
        message_id: 'job-tool',
        channel: 'web',
        chat_id: 'chat-tool',
        sender_id: 'user-1',
        recorded_at_ms: 10,
        payload: {
          invocation_id: 'invoke-1',
          tool_name: 'bash_exec',
          tool_use_id: 'toolu_123',
          permission_state: 'requested',
          phase: 'detected',
          details: { tool_input: { command: 'ls' } },
        },
      },
      {
        event: 'tool_permission_requested',
        trace_id: 'trace-tool',
        message_id: 'job-tool',
        channel: 'web',
        chat_id: 'chat-tool',
        sender_id: 'user-1',
        recorded_at_ms: 20,
        payload: {
          invocation_id: 'invoke-1',
          tool_name: 'bash_exec',
          tool_use_id: 'toolu_123',
          permission_state: 'requested',
          phase: 'permission_requested',
          prompt: 'allow bash_exec?',
        },
      },
      {
        event: 'tool_execution_started',
        trace_id: 'trace-tool',
        message_id: 'job-tool',
        channel: 'web',
        chat_id: 'chat-tool',
        sender_id: 'user-1',
        recorded_at_ms: 30,
        payload: {
          invocation_id: 'invoke-1',
          tool_name: 'bash_exec',
          tool_use_id: 'toolu_123',
          permission_state: 'allowed',
          phase: 'executing',
          details: { tool_input: { command: 'ls' } },
        },
      },
      {
        event: 'tool_result_recorded',
        trace_id: 'trace-tool',
        message_id: 'job-tool',
        channel: 'web',
        chat_id: 'chat-tool',
        sender_id: 'user-1',
        recorded_at_ms: 40,
        payload: {
          invocation_id: 'invoke-1',
          tool_name: 'bash_exec',
          tool_use_id: 'toolu_123',
          permission_state: 'allowed',
          phase: 'result_recorded',
          result_summary: 'file-a\nfile-b',
        },
      },
    ]);

    expect(entries.map((entry) => entry.kind)).toEqual(['tool_detected', 'tool_permission', 'tool_execution', 'tool_result']);
    expect(entries[0].title).toContain('检测到工具调用');
    expect(entries[0].preview).toContain('command');
    expect(entries[1].title).toContain('等待工具权限确认');
    expect(entries[2].title).toContain('开始执行工具');
    expect(entries[3].preview).toBe('file-a\nfile-b');
  });

  it('renders orchestration lowering and whitelist parallel details', () => {
    const runtimeEntries = deriveProcessEntriesFromRuntimeTimeline([
      {
        event: 'orchestration_subtask_started',
        trace_id: 'trace-2',
        message_id: 'job-2',
        channel: 'web',
        chat_id: 'chat-2',
        sender_id: 'user-2',
        recorded_at_ms: 10,
        payload: {
          subtask_name: 'generate-report',
          original_task_type: 'reporting',
          lowered_task_type: 'transform',
          execution_mode: 'whitelist_parallel',
          parallel_safe: true,
          parallel_group_index: 1,
          parallel_group_size: 2,
          result_kind: 'transform',
        },
      },
      {
        event: 'orchestration_subtask_completed',
        trace_id: 'trace-2',
        message_id: 'job-2',
        channel: 'web',
        chat_id: 'chat-2',
        sender_id: 'user-2',
        recorded_at_ms: 20,
        payload: {
          subtask_name: 'generate-report',
          original_task_type: 'reporting',
          lowered_task_type: 'transform',
          execution_mode: 'whitelist_parallel',
          result_kind: 'transform',
          result_preview: '整理好的报告摘要',
        },
      },
      {
        event: 'orchestration_subtask_failed',
        trace_id: 'trace-2',
        message_id: 'job-2',
        channel: 'web',
        chat_id: 'chat-2',
        sender_id: 'user-2',
        recorded_at_ms: 30,
        payload: {
          subtask_name: 'analyze-data',
          original_task_type: 'analysis',
          lowered_task_type: 'api-call',
          execution_mode: 'serial',
          result_kind: 'upstream',
          error: 'upstream exploded',
        },
      },
    ]);

    expect(runtimeEntries.map((entry) => entry.kind)).toEqual(['assignment', 'upstream_response', 'failed']);
    expect(runtimeEntries[0].title).toContain('generate-report');
    expect(runtimeEntries[0].detail).toContain('whitelist_parallel');
    expect(runtimeEntries[0].detail).toContain('允许白名单并行');
    expect(runtimeEntries[1].preview).toBe('整理好的报告摘要');
    expect(runtimeEntries[2].title).toContain('analyze-data');
    expect(runtimeEntries[2].preview).toBe('upstream exploded');
  });

  it('derives projection-first entries from job state when recent events are missing', () => {
    const entries = deriveProcessEntriesFromJob({
      job_id: 'job-1',
      trace_id: 'trace-1',
      message_id: 'job-1',
      kind: 'main',
      parent_job_id: null,
      channel: 'web',
      chat_id: 'chat-1',
      sender_id: 'user-1',
      user_content: 'hello',
      status: 'waiting_user_input',
      status_label: 'waiting_user_input',
      accepted: true,
      started_at_ms: 1,
      updated_at_ms: 200,
      elapsed_ms: 199,
      current_step: '等待权限确认',
      recent_events: [],
      worker: null,
      review: null,
      pending_question: {
        question_id: 'q-tool-1',
        question_kind: 'confirm',
        prompt: '允许工具 bash_exec 使用当前参数执行吗？',
        options: ['allow', 'deny'],
        raw_question: {
          type: 'tool_permission',
          tool_name: 'bash_exec',
          tool_use_id: 'toolu_123',
          input_preview: '{"command":"ls"}',
        },
        decision_mode: 'user_required',
        risk_level: 'high',
        requires_user_confirmation: true,
        opencode_session_id: 'sess-1',
        answer_summary: null,
        resolution_source: null,
      },
      tool_state: {
        invocation_id: 'invoke-1',
        tool_name: 'bash_exec',
        tool_use_id: 'toolu_123',
        phase: 'permission_requested',
        invocation_status: 'pending',
        status_text: 'awaiting permission',
        permission_state: 'requested',
        input_preview: '{"command":"ls"}',
        result_preview: null,
        error: null,
        awaits_user_confirmation: true,
      },
      execution_summary: {
        plan_type: 'single',
        status: 'waiting_user_input',
        cache_hit: false,
        worker_id: null,
        error_code: null,
        error_stage: null,
        task_duration_ms: 199,
        stages: {},
      },
      failure: null,
      orchestration: {
        plan_id: 'plan-1',
        active_subtask_name: 'run-check',
        active_subtask_type: 'testing',
        active_subtask_id: 'sub-1',
        total_subtasks: 2,
        active_subtasks: 1,
        completed_subtasks: 0,
        failed_subtasks: 0,
        child_job_ids: [],
      },
    });

    expect(entries.some((entry) => entry.title.includes('等待工具权限确认：bash_exec'))).toBe(true);
    expect(entries.some((entry) => entry.title.includes('当前存在 orchestration 计划'))).toBe(true);
    expect(entries.some((entry) => entry.preview === 'run-check（testing）')).toBe(true);
  });
});
