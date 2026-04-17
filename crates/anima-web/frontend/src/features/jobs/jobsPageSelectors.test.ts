import { describe, expect, it } from 'vitest';
import {
  buildHierarchySummary,
  buildScopeSummary,
  filterJobsByListFilter,
  pickDetailJob,
  sortJobsByUpdatedAtAsc,
  sortJobsByUpdatedAtDesc,
} from './jobsPageSelectors';
import type { JobView, SessionSummary } from '@/shared/utils/types';
import type { WorkbenchContext } from '@/shared/utils/workbench';

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
    current_step: '处理中',
    pending_question: null,
    recent_events: [],
    worker: null,
    execution_summary: null,
    failure: null,
    review: null,
    tool_state: null,
    orchestration: null,
    ...partial,
  };
}

function makeContext(partial: Partial<WorkbenchContext>): WorkbenchContext {
  const selectedSession: SessionSummary = {
    session_id: 'chat-1',
    chat_id: 'chat-1',
    channel: 'web',
    history_len: 1,
    last_user_message_preview: 'hello',
    last_active: 100,
  };

  return {
    scope: 'global',
    selectedSession: null,
    selectedJob: null,
    sessionJobs: [],
    ...partial,
    selectedSession: partial.selectedSession === undefined ? null : partial.selectedSession,
    selectedJob: partial.selectedJob === undefined ? null : partial.selectedJob,
  };
}

describe('jobsPageSelectors', () => {
  it('filters active/review/failed job lists', () => {
    const jobs = [
      makeJob({ job_id: 'job-active', status: 'executing' }),
      makeJob({ job_id: 'job-review', status: 'completed', review: null }),
      makeJob({ job_id: 'job-reviewed', status: 'completed', review: { verdict: 'accepted', reviewed_at_ms: 1, note: null, reason: null } }),
      makeJob({ job_id: 'job-failed', status: 'failed' }),
    ];

    expect(filterJobsByListFilter(jobs, 'active').map((job) => job.job_id)).toEqual(['job-active']);
    expect(filterJobsByListFilter(jobs, 'review').map((job) => job.job_id)).toEqual(['job-review']);
    expect(filterJobsByListFilter(jobs, 'failed').map((job) => job.job_id)).toEqual(['job-failed']);
    expect(filterJobsByListFilter(jobs, 'all')).toHaveLength(4);
  });

  it('sorts jobs by updated time in both directions', () => {
    const jobs = [
      makeJob({ job_id: 'job-2', updated_at_ms: 200 }),
      makeJob({ job_id: 'job-1', updated_at_ms: 100 }),
      makeJob({ job_id: 'job-3', updated_at_ms: 300 }),
    ];

    expect(sortJobsByUpdatedAtAsc(jobs).map((job) => job.job_id)).toEqual(['job-1', 'job-2', 'job-3']);
    expect(sortJobsByUpdatedAtDesc(jobs).map((job) => job.job_id)).toEqual(['job-3', 'job-2', 'job-1']);
  });

  it('builds scope and hierarchy summaries', () => {
    const context = makeContext({ scope: 'session', selectedSession: {
      session_id: 'chat-1',
      chat_id: 'chat-1',
      channel: 'web',
      history_len: 1,
      last_user_message_preview: 'hello',
      last_active: 100,
    } });

    expect(buildScopeSummary('job', context)).toBe('当前任务');
    expect(buildScopeSummary('session', context)).toContain('chat-1');
    expect(buildScopeSummary('global', context)).toBe('全部会话');

    expect(buildHierarchySummary([
      makeJob({ job_id: 'main-1', kind: 'main' }),
      makeJob({ job_id: 'sub-1', kind: 'subtask' }),
      makeJob({ job_id: 'sub-2', kind: 'subtask' }),
    ])).toEqual({ main: 1, subtasks: 2 });
  });

  it('prefers selected detail job and falls back to first visible job', () => {
    const selected = makeJob({ job_id: 'job-selected' });
    const visible = [makeJob({ job_id: 'job-first' }), makeJob({ job_id: 'job-second' })];

    expect(pickDetailJob(selected, visible)?.job_id).toBe('job-selected');
    expect(pickDetailJob(null, visible)?.job_id).toBe('job-first');
    expect(pickDetailJob(null, [])).toBeNull();
  });
});
