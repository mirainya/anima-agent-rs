import { describe, expect, it } from 'vitest';
import { deriveProcessEntriesFromEvents, deriveProcessEntriesFromRuntimeTimeline } from './deriveProcessEntries';

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
});
