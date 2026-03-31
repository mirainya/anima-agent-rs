import { useMemo, useState } from 'react';
import type { JobView } from '@/shared/utils/types';
import { useUiStore } from '@/shared/state/useUiStore';
import { formatDurationShort, formatTimestamp, shortId } from '@/shared/utils/format';
import { statusLabel, statusTone } from '@/shared/utils/jobStatus';
import { useReviewJobMutation } from '@/shared/api/review';
import './jobs.css';

interface JobDetailProps {
  jobs: JobView[];
  selectedSessionChatId: string | null;
}

interface FailureView {
  error_code?: string;
  error_stage?: string;
  internal_message?: string;
  occurred_at_ms?: number;
}

interface JobEventPayloadView {
  response_preview?: string;
  response_text?: string;
}

interface ExecutionSummaryView {
  plan_type?: string;
  status?: string;
  cache_hit?: boolean;
  worker_id?: string | null;
  error_code?: string | null;
  error_stage?: string | null;
  task_duration_ms?: number;
  stages?: {
    context_ms?: number;
    session_ms?: number;
    classify_ms?: number;
    execute_ms?: number;
    total_ms?: number;
  };
}

type JobDetailTab = 'summary' | 'result' | 'execution' | 'events';

function explainStatus(job: {
  status: string;
  worker?: { task_type?: string; worker_id?: string } | null;
  review?: { verdict?: string } | null;
  failure?: FailureView | null;
  execution_summary?: ExecutionSummaryView | null;
  recent_events: Array<{ event: string }>;
}): string {
  const lastEvent = job.recent_events[job.recent_events.length - 1]?.event;

  switch (job.status) {
    case 'completed':
      if (job.review?.verdict === 'accepted') {
        return '该 Job 已执行完成，且用户已接受这次结果。';
      }
      if (job.review?.verdict === 'rejected') {
        return '该 Job 已执行完成，但用户拒绝了这次结果；这属于反馈，不会改变主任务状态。';
      }
      return '该 Job 已执行完成，当前可以继续提交结果反馈。';
    case 'failed':
      if (job.failure?.error_code || job.failure?.error_stage) {
        return `因为运行时记录了失败快照（${job.failure.error_stage ?? '-'} / ${job.failure.error_code ?? '-'}），所以该 Job 已失败。`;
      }
      if (lastEvent === 'session_create_failed') {
        return '因为上游会话创建失败，所以该 Job 被判定为失败。';
      }
      if (lastEvent === 'message_failed') {
        return '因为消息执行阶段返回失败事件，所以该 Job 被判定为失败。';
      }
      return '该 Job 遇到了失败信号，当前需要排查失败原因。';
    case 'executing':
      if (job.worker?.task_type) {
        return `因为 worker 正在执行 ${job.worker.task_type}，所以该 Job 仍处于执行中。`;
      }
      if (lastEvent === 'cache_hit' || lastEvent === 'cache_miss') {
        return `因为最近事件是 ${lastEvent}，说明任务已经进入执行阶段，所以当前仍显示为执行中。`;
      }
      return '该 Job 已经过了规划阶段，正在执行实际任务。';
    case 'creating_session':
      return '因为当前 worker 正在执行 session-create，所以系统判断它正在创建上游会话。';
    case 'planning':
      if (lastEvent === 'plan_built') {
        return `因为规划刚完成，正在准备执行 ${job.execution_summary?.plan_type ?? '当前计划'}。`;
      }
      if (lastEvent === 'session_ready') {
        return '因为上游会话已经就绪，系统正在构建后续执行计划。';
      }
      return '该 Job 正在进行规划与执行前准备。';
    case 'waiting_upstream_input':
      return '因为 plan_built 之后长时间没有新的更强信号，系统推断它可能正在等待上游交互输入。';
    case 'preparing_context':
      return '因为已经收到消息，但还没进入会话或规划阶段，所以当前显示为准备上下文。';
    case 'queued':
      return '因为 Job 已被接收，但还没有出现更进一步的运行时事件，所以当前仍在队列中。';
    default:
      return '当前状态由最近的运行时事件、worker 情况和 review / failure 信息共同推导得出。';
  }
}

export function JobDetail({ jobs, selectedSessionChatId }: JobDetailProps) {
  const [activeTab, setActiveTab] = useState<JobDetailTab>('summary');
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const job = useMemo(() => jobs.find((item) => item.job_id === selectedJobId) ?? jobs[0], [jobs, selectedJobId]);
  const { mutate: reviewJob, isPending } = useReviewJobMutation(job?.job_id);

  if (!job) {
    return <div className="jobs-empty">{selectedSessionChatId ? `当前会话 ${shortId(selectedSessionChatId)} 暂无可查看的 Job` : '请选择一个 Job'}</div>;
  }

  const canReview = job.status === 'completed';
  const failure = (job.failure ?? null) as FailureView | null;
  const executionSummary = (job.execution_summary ?? null) as ExecutionSummaryView | null;
  const responsePreview = job.recent_events
    .slice()
    .reverse()
    .find((event) => event.event === 'message_completed')?.payload as JobEventPayloadView | undefined;
  const statusExplanation = explainStatus({
    status: job.status,
    worker: job.worker,
    review: job.review,
    failure,
    execution_summary: executionSummary,
    recent_events: job.recent_events,
  });

  return (
    <div className="job-detail">
      <div className="job-detail-header">
        <div>
          <div className="job-detail-title">{statusLabel(job.status)}</div>
          <div className="job-detail-subtitle">job={shortId(job.job_id)} · trace={shortId(job.trace_id)}</div>
        </div>
        <span className={`job-status-pill ${statusTone(job.status)}`}>{job.status}</span>
      </div>

      <div className="job-detail-section job-hero-section">
        <div className="job-detail-step">{job.current_step}</div>
        <div className="job-status-explanation">{statusExplanation}</div>
        {canReview && (
          <div className="job-review-actions">
            <button
              type="button"
              disabled={isPending}
              onClick={() => reviewJob('accepted')}
              className="job-review-btn accept"
            >
              接受结果
            </button>
            <button
              type="button"
              disabled={isPending}
              onClick={() => reviewJob('rejected')}
              className="job-review-btn reject"
            >
              拒绝结果
            </button>
          </div>
        )}
      </div>

      <div className="job-detail-tabs" role="tablist" aria-label="Job detail tabs">
        {[
          { key: 'summary', label: '概览' },
          { key: 'result', label: '结果' },
          { key: 'execution', label: '执行' },
          { key: 'events', label: '事件' },
        ].map((tab) => (
          <button
            key={tab.key}
            type="button"
            className={`job-detail-tab ${activeTab === tab.key ? 'active' : ''}`}
            onClick={() => setActiveTab(tab.key as JobDetailTab)}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {activeTab === 'summary' && (
        <div className="job-detail-section">
          <div className="job-detail-grid">
            <span className="detail-label">chat</span><span>{shortId(job.chat_id ?? null)}</span>
            <span className="detail-label">channel</span><span>{job.channel}</span>
            <span className="detail-label">sender</span><span>{job.sender_id}</span>
            <span className="detail-label">elapsed</span><span>{formatDurationShort(job.elapsed_ms)}</span>
            <span className="detail-label">updated</span><span>{formatTimestamp(job.updated_at_ms)}</span>
            <span className="detail-label">worker</span><span>{job.worker?.worker_id ?? '-'}</span>
            <span className="detail-label">task</span><span>{job.worker?.task_type ?? '-'}</span>
            <span className="detail-label">review</span><span>{job.review?.verdict ?? '-'}</span>
          </div>
        </div>
      )}

      {activeTab === 'result' && (
        <div className="job-detail-section">
          <div className="section-title">返回结果</div>
          {responsePreview?.response_text || responsePreview?.response_preview ? (
            <div className="job-result-preview job-prewrap">{responsePreview?.response_text ?? responsePreview?.response_preview}</div>
          ) : (
            <div className="jobs-empty">当前暂无可展示的返回结果</div>
          )}
        </div>
      )}

      {activeTab === 'execution' && (
        <div className="job-detail-section job-detail-stack">
          <div>
            <div className="section-title">执行摘要</div>
            {executionSummary ? (
              <div className="job-detail-grid summary-grid">
                <span className="detail-label">plan</span><span>{executionSummary.plan_type ?? '-'}</span>
                <span className="detail-label">status</span><span>{executionSummary.status ?? '-'}</span>
                <span className="detail-label">cache</span><span>{executionSummary.cache_hit ? 'hit' : 'miss'}</span>
                <span className="detail-label">worker</span><span>{executionSummary.worker_id ?? '-'}</span>
                <span className="detail-label">task ms</span><span>{formatDurationShort(executionSummary.task_duration_ms ?? 0)}</span>
                <span className="detail-label">stages</span>
                <span>
                  ctx {formatDurationShort(executionSummary.stages?.context_ms ?? 0)} · sess {formatDurationShort(executionSummary.stages?.session_ms ?? 0)} · cls {formatDurationShort(executionSummary.stages?.classify_ms ?? 0)} · exe {formatDurationShort(executionSummary.stages?.execute_ms ?? 0)}
                </span>
              </div>
            ) : (
              <div className="jobs-empty">暂无执行摘要</div>
            )}
          </div>
          <div>
            <div className="section-title">失败信息</div>
            {failure ? (
              <div className="job-detail-grid summary-grid">
                <span className="detail-label">code</span><span>{failure.error_code ?? '-'}</span>
                <span className="detail-label">stage</span><span>{failure.error_stage ?? '-'}</span>
                <span className="detail-label">time</span><span>{failure.occurred_at_ms ? formatTimestamp(failure.occurred_at_ms) : '-'}</span>
                <span className="detail-label">message</span><span className="job-prewrap">{failure.internal_message ?? '-'}</span>
              </div>
            ) : (
              <div className="jobs-empty">暂无失败信息</div>
            )}
          </div>
        </div>
      )}

      {activeTab === 'events' && (
        <div className="job-detail-section">
          <div className="section-title">最近事件</div>
          <div className="job-events">
            {job.recent_events.length === 0 ? (
              <div className="jobs-empty">暂无事件</div>
            ) : (
              job.recent_events.map((event) => (
                <div key={`${event.event}-${event.recorded_at_ms}`} className="job-event-row">
                  <span>{event.event}</span>
                  <span>{formatTimestamp(event.recorded_at_ms)}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
