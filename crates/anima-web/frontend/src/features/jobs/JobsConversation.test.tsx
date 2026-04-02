import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { JobsConversation } from './JobsConversation';
import type { JobView, StatusSnapshot } from '@/shared/utils/types';

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
    user_content: '请帮我检查部署状态',
    status: 'executing',
    status_label: 'executing',
    accepted: true,
    started_at_ms: 1710000000000,
    updated_at_ms: 1710000005000,
    elapsed_ms: 5000,
    current_step: '正在执行检查',
    pending_question: null,
    recent_events: [],
    worker: null,
    execution_summary: null,
    failure: null,
    review: null,
    ...partial,
  };
}

const selectedSession: StatusSnapshot['recent_sessions'][number] = {
  chat_id: 'chat-1',
  channel: 'web',
  session_id: 'sess-1',
  history_len: 3,
  last_user_message_preview: '请帮我检查部署状态',
};

describe('JobsConversation', () => {
  it('shows hierarchy, worker info, latest output and key process summary', () => {
    render(
      <JobsConversation
        jobs={[
          makeJob({
            kind: 'subtask',
            parent_job_id: 'job-main',
            worker: {
              worker_id: 'worker-7',
              status: 'busy',
              task_id: 'task-1',
              trace_id: 'trace-1',
              task_type: 'api-call',
              elapsed_ms: 3000,
              content_preview: 'running test',
            },
            recent_events: [
              { event: 'worker_task_assigned', recorded_at_ms: 1710000001000, payload: { task_type: 'api-call', task_summary: '主 agent 已将任务派发给 worker', task_preview: '检查部署状态' } },
              { event: 'api_call_started', recorded_at_ms: 1710000002000, payload: { task_type: 'api-call', request_preview: '检查部署状态' } },
              { event: 'upstream_response_observed', recorded_at_ms: 1710000003000, payload: { worker_id: 'worker-7', task_type: 'api-call', response_preview: '检查完成，服务可访问。' } },
              { event: 'message_completed', recorded_at_ms: 1710000004000, payload: { response_text: '检查完成，服务可访问。' } },
            ],
          }),
        ]}
        selectedJob={makeJob({ trace_id: 'trace-1' })}
        selectedSession={selectedSession}
        selectedSessionId="chat-1"
        scopeSummary="当前会话"
        runtimeTimeline={[]}
        onOpenJobDetails={vi.fn()}
      />,
    );

    expect(screen.getByText('协作痕迹')).toBeTruthy();
    expect(screen.getByText(/子任务 · 属于/)).toBeTruthy();
    expect(screen.getByText('worker-7')).toBeTruthy();
    expect(screen.getAllByText('检查完成，服务可访问。').length).toBeGreaterThan(0);
    expect(screen.getByText('关键过程摘要')).toBeTruthy();
    expect(screen.getByText('主 agent 已派发任务给 worker')).toBeTruthy();
    expect(screen.getByText('worker 开始调用上游')).toBeTruthy();
    expect(screen.getByText('已收到上游响应')).toBeTruthy();
  });

  it('shows orchestration overview for main job conversation', () => {
    render(
      <JobsConversation
        jobs={[
          makeJob({
            orchestration: {
              plan_id: 'plan-1',
              active_subtask_name: 'implement-api',
              active_subtask_id: 'sub-2',
              total_subtasks: 3,
              active_subtasks: 1,
              completed_subtasks: 1,
              failed_subtasks: 0,
              child_job_ids: ['job-sub-1', 'job-sub-2'],
            },
          }),
        ]}
        selectedJob={makeJob({ job_id: 'job-1', trace_id: 'trace-1', orchestration: {
          plan_id: 'plan-1',
          active_subtask_name: 'implement-api',
          active_subtask_id: 'sub-2',
          total_subtasks: 3,
          active_subtasks: 1,
          completed_subtasks: 1,
          failed_subtasks: 0,
          child_job_ids: ['job-sub-1', 'job-sub-2'],
        } })}
        selectedSession={selectedSession}
        selectedSessionId="chat-1"
        scopeSummary="当前会话"
        runtimeTimeline={[]}
        onOpenJobDetails={vi.fn()}
      />,
    );

    expect(screen.getByText('Orchestration 概览')).toBeTruthy();
    expect(screen.getByText(/子任务 1\/3/)).toBeTruthy();
    expect(screen.getByText(/当前 implement-api/)).toBeTruthy();
  });

  it('shows main agent decision summary for clarification flows', () => {
    render(
      <JobsConversation
        jobs={[
          makeJob({
            recent_events: [
              {
                event: 'question_asked',
                recorded_at_ms: 1710000001000,
                payload: {
                  question_id: 'q-1',
                  prompt: '请选择前端技术栈',
                  raw_question: { prompt: '请选择前端技术栈' },
                },
              },
            ],
          }),
        ]}
        selectedJob={makeJob({ trace_id: 'trace-1' })}
        selectedSession={selectedSession}
        selectedSessionId="chat-1"
        scopeSummary="当前会话"
        runtimeTimeline={[]}
        onOpenJobDetails={vi.fn()}
      />,
    );

    expect(screen.getByText('主 agent 决策')).toBeTruthy();
    expect(screen.getByText('先澄清需求再继续实现')).toBeTruthy();
    expect(screen.getByText(/原因：请选择前端技术栈/)).toBeTruthy();
    expect(screen.getByText(/下一步：等待用户补充信息或回答结构化问题/)).toBeTruthy();
  });

  it('shows pending question summary without rendering answer form', () => {
    render(
      <JobsConversation
        jobs={[
          makeJob({
            status: 'waiting_user_input',
            pending_question: {
              question_id: 'q-1',
              question_kind: 'input',
              prompt: '请确认部署环境',
              options: [],
              raw_question: {},
              decision_mode: 'user_required',
              risk_level: 'high',
              requires_user_confirmation: true,
              opencode_session_id: null,
              answer_summary: 'staging',
              resolution_source: 'user',
            },
          }),
        ]}
        selectedJob={makeJob({ trace_id: 'trace-1' })}
        selectedSession={selectedSession}
        selectedSessionId="chat-1"
        scopeSummary="当前会话"
        runtimeTimeline={[]}
        onOpenJobDetails={vi.fn()}
      />,
    );

    expect(screen.getByText('待处理问题')).toBeTruthy();
    expect(screen.getAllByText('请确认部署环境').length).toBeGreaterThan(0);
    expect(screen.getByText(/最近回答：staging/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: '提交回答' })).toBeNull();
  });

  it('shows scoped runtime process timeline for selected job', () => {
    render(
      <JobsConversation
        jobs={[makeJob()]}
        selectedJob={makeJob({ job_id: 'job-1', trace_id: 'trace-1' })}
        selectedSession={selectedSession}
        selectedSessionId="chat-1"
        scopeSummary="当前任务"
        runtimeTimeline={[
          {
            event: 'worker_task_assigned',
            trace_id: 'trace-1',
            message_id: 'job-1',
            channel: 'web',
            chat_id: 'chat-1',
            sender_id: 'user-1',
            recorded_at_ms: 1710000005000,
            payload: { task_type: 'api-call', task_summary: '主 agent 已将任务派发给 worker', task_preview: 'runtime preview' },
          },
          {
            event: 'message_completed',
            trace_id: 'trace-1',
            message_id: 'job-1',
            channel: 'web',
            chat_id: 'chat-1',
            sender_id: 'user-1',
            recorded_at_ms: 1710000006000,
            payload: { response_preview: 'runtime preview done' },
          },
          {
            event: 'message_completed',
            trace_id: 'trace-other',
            message_id: 'job-2',
            channel: 'web',
            chat_id: 'chat-2',
            sender_id: 'user-2',
            recorded_at_ms: 1710000007000,
            payload: { response_preview: 'other preview' },
          },
        ]}
        onOpenJobDetails={vi.fn()}
      />,
    );

    expect(screen.getByText('最近运行轨迹')).toBeTruthy();
    expect(screen.getByText('主 agent 已派发任务给 worker')).toBeTruthy();
    expect(screen.getByText('runtime preview')).toBeTruthy();
    expect(screen.getByText('任务完成')).toBeTruthy();
    expect(screen.queryByText('other preview')).toBeNull();
  });
});
