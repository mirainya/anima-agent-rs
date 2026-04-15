import { fireEvent, render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { JobDetail } from './JobDetail';
import { useUiStore } from '@/shared/state/useUiStore';
import type { JobView } from '@/shared/utils/types';

vi.mock('@/shared/api/review', () => ({
  useReviewJobMutation: () => ({ mutate: vi.fn(), isPending: false }),
}));

vi.mock('@/shared/api/questionAnswer', () => ({
  useQuestionAnswerMutation: () => ({ mutate: vi.fn(), isPending: false }),
}));

function makeJob(partial: Partial<JobView>): JobView {
  return {
    job_id: 'job-1',
    trace_id: 'trace-1',
    message_id: 'job-1',
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
    current_step: 'step',
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

function renderWithClient(ui: ReactElement) {
  const client = new QueryClient();
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

function renderJob(job: JobView) {
  useUiStore.setState({ selectedJobId: job.job_id });
  return renderWithClient(
    <JobDetail jobs={[job]} selectedSessionChatId={job.chat_id ?? null} />,
  );
}

describe('JobDetail status interactions', () => {
  beforeEach(() => {
    useUiStore.setState({ selectedJobId: null });
  });

  it('does not break hooks when jobs appear after initial empty render', () => {
    useUiStore.setState({ selectedJobId: 'job-1' });
    const job = makeJob({ job_id: 'job-1', chat_id: 'chat-1' });
    const view = renderWithClient(
      <JobDetail jobs={[]} selectedSessionChatId="chat-1" />,
    );

    expect(screen.getByText(/暂无可查看的 Job/)).toBeTruthy();

    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <JobDetail jobs={[job]} selectedSessionChatId="chat-1" />
      </QueryClientProvider>,
    );

    expect(screen.getByText(job.status)).toBeTruthy();
  });

  it('shows waiting panel and answer UI for unanswered waiting_user_input', () => {
    renderJob(makeJob({
      status: 'waiting_user_input',
      status_label: 'waiting_user_input',
      pending_question: {
        question_id: 'q-1',
        question_kind: 'input',
        prompt: '请补充部署环境',
        options: [],
        raw_question: { prompt: '请补充部署环境' },
        decision_mode: 'user_required',
        risk_level: 'high',
        requires_user_confirmation: true,
        opencode_session_id: 'sess-1',
        answer_summary: null,
        resolution_source: null,
      },
    }));

    expect(screen.getByText('待处理问题')).toBeTruthy();
    expect(screen.getAllByText('请补充部署环境').length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: '提交回答' })).toBeTruthy();
  });

  it('shows submitted answer summary for answered waiting_user_input', () => {
    renderJob(makeJob({
      status: 'waiting_user_input',
      pending_question: {
        question_id: 'q-1',
        question_kind: 'input',
        prompt: '请补充部署环境',
        options: [],
        raw_question: { prompt: '请补充部署环境' },
        decision_mode: 'user_required',
        risk_level: 'high',
        requires_user_confirmation: true,
        opencode_session_id: 'sess-1',
        answer_summary: 'production-us-east-1',
        resolution_source: 'user',
      },
    }));

    expect(screen.getAllByText(/最近已提交回答：production-us-east-1/).length).toBeGreaterThan(0);
  });

  it('renders tool permission question with tool details and action labels', () => {
    renderJob(makeJob({
      status: 'waiting_user_input',
      pending_question: {
        question_id: 'q-tool-1',
        question_kind: 'confirm',
        prompt: '允许工具 bash_exec 使用当前参数执行吗？',
        options: ['allow', 'deny'],
        raw_question: {
          type: 'tool_permission',
          tool_name: 'bash_exec',
          tool_use_id: 'toolu_123',
          tool_input: { command: 'rm test.txt' },
          input_preview: '{"command":"rm test.txt"}',
        },
        decision_mode: 'user_required',
        risk_level: 'high',
        requires_user_confirmation: true,
        opencode_session_id: 'sess-1',
        answer_summary: null,
        resolution_source: null,
      },
    }));

    expect(screen.getByText('bash_exec')).toBeTruthy();
    expect(screen.getByText('toolu_123')).toBeTruthy();
    expect(screen.getByText('允许执行')).toBeTruthy();
    expect(screen.getByText('拒绝执行')).toBeTruthy();
    expect(screen.getAllByText(/权限确认/).length).toBeGreaterThan(0);
  });

  it('shows read-only tool state summary outside waiting state', () => {
    renderJob(makeJob({
      status: 'executing',
      status_label: 'executing',
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

    expect(screen.getByText('工具执行状态')).toBeTruthy();
    expect(screen.getByText('read_file')).toBeTruthy();
    expect(screen.getByText('result_recorded')).toBeTruthy();
    expect(screen.getAllByText(/line 1/).length).toBeGreaterThan(0);
  });

  it('shows auto follow-up hint without answer UI while executing', () => {
    renderJob(makeJob({
      status: 'executing',
      status_label: 'executing',
      recent_events: [
        { event: 'requirement_followup_scheduled', recorded_at_ms: 2, payload: {} },
      ],
    }));

    expect(screen.getAllByText(/正在自动继续补充处理/).length).toBeGreaterThan(0);
    expect(screen.queryByText('待处理问题')).toBeNull();
    expect(screen.queryByRole('button', { name: '提交回答' })).toBeNull();
  });

  it('shows stalled hint without implying answer UI', () => {
    renderJob(makeJob({
      status: 'stalled',
      status_label: 'stalled',
    }));

    expect(screen.getAllByText(/暂时停滞/).length).toBeGreaterThan(0);
    expect(screen.queryByText('待处理问题')).toBeNull();
    expect(screen.queryByRole('button', { name: '提交回答' })).toBeNull();
  });

  it('shows review actions for completed job without review', () => {
    renderJob(makeJob({
      status: 'completed',
      status_label: 'completed',
    }));

    expect(screen.getByRole('button', { name: '接受结果' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '拒绝结果' })).toBeTruthy();
  });

  it('keeps completed semantics after accepted review', () => {
    renderJob(makeJob({
      status: 'completed',
      status_label: 'completed',
      review: {
        verdict: 'accepted',
        reviewed_at_ms: 3,
        reason: null,
        note: null,
      },
    }));

    expect(screen.getByText('已完成')).toBeTruthy();
    expect(screen.getAllByText('结果已被接受').length).toBeGreaterThan(0);
  });

  it('shows orchestration overview for main jobs', () => {
    renderJob(makeJob({
      status: 'executing',
      orchestration: {
        plan_id: 'plan-1',
        active_subtask_name: 'design-api',
        active_subtask_type: 'design',
        active_subtask_id: 'sub-1',
        total_subtasks: 3,
        active_subtasks: 1,
        completed_subtasks: 1,
        failed_subtasks: 0,
        child_job_ids: ['job-sub-1'],
      },
    }));

    expect(screen.getByText('Orchestration 概览')).toBeTruthy();
    expect(screen.getByText('design-api（design）')).toBeTruthy();
    expect(screen.getAllByText('3').length).toBeGreaterThan(0);
  });

  it('renders human-readable main agent decision summary', () => {
    renderJob(makeJob({
      status: 'executing',
      status_label: 'executing',
      recent_events: [
        {
          event: 'question_asked',
          recorded_at_ms: 10,
          payload: {
            question_id: 'q-1',
            prompt: '请选择前端技术栈',
            raw_question: { prompt: '请选择前端技术栈' },
          },
        },
      ],
    }));

    fireEvent.click(screen.getByRole('button', { name: '执行' }));
    expect(screen.getByText('主 agent 决策')).toBeTruthy();
    expect(screen.getByText('先澄清需求再继续实现')).toBeTruthy();
    expect(screen.getAllByText('请选择前端技术栈').length).toBeGreaterThan(0);
    expect(screen.getByText('等待用户补充信息或回答结构化问题。')).toBeTruthy();
  });

  it('renders detailed process timeline with raw payload expansion and fallbacks', () => {
    renderJob(makeJob({
      status: 'executing',
      status_label: 'executing',
      recent_events: [
        {
          event: 'worker_task_assigned',
          recorded_at_ms: 10,
          payload: {
            task_id: 'task-1',
            task_type: 'api-call',
            task_summary: '主 agent 已将任务派发给 worker',
            task_preview: '请检查部署状态',
          },
        },
        {
          event: 'api_call_started',
          recorded_at_ms: 20,
          payload: {
            task_id: 'task-1',
            task_type: 'api-call',
            request_preview: '请检查部署状态',
          },
        },
        {
          event: 'upstream_response_observed',
          recorded_at_ms: 30,
          payload: {
            worker_id: 'worker-1',
            task_type: 'api-call',
            provider: 'opencode',
            operation: 'send_prompt',
            response_preview: '已经拿到上游回复',
            raw_result: { content: '已经拿到上游回复' },
          },
        },
        {
          event: 'question_asked',
          recorded_at_ms: 40,
          payload: {
            question_id: 'q-1',
            prompt: '请选择部署环境',
            raw_question: { prompt: '请选择部署环境' },
          },
        },
        {
          event: 'question_answer_submitted',
          recorded_at_ms: 50,
          payload: {
            question_id: 'q-1',
            answer_summary: 'staging',
          },
        },
        {
          event: 'upstream_response_observed',
          recorded_at_ms: 60,
          payload: {
            opencode_session_id: 'legacy-session',
          },
        },
      ],
    }));

    fireEvent.click(screen.getByRole('button', { name: '执行' }));
    expect(screen.getByText('详细过程时间线')).toBeTruthy();
    expect(screen.getByText('主 agent 已派发任务给 worker')).toBeTruthy();
    expect(screen.getByText('worker 开始调用上游')).toBeTruthy();
    expect(screen.getAllByText('已收到上游响应').length).toBeGreaterThan(0);
    expect(screen.getByText('触发结构化问题')).toBeTruthy();
    expect(screen.getByText('已提交问题回答')).toBeTruthy();
    expect(screen.getAllByText('请检查部署状态').length).toBeGreaterThan(0);
    expect(screen.getByText('已经拿到上游回复')).toBeTruthy();
    expect(screen.getByText('legacy-session')).toBeTruthy();

    fireEvent.click(screen.getAllByText('已收到上游响应')[0]!);
    expect(screen.getAllByText(/content/).length).toBeGreaterThan(0);
  });
});
