import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DiagnosticsInspector } from './DiagnosticsInspector';
import { useUiStore } from '@/shared/state/useUiStore';

vi.mock('@/shared/api/jobs', () => ({
  useJobsQuery: () => ({
    data: [
      {
        job_id: 'job-1',
        trace_id: 'trace-1',
        message_id: 'job-1',
        kind: 'main',
        parent_job_id: null,
        channel: 'web',
        chat_id: 'chat-1',
        sender_id: 'web-user',
        user_content: 'hello',
        status: 'failed',
        status_label: 'failed',
        accepted: true,
        started_at_ms: 100,
        updated_at_ms: 200,
        elapsed_ms: 100,
        current_step: '执行失败，等待处理',
        pending_question: null,
        recent_events: [],
        worker: { worker_id: 'worker-1', status: 'busy', task_id: 'task-1', trace_id: 'trace-1', task_type: 'api-call', elapsed_ms: 500, content_preview: 'preview' },
        tool_state: null,
        execution_summary: {
          plan_type: 'single',
          status: 'failed',
          cache_hit: false,
          worker_id: 'worker-1',
          error_code: 'tool_permission_denied',
          error_stage: 'task_execution',
          task_duration_ms: 500,
          stages: { context_ms: 10, session_ms: 20, classify_ms: 30, execute_ms: 440 },
        },
        failure: {
          error_code: 'tool_permission_denied',
          error_stage: 'task_execution',
          message_id: 'job-1',
          channel: 'web',
          chat_id: 'chat-1',
          occurred_at_ms: 200,
          internal_message: 'tool was denied',
        },
        review: null,
        orchestration: null,
      },
    ],
  }),
}));

vi.mock('@/shared/api/status', () => ({
  useStatusQuery: () => ({
    data: {
      agent: { status: 'running', context_status: 'ready' },
      worker_pool: { active: 1, size: 2, idle: 1, stopped: 0 },
      workers: [
        {
          id: 'worker-1',
          status: 'busy',
          current_task: {
            task_id: 'task-1',
            trace_id: 'trace-1',
            task_type: 'api-call',
            elapsed_ms: 500,
            content_preview: 'preview',
          },
        },
      ],
      recent_sessions: [
        {
          session_id: 'chat-1',
          chat_id: 'chat-1',
          channel: 'web',
          history_len: 1,
          last_user_message_preview: 'hello',
          last_active: 200,
        },
      ],
      failures: {
        last_failure: {
          error_code: 'global_error',
          error_stage: 'plan_execute',
          internal_message: 'global failure',
          occurred_at_ms: 100,
          chat_id: 'chat-2',
        },
      },
      warnings: {
        bus_drop_total: 0,
      },
      metrics: {
        gauges: {
          bus_inbound_queue_depth: 0,
          bus_outbound_queue_depth: 0,
          bus_internal_queue_depth: 0,
          bus_control_queue_depth: 0,
        },
      },
      runtime_timeline: [
        {
          event: 'tool_execution_failed',
          trace_id: 'trace-1',
          message_id: 'job-1',
          chat_id: 'chat-1',
          recorded_at_ms: 210,
        },
        {
          event: 'message_completed',
          trace_id: 'trace-2',
          message_id: 'job-2',
          chat_id: 'chat-2',
          recorded_at_ms: 220,
        },
      ],
      recent_execution_summaries: [
        {
          trace_id: 'trace-1',
          message_id: 'job-1',
          chat_id: 'chat-1',
          plan_type: 'single',
          status: 'failed',
          cache_hit: false,
          worker_id: 'worker-1',
          error_code: 'tool_permission_denied',
          error_stage: 'task_execution',
          task_duration_ms: 500,
          stages: { context_ms: 10, session_ms: 20, classify_ms: 30, execute_ms: 440 },
        },
      ],
    },
    isLoading: false,
    error: null,
  }),
}));

function renderInspector() {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <DiagnosticsInspector />
    </QueryClientProvider>,
  );
}

describe('DiagnosticsInspector', () => {
  beforeEach(() => {
    useUiStore.setState({
      selectedScope: 'job',
      selectedSessionId: 'chat-1',
      selectedJobId: 'job-1',
      selectNewestJob: false,
      activeInspectorTab: 'failure',
      isInspectorOpen: true,
      isJobsDrawerOpen: false,
      jobListFilter: 'all',
    });
  });

  it('prefers selected job failure over unrelated global failure', () => {
    renderInspector();

    expect(screen.getByText(/code=tool_permission_denied/)).toBeTruthy();
    expect(screen.getByText(/stage=task_execution/)).toBeTruthy();
    expect(screen.getByText(/tool was denied/)).toBeTruthy();
    expect(screen.queryByText(/global failure/)).toBeNull();
  });

  it('filters timeline and execution summary by selected job context', () => {
    renderInspector();

    fireEvent.click(screen.getByRole('button', { name: 'Timeline' }));
    expect(screen.getByText('tool_execution_failed')).toBeTruthy();
    expect(screen.queryByText('message_completed')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Execution' }));
    expect(screen.getByText(/plan=single/)).toBeTruthy();
    expect(screen.getByText(/worker=worker-1/)).toBeTruthy();
  });

  it('falls back to session-scoped diagnostics when no job is selected', () => {
    useUiStore.setState({
      selectedScope: 'session',
      selectedSessionId: 'chat-1',
      selectedJobId: null,
      activeInspectorTab: 'timeline',
    });

    renderInspector();

    expect(screen.getByText('tool_execution_failed')).toBeTruthy();
    expect(screen.queryByText('message_completed')).toBeNull();
  });

  it('shows global diagnostics when no session or job is selected', () => {
    useUiStore.setState({
      selectedScope: 'global',
      selectedSessionId: null,
      selectedJobId: null,
      activeInspectorTab: 'failure',
    });

    renderInspector();

    expect(screen.getByText(/code=global_error/)).toBeTruthy();
    expect(screen.getByText(/global failure/)).toBeTruthy();
  });

  it('renders collapsed inspector toggle without diagnostics body', () => {
    useUiStore.setState({
      isInspectorOpen: false,
    });

    renderInspector();

    expect(screen.getByRole('button', { name: 'Inspector' })).toBeTruthy();
    expect(screen.queryByText('次级诊断信息集中展示，不打断主工作流')).toBeNull();
  });
});
