import { useEffect } from 'react';
import type { JobView } from '@/shared/utils/types';
import { useUiStore } from '@/shared/state/useUiStore';
import { formatDurationShort, shortId } from '@/shared/utils/format';
import { deriveJobSummary } from './deriveJobSummary';
import { formatActiveSubtask } from './formatOrchestration';
import './jobs.css';

interface JobsListProps {
  jobs: JobView[];
  selectedSessionChatId: string | null;
}

export function JobsList({ jobs, selectedSessionChatId }: JobsListProps) {
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const setSelectedJobId = useUiStore((state) => state.setSelectedJobId);
  const selectNewestJob = useUiStore((state) => state.selectNewestJob);

  useEffect(() => {
    if (jobs.length === 0) {
      if (selectedJobId) {
        setSelectedJobId(null);
      }
      return;
    }

    const selectedStillExists = selectedJobId ? jobs.some((job) => job.job_id === selectedJobId) : false;
    if (selectNewestJob || !selectedJobId || !selectedStillExists) {
      setSelectedJobId(jobs[0].job_id);
    }
  }, [jobs, selectedJobId, selectNewestJob, setSelectedJobId]);

  if (jobs.length === 0) {
    return (
      <div className="jobs-empty">
        {selectedSessionChatId ? `当前会话 ${shortId(selectedSessionChatId)} 暂无 Job，可继续发送消息创建新任务。` : '当前暂无 Job，请先发送一条消息创建任务。'}
      </div>
    );
  }

  return (
    <div className="jobs-list">
      {jobs.map((job) => {
        const active = job.job_id === selectedJobId;
        const summary = deriveJobSummary(job);
        const orchestration = job.orchestration ?? null;
        return (
          <button
            key={job.job_id}
            type="button"
            className={`job-list-card ${active ? 'active' : ''}`}
            onClick={() => setSelectedJobId(job.job_id)}
          >
            <div className="job-list-top">
              <div className="job-list-badges">
                <span className={`job-status-pill ${summary.statusTone}`}>{summary.statusLabelText}</span>
                <span className={`job-kind-pill ${job.kind}`}>{job.kind === 'main' ? '主任务' : '子任务'}</span>
              </div>
              <span className="job-meta-time">{formatDurationShort(job.elapsed_ms)}</span>
            </div>
            <div className="job-list-id">job={shortId(job.job_id)} · trace={shortId(job.trace_id)}</div>
            <div className="job-list-step">{summary.headline}</div>
            <div className="job-list-reason">{summary.detail}</div>
            {job.kind === 'main' && orchestration && orchestration.total_subtasks > 0 && (
              <div className="job-hierarchy-note">
                子任务 {orchestration.completed_subtasks}/{orchestration.total_subtasks}
                {formatActiveSubtask(orchestration.active_subtask_name, orchestration.active_subtask_type)
                  ? ` · 当前 ${formatActiveSubtask(orchestration.active_subtask_name, orchestration.active_subtask_type)}`
                  : ''}
              </div>
            )}
            <div className="job-list-bottom">
              <span>chat={shortId(job.chat_id ?? null)}</span>
              <span>{job.parent_job_id ? `属于 ${shortId(job.parent_job_id)}` : job.worker?.task_type ?? '-'}</span>
            </div>
          </button>
        );
      })}
    </div>
  );
}
