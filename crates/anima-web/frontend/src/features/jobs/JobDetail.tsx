import { useMemo, useState } from 'react';
import type { JobView } from '@/shared/utils/types';
import { useUiStore } from '@/shared/state/useUiStore';
import { formatDurationShort, formatTimestamp, shortId } from '@/shared/utils/format';
import { statusTone } from '@/shared/utils/jobStatus';
import { useReviewJobMutation } from '@/shared/api/review';
import { useQuestionAnswerMutation } from '@/shared/api/questionAnswer';
import { deriveJobSummary } from './deriveJobSummary';
import { deriveAgentDecisionSummary, deriveProcessEntriesFromEvents } from './deriveProcessEntries';
import './jobs.css';

interface JobDetailProps {
  jobs: JobView[];
  selectedSessionChatId: string | null;
  jobId?: string | null;
}

interface FailureView {
  error_code?: string;
  error_stage?: string;
  internal_message?: string;
  occurred_at_ms?: number;
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

export function JobDetail({ jobs, selectedSessionChatId, jobId }: JobDetailProps) {
  const [activeTab, setActiveTab] = useState<JobDetailTab>('summary');
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const resolvedJobId = jobId ?? selectedJobId;
  const job = useMemo(() => jobs.find((item) => item.job_id === resolvedJobId) ?? jobs[0], [jobs, resolvedJobId]);
  const { mutate: reviewJob, isPending } = useReviewJobMutation(job?.job_id);
  const { mutate: answerQuestion, isPending: isAnswering } = useQuestionAnswerMutation(job?.job_id);
  const [questionInput, setQuestionInput] = useState('');

  if (!job) {
    return <div className="jobs-empty">{selectedSessionChatId ? `当前会话 ${shortId(selectedSessionChatId)} 暂无可查看的 Job` : '请选择一个 Job'}</div>;
  }

  const canReview = job.status === 'completed';
  const failure = (job.failure ?? null) as FailureView | null;
  const executionSummary = (job.execution_summary ?? null) as ExecutionSummaryView | null;
  const pendingQuestion = job.pending_question ?? null;
  const summary = deriveJobSummary(job);
  const processEntries = deriveProcessEntriesFromEvents(job.recent_events);
  const decisionSummary = deriveAgentDecisionSummary(processEntries);
  const orchestration = job.orchestration ?? null;
  const subtaskJobs = useMemo(
    () => jobs.filter((item) => item.parent_job_id === job.job_id),
    [jobs, job.job_id],
  );

  return (
    <div className="job-detail">
      <div className="job-detail-header">
        <div>
          <div className="job-detail-title">{summary.statusLabelText}</div>
          <div className="job-detail-subtitle">job={shortId(job.job_id)} · trace={shortId(job.trace_id)}</div>
        </div>
        <div className="job-detail-header-badges">
          <span className={`job-kind-pill ${job.kind}`}>{job.kind === 'main' ? '主任务' : '子任务'}</span>
          <span className={`job-status-pill ${summary.statusTone}`}>{job.status}</span>
        </div>
      </div>

      <div className="job-detail-section job-hero-section">
        <div className="job-status-panel">
          <div className="job-status-panel-main">
            <div className="job-detail-step">{summary.headline}</div>
            <div className="job-status-explanation">{summary.detail}</div>
          </div>
          <div className="job-status-panel-side">
            <div className="job-status-panel-label">当前动作</div>
            <div>{job.current_step || summary.statusLabelText}</div>
            <div className="job-status-panel-label">层级</div>
            <div>{summary.hierarchyText}</div>
          </div>
        </div>
        <div className="job-hierarchy-note">{summary.hierarchyText}</div>
        {job.kind === 'main' && orchestration && orchestration.total_subtasks > 0 && (
          <div className="job-detail-section">
            <div className="section-title">Orchestration 概览</div>
            <div className="job-detail-grid summary-grid">
              <span className="detail-label">plan</span><span>{orchestration.plan_id ?? '-'}</span>
              <span className="detail-label">总子任务</span><span>{orchestration.total_subtasks}</span>
              <span className="detail-label">已完成</span><span>{orchestration.completed_subtasks}</span>
              <span className="detail-label">失败</span><span>{orchestration.failed_subtasks}</span>
              <span className="detail-label">活跃子任务</span><span>{orchestration.active_subtask_name ?? '-'}</span>
            </div>
            <div className="job-process-list">
              {subtaskJobs.length === 0 ? (
                <div className="jobs-empty">当前主任务已记录 orchestration 事件，但还没有独立子任务 Job。</div>
              ) : (
                subtaskJobs.map((subtask) => (
                  <div key={subtask.job_id} className="job-process-item">
                    <div className="job-process-summary">
                      <div>
                        <div className="job-process-title">{subtask.current_step || shortId(subtask.job_id)}</div>
                        <div className="job-message-meta">{subtask.kind === 'subtask' ? `子任务 ${shortId(subtask.job_id)}` : shortId(subtask.job_id)}</div>
                      </div>
                      <span className={`job-status-pill ${statusTone(subtask.status)}`}>{subtask.status}</span>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        )}
        {job.status === 'waiting_user_input' && pendingQuestion && (
          <div className="job-detail-section question-card">
            <div className="section-title">待处理问题</div>
            <div className="job-status-explanation">这里只展示上游返回的真实结构化 question；下方 JSON 为 question 原文。</div>
            <div className="job-detail-grid summary-grid">
              <span className="detail-label">kind</span><span>{pendingQuestion.question_kind}</span>
              <span className="detail-label">mode</span><span>{pendingQuestion.decision_mode}</span>
              <span className="detail-label">risk</span><span>{pendingQuestion.risk_level}</span>
              <span className="detail-label">prompt</span><span className="job-prewrap">{pendingQuestion.prompt}</span>
              <span className="detail-label">question 原文</span>
              <span className="job-prewrap">{JSON.stringify(pendingQuestion.raw_question, null, 2)}</span>
            </div>
            {pendingQuestion.requires_user_confirmation && (
              <div className="job-status-explanation">该问题不能由主 agent 自动决定，需要用户显式确认。</div>
            )}
            {pendingQuestion.options.length > 0 && (
              <div className="job-review-actions">
                {pendingQuestion.options.map((option) => (
                  <button
                    key={option}
                    type="button"
                    disabled={isAnswering}
                    className="job-review-btn accept"
                    onClick={() => answerQuestion({
                      question_id: pendingQuestion.question_id,
                      source: 'user',
                      answer_type: pendingQuestion.question_kind === 'confirm' ? 'confirm' : 'choice',
                      answer: option,
                    })}
                  >
                    {option}
                  </button>
                ))}
              </div>
            )}
            {pendingQuestion.question_kind === 'input' && (
              <div className="job-detail-stack">
                <textarea
                  className="job-result-preview"
                  rows={4}
                  value={questionInput}
                  onChange={(event) => setQuestionInput(event.target.value)}
                  placeholder="输入给上游问题的回答"
                />
                <div className="job-review-actions">
                  <button
                    type="button"
                    disabled={isAnswering || !questionInput.trim()}
                    className="job-review-btn accept"
                    onClick={() => {
                      answerQuestion({
                        question_id: pendingQuestion.question_id,
                        source: 'user',
                        answer_type: 'text',
                        answer: questionInput.trim(),
                      });
                      setQuestionInput('');
                    }}
                  >
                    提交回答
                  </button>
                </div>
              </div>
            )}
            {pendingQuestion.answer_summary && (
              <div className="job-message-meta">最近已提交回答：{pendingQuestion.answer_summary}</div>
            )}
          </div>
        )}
        {summary.isAutoContinuing && (
          <div className="job-status-explanation">
            主 agent 判断当前结果还没完全满足原始需求，正在自动继续补充处理；此时不需要用户回答问题。
          </div>
        )}
        {summary.isStalledWithoutQuestion && (
          <div className="job-status-explanation">
            当前只是暂时停滞提示，没有结构化问题可提交回答。可以稍后刷新，或查看事件判断是否需要人工介入。
          </div>
        )}
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
            <span className="detail-label">kind</span><span>{job.kind === 'main' ? '主任务' : '子任务'}</span>
            <span className="detail-label">parent</span><span>{job.parent_job_id ? shortId(job.parent_job_id) : '-'}</span>
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
          {summary.hasResponse ? (
            <div className="job-result-preview job-prewrap">{summary.latestResponseText}</div>
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
            <div className="section-title">当前 worker / 任务</div>
            {job.worker ? (
              <div className="job-detail-grid summary-grid">
                <span className="detail-label">worker</span><span>{job.worker.worker_id}</span>
                <span className="detail-label">status</span><span>{job.worker.status}</span>
                <span className="detail-label">task</span><span>{job.worker.task_type}</span>
                <span className="detail-label">preview</span><span className="job-prewrap">{job.worker.content_preview || '-'}</span>
              </div>
            ) : (
              <div className="jobs-empty">当前没有活跃 worker</div>
            )}
          </div>
          <div>
            <div className="section-title">主 agent 决策</div>
            {decisionSummary ? (
              <div className="job-decision-card">
                <div className="job-decision-title">{decisionSummary.title}</div>
                <div className="job-detail-grid summary-grid">
                  <span className="detail-label">原因</span><span className="job-prewrap">{decisionSummary.reason}</span>
                  <span className="detail-label">下一步</span><span className="job-prewrap">{decisionSummary.nextAction}</span>
                  <span className="detail-label">时间</span><span>{decisionSummary.timestamp ? formatTimestamp(decisionSummary.timestamp) : '-'}</span>
                </div>
              </div>
            ) : (
              <div className="jobs-empty">暂无可提炼的主 agent 决策</div>
            )}
          </div>
          <div>
            <div className="section-title">详细过程时间线</div>
            {processEntries.length === 0 ? (
              <div className="jobs-empty">暂无可展示的执行过程</div>
            ) : (
              <div className="job-process-list">
                {processEntries.map((entry) => (
                  <details key={entry.id} className="job-process-item">
                    <summary className="job-process-summary">
                      <div>
                        <div className="job-process-title">{entry.title}</div>
                        <div className="job-message-meta">{entry.detail}</div>
                        {entry.preview && <div className="job-process-preview">{entry.preview}</div>}
                      </div>
                      <span className="job-message-meta">{formatTimestamp(entry.timestamp)}</span>
                    </summary>
                    {entry.raw !== undefined && (
                      <pre className="job-process-raw">{JSON.stringify(entry.raw, null, 2)}</pre>
                    )}
                  </details>
                ))}
              </div>
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
