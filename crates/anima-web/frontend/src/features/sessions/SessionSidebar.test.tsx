import { fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SessionSidebar } from './SessionSidebar';
import { useUiStore } from '@/shared/state/useUiStore';

vi.mock('@/shared/api/sessions', () => ({
  useSessionsQuery: () => ({
    data: [
      {
        session_id: 'session-1',
        chat_id: 'session-1',
        channel: 'web',
        history_len: 2,
        last_user_message_preview: '请帮我检查部署状态',
        last_active: 1710000000000,
      },
      {
        session_id: 'session-2',
        chat_id: 'session-2',
        channel: 'web',
        history_len: 1,
        last_user_message_preview: '继续执行第二步',
        last_active: 1710000001000,
      },
    ],
    isLoading: false,
  }),
}));

vi.mock('@/shared/api/jobs', () => ({
  useJobsQuery: () => ({
    data: [
      {
        job_id: 'job-1',
        trace_id: 'trace-1',
        message_id: 'msg-1',
        kind: 'main',
        parent_job_id: null,
        channel: 'web',
        chat_id: 'session-1',
        sender_id: 'user-1',
        user_content: '请帮我检查部署状态',
        status: 'executing',
        status_label: 'executing',
        accepted: true,
        started_at_ms: 1,
        updated_at_ms: 2,
        elapsed_ms: 1,
        current_step: '执行中',
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

function renderSidebar() {
  const client = new QueryClient();
  return render(
    <QueryClientProvider client={client}>
      <SessionSidebar />
    </QueryClientProvider>,
  );
}

describe('SessionSidebar', () => {
  beforeEach(() => {
    useUiStore.setState({
      selectedScope: 'global',
      selectedSessionId: null,
      selectedJobId: null,
      selectNewestJob: true,
    });
  });

  it('renders sessions from the formal sessions api and updates selected session', () => {
    renderSidebar();

    expect(screen.getByText('请帮我检查部署状态')).toBeTruthy();
    expect(screen.getByText('继续执行第二步')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: /session-1/i }));
    expect(useUiStore.getState().selectedSessionId).toBe('session-1');
  });
});
