import { useEffect } from 'react';
import type { JobView } from '@/shared/utils/types';
import { useUiStore } from '@/shared/state/useUiStore';
import { formatDurationShort, shortId } from '@/shared/utils/format';
import { statusLabel, statusTone } from '@/shared/utils/jobStatus';
import './jobs.css';

interface JobsListProps {
  jobs: JobView[];
  selectedSessionChatId: string | null;
}

function summarizeStatus(job: {
  status: string;
  worker?: { task_type?: string } | null;
  review?: { verdict?: string } | null;
  failure?: { error_code?: string; error_stage?: string } | null;
  recent_events: Array<{ event: string }>;
}): string {
  const lastEvent = job.recent_events[job.recent_events.length - 1]?.event;

  switch (job.status) {
    case 'completed':
      if (job.review?.verdict === 'accepted') return '用户已接受结果';
      if (job.review?.verdict === 'rejected') return '用户拒绝了结果';
      return '任务已完成，等待用户反馈';
    case 'failed':
      if (job.failure?.error_code) return `失败：${job.failure.error_code}`;
      if (lastEvent === 'session_create_failed') return '上游会话创建失败';
      if (lastEvent === 'message_failed') return '执行阶段失败';
      return '任务执行失败';
    case 'executing':
      if (job.worker?.task_type) return `执行 ${job.worker.task_type}`;
      if (lastEvent === 'cache_hit') return '缓存命中后处理中';
      if (lastEvent === 'cache_miss') return '缓存未命中后执行中';
      return '正在执行任务';
    case 'creating_session':
      return '正在创建上游会话';
    case 'planning':
      if (lastEvent === 'plan_built') return '规划已完成，准备执行';
      if (lastEvent === 'session_ready') return '会话就绪，构建计划中';
      return '正在规划任务';
    case 'waiting_upstream_input':
      return '等待上游交互输入';
    case 'preparing_context':
      return '正在准备上下文';
    case 'queued':
      return '等待系统开始处理';
    default:
      return '状态推导中';
  }
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
        const tone = statusTone(job.status);
        const statusSummary = summarizeStatus({
          status: job.status,
          worker: job.worker,
          review: job.review,
          failure: (job.failure ?? null) as { error_code?: string; error_stage?: string } | null,
          recent_events: job.recent_events,
        });
        return (
          <button
            key={job.job_id}
            type="button"
            className={`job-list-card ${active ? 'active' : ''}`}
            onClick={() => setSelectedJobId(job.job_id)}
          >
            <div className="job-list-top">
              <span className={`job-status-pill ${tone}`}>{statusLabel(job.status)}</span>
              <span className="job-meta-time">{formatDurationShort(job.elapsed_ms)}</span>
            </div>
            <div className="job-list-id">job={shortId(job.job_id)} · trace={shortId(job.trace_id)}</div>
            <div className="job-list-step">{job.current_step}</div>
            <div className="job-list-reason">{statusSummary}</div>
            <div className="job-list-bottom">
              <span>chat={shortId(job.chat_id ?? null)}</span>
              <span>{job.worker?.task_type ?? '-'}</span>
            </div>
          </button>
        );
      })}
    </div>
  );
}
