import { describe, expect, it } from 'vitest';
import { deriveJobSummary } from './deriveJobSummary';
import type { JobView } from '@/shared/utils/types';

function makeJob(partial: Partial<JobView>): JobView {
  return {
    job_id: 'job-1',
    trace_id: 'trace-1',
    message_id: 'msg-1',
    kind: 'main',
    parent_job_id: null,
    channel: 'web',
    chat_id: 'chat-1',
    sender_id: 'user-1',
    user_content: 'hello',
    status: 'executing',
    status_label: 'executing',
    accepted: true,
    started_at_ms: 1,
    updated_at_ms: 2,
    elapsed_ms: 1,
    current_step: '正在处理',
    pending_question: null,
    recent_events: [],
    worker: null,
    execution_summary: null,
    failure: null,
    review: null,
    tool_state: null,
    ...partial,
  };
}

describe('deriveJobSummary', () => {
  it('derives waiting_user_input with pending question', () => {
    const summary = deriveJobSummary(makeJob({
      status: 'waiting_user_input',
      pending_question: {
        question_id: 'q-1',
        question_kind: 'input',
        prompt: '请补充部署环境',
        options: [],
        raw_question: {},
        decision_mode: 'user_required',
        risk_level: 'high',
        requires_user_confirmation: true,
        answer_summary: null,
        opencode_session_id: null,
        resolution_source: null,
      },
    }));

    expect(summary.needsUserAction).toBe(true);
    expect(summary.headline).toContain('请补充部署环境');
  });

  it('derives tool permission waiting summary', () => {
    const summary = deriveJobSummary(makeJob({
      status: 'waiting_user_input',
      current_step: '等待用户确认工具调用权限',
      pending_question: {
        question_id: 'q-tool',
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
        answer_summary: null,
        opencode_session_id: null,
        resolution_source: null,
      },
    }));

    expect(summary.needsUserAction).toBe(true);
    expect(summary.headline).toContain('等待确认工具权限');
    expect(summary.detail).toContain('toolu_123');
  });

  it('derives tool execution summary from tool_state', () => {
    const summary = deriveJobSummary(makeJob({
      status: 'executing',
      current_step: '工具结果已记录，等待后续推进',
      tool_state: {
        invocation_id: 'invoke-1',
        tool_name: 'read_file',
        tool_use_id: 'toolu_456',
        phase: 'result_recorded',
        permission_state: 'allowed',
        input_preview: '/tmp/demo.txt',
        result_preview: 'line 1\nline 2',
        error: null,
        awaits_user_confirmation: false,
      },
    }));

    expect(summary.headline).toContain('工具结果已记录');
    expect(summary.detail).toContain('line 1');
  });

  it('derives executing follow-up as auto continuing', () => {
    const summary = deriveJobSummary(makeJob({
      status: 'executing',
      recent_events: [{ event: 'requirement_followup_scheduled', recorded_at_ms: 2, payload: {} }],
    }));

    expect(summary.isAutoContinuing).toBe(true);
    expect(summary.needsUserAction).toBe(false);
  });

  it('derives stalled without question', () => {
    const summary = deriveJobSummary(makeJob({ status: 'stalled' }));

    expect(summary.isStalledWithoutQuestion).toBe(true);
    expect(summary.detail).toContain('暂时缺少新的运行时进展');
  });

  it('keeps completed semantics with accepted review', () => {
    const summary = deriveJobSummary(makeJob({
      status: 'completed',
      review: { verdict: 'accepted', reviewed_at_ms: 1, note: null, reason: null },
    }));

    expect(summary.statusLabelText).toBe('已完成');
    expect(summary.headline).toBe('结果已被接受');
  });

  it('derives failed from failure snapshot', () => {
    const summary = deriveJobSummary(makeJob({
      status: 'failed',
      failure: { error_code: 'worker_timeout', error_stage: 'execute' },
    }));

    expect(summary.statusTone).toBe('failed');
    expect(summary.detail).toContain('worker_timeout');
  });

  it('extracts latest response text from message_completed payload', () => {
    const summary = deriveJobSummary(makeJob({
      recent_events: [
        { event: 'message_completed', recorded_at_ms: 2, payload: { response_preview: 'preview text' } },
      ],
    }));

    expect(summary.hasResponse).toBe(true);
    expect(summary.latestResponseText).toBe('preview text');
  });
});
