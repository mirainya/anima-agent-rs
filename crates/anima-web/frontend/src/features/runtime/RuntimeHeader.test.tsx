import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { RuntimeHeader } from './RuntimeHeader';
import { useUiStore } from '@/shared/state/useUiStore';

vi.mock('@/shared/api/jobs', () => ({
  useJobsQuery: () => ({
    data: [
      {
        job_id: 'job-1234567890',
        trace_id: 'trace-1',
        message_id: 'msg-1',
        kind: 'main',
        parent_job_id: null,
        channel: 'web',
        chat_id: 'chat-1234567890',
        sender_id: 'web-user',
        user_content: 'run job',
        status: 'waiting_user_input',
        status_label: 'waiting_user_input',
        accepted: true,
        started_at_ms: 100,
        updated_at_ms: 120,
        elapsed_ms: 20,
        current_step: '等待用户确认工具调用权限',
        pending_question: null,
        recent_events: [],
        worker: null,
        tool_state: null,
        execution_summary: null,
        failure: null,
        review: null,
        orchestration: null,
      },
    ],
  }),
}));

vi.mock('@/shared/api/status', () => ({
  useStatusQuery: () => ({
    data: {
      agent: { status: 'running' },
      worker_pool: { active: 1, size: 3 },
      warnings: { bus_overflow_active: true, bus_drop_total: 3 },
      metrics: { counters: {}, gauges: {}, histograms: {} },
      workers: [
        { id: 'worker-a', status: 'busy' },
        { id: 'worker-b', status: 'idle' },
      ],
      recent_sessions: [
        {
          session_id: 'chat-1234567890',
          chat_id: 'chat-1234567890',
          channel: 'web',
          history_len: 1,
          last_user_message_preview: 'run job',
          last_active: 120,
        },
      ],
    },
  }),
}));

function renderHeader(sseState: 'connected' | 'connecting' | 'disconnected' = 'connected') {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <RuntimeHeader sseState={sseState} />
    </QueryClientProvider>,
  );
}

describe('RuntimeHeader', () => {
  beforeEach(() => {
    useUiStore.setState({
      selectedScope: 'job',
      selectedSessionId: 'chat-1234567890',
      selectedJobId: 'job-1234567890',
      selectNewestJob: false,
      activeInspectorTab: 'overview',
      isInspectorOpen: true,
      isJobsDrawerOpen: false,
      jobListFilter: 'all',
    });
  });

  it('renders selected job scope and current runtime meta', () => {
    renderHeader();

    expect(screen.getByText(/运行时就绪/)).toBeTruthy();
    expect(screen.getByText('Agent running')).toBeTruthy();
    expect(screen.getByText('Pool 1/3')).toBeTruthy();
    expect(screen.getByText('Bus 压力 3')).toBeTruthy();
    expect(screen.getByText(/当前 Job job-1234/)).toBeTruthy();
    expect(screen.getByLabelText('workers').textContent).toContain('W');
  });

  it('renders waiting-state step labels and collapse button text', () => {
    renderHeader();

    expect(screen.getByLabelText('runtime steps').textContent).toContain('待确认');
    expect(screen.getByLabelText('runtime steps').textContent).toContain('等待用户确认工具调用权限');
    expect(screen.getByRole('button', { name: '收起 Inspector' })).toBeTruthy();
  });

  it('falls back to session scope summary when no job is selected', () => {
    useUiStore.setState({
      selectedScope: 'session',
      selectedSessionId: 'chat-1234567890',
      selectedJobId: null,
    });

    renderHeader();

    expect(screen.getByText(/当前会话 chat-123/)).toBeTruthy();
    expect(screen.queryByText(/当前 Job/)).toBeNull();
  });

  it('shows global summary and connection status without selected context', () => {
    useUiStore.setState({
      selectedScope: 'global',
      selectedSessionId: null,
      selectedJobId: null,
      isInspectorOpen: false,
    });

    renderHeader('connecting');

    expect(screen.getByText('全部会话')).toBeTruthy();
    expect(screen.getByText(/正在连接运行时事件流/)).toBeTruthy();
    expect(screen.getByRole('button', { name: '展开 Inspector' })).toBeTruthy();
  });
});
