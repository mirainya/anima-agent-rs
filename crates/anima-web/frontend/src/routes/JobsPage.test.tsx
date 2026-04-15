import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { JobsPage } from './JobsPage';
import { useUiStore } from '@/shared/state/useUiStore';

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
        chat_id: 'chat-1',
        sender_id: 'web-user',
        user_content: 'first job',
        status: 'running',
        status_label: 'executing',
        accepted: true,
        started_at_ms: 100,
        updated_at_ms: 120,
        elapsed_ms: 20,
        current_step: '处理中',
        pending_question: null,
        recent_events: [],
        worker: null,
        tool_state: null,
        execution_summary: null,
        failure: null,
        review: null,
        orchestration: null,
      },
      {
        job_id: 'job-2',
        trace_id: 'trace-2',
        message_id: 'msg-2',
        kind: 'main',
        parent_job_id: null,
        channel: 'web',
        chat_id: 'chat-2',
        sender_id: 'web-user',
        user_content: 'second job',
        status: 'completed',
        status_label: 'completed',
        accepted: true,
        started_at_ms: 200,
        updated_at_ms: 260,
        elapsed_ms: 60,
        current_step: '已完成',
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

vi.mock('@/shared/api/sessions', () => ({
  useSessionsQuery: () => ({
    data: [
      {
        session_id: 'chat-1',
        chat_id: 'chat-1',
        channel: 'web',
        history_len: 1,
        last_user_message_preview: 'first job',
        last_active: 120,
      },
      {
        session_id: 'chat-2',
        chat_id: 'chat-2',
        channel: 'web',
        history_len: 1,
        last_user_message_preview: 'second job',
        last_active: 260,
      },
    ],
  }),
}));

vi.mock('@/shared/api/status', () => ({
  useStatusQuery: () => ({
    data: {
      runtime_timeline: [],
    },
  }),
}));

vi.mock('@/features/chat/MessageComposer', () => ({
  MessageComposer: () => <div>composer</div>,
}));

vi.mock('@/features/jobs/JobsConversation', () => ({
  JobsConversation: ({ selectedJob, selectedSession }: { selectedJob: { job_id: string } | null; selectedSession: { chat_id: string } | null }) => (
    <div>
      <div data-testid="selected-job">{selectedJob?.job_id ?? 'none'}</div>
      <div data-testid="selected-session">{selectedSession?.chat_id ?? 'none'}</div>
    </div>
  ),
}));

vi.mock('@/features/jobs/JobWorkbenchDrawer', () => ({
  JobWorkbenchDrawer: () => <div>drawer</div>,
}));

function renderPage(initialEntry: string) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/" element={<JobsPage />} />
          <Route path="/jobs/:jobId" element={<JobsPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe('JobsPage route selection', () => {
  beforeEach(() => {
    useUiStore.setState({
      selectedScope: 'global',
      selectedSessionId: 'chat-1',
      selectedJobId: 'job-1',
      selectNewestJob: false,
      activeInspectorTab: 'overview',
      isInspectorOpen: true,
      isJobsDrawerOpen: false,
      jobListFilter: 'all',
    });
  });

  afterEach(() => {
    cleanup();
  });

  it('uses route job id as source of truth for selected job and session', () => {
    renderPage('/jobs/job-2');

    expect(screen.getByTestId('selected-job').textContent).toBe('job-2');
    expect(screen.getByTestId('selected-session').textContent).toBe('chat-2');
  });

  it('falls back to store selection when route param is absent', () => {
    renderPage('/');

    expect(screen.getByTestId('selected-job').textContent).toBe('job-1');
    expect(screen.getByTestId('selected-session').textContent).toBe('chat-1');
  });
});
