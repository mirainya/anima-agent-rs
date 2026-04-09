import type { JobView, SessionSummary } from '@/shared/utils/types';
import { formatTimestamp, shortId } from '@/shared/utils/format';
import { deriveJobSummary } from './deriveJobSummary';
import { deriveAgentDecisionSummary, deriveProcessEntriesFromJob, deriveProcessEntriesFromRuntimeTimeline } from './deriveProcessEntries';
import { formatActiveSubtask } from './formatOrchestration';
import './jobs.css';

interface RuntimeTimelineEvent {
  event: string;
  trace_id: string;
  message_id: string;
  channel: string;
  chat_id?: string | null;
  sender_id: string;
  recorded_at_ms: number;
  payload: unknown;
}

interface JobsConversationProps {
  jobs: JobView[];
  selectedJob: JobView | null;
  selectedSession: SessionSummary | null;
  selectedSessionId: string | null;
  scopeSummary: string;
  runtimeTimeline: RuntimeTimelineEvent[];
  onOpenJobDetails: (jobId: string) => void;
}

function getUserMessage(job: JobView): string {
  return job.user_content?.trim() || '这条用户消息暂时没有可展示的原文。';
}

export function JobsConversation({ jobs, selectedJob, selectedSession, selectedSessionId, scopeSummary, runtimeTimeline, onOpenJobDetails }: JobsConversationProps) {
  if (!selectedSessionId) {
    return (
      <div className="jobs-conversation-empty-state">
        <div className="jobs-conversation-empty-kicker">当前是全局视角</div>
        <div className="jobs-conversation-empty-title">先从左侧选择一个会话，或者直接发送一条新消息吧</div>
        <div className="jobs-conversation-empty-body">
          主舞台不会把不同会话的 Job 强行拼成一条聊天历史。你可以先选中某个会话进入对话流，
          也可以直接在下方输入框发送消息来创建新的任务。
        </div>
        {jobs.length > 0 && (
          <div className="jobs-global-summary-list">
            {jobs.slice(0, 3).map((job) => {
              const summary = deriveJobSummary(job);
              return (
                <div key={job.job_id} className="jobs-global-summary-card">
                  <div className="jobs-global-summary-top">
                    <span className={`job-status-pill ${summary.statusTone}`}>{summary.statusLabelText}</span>
                    <span className="job-message-meta">{formatTimestamp(job.updated_at_ms)}</span>
                  </div>
                  <div className="jobs-global-summary-title">最近任务 {shortId(job.job_id)} · chat={shortId(job.chat_id ?? null)}</div>
                  <div className="jobs-global-summary-body">{summary.detail}</div>
                  <button type="button" className="job-message-link" onClick={() => onOpenJobDetails(job.job_id)}>
                    打开任务详情
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    );
  }

  const relatedJobIds = selectedJob
    ? new Set([
      selectedJob.job_id,
      ...(selectedJob.orchestration?.child_job_ids ?? []),
      ...jobs.filter((job) => job.parent_job_id === selectedJob.job_id).map((job) => job.job_id),
    ])
    : null;
  const scopedTimeline = selectedJob
    ? runtimeTimeline.filter((event) => event.trace_id === selectedJob.trace_id || relatedJobIds?.has(event.message_id))
    : selectedSession
      ? runtimeTimeline.filter((event) => event.chat_id === selectedSession.chat_id)
      : runtimeTimeline;
  const scopedProcessEntries = deriveProcessEntriesFromRuntimeTimeline(scopedTimeline).slice(-6).reverse();

  return (
    <div className="jobs-conversation-scroller">
      <div className="jobs-conversation-track">
        <div className="jobs-conversation-section-header">
          <div>
            <div className="section-title">协作痕迹 / 过程记录</div>
            <div className="jobs-conversation-section-body">这里展示主任务、子任务、worker、最近运行事件与最近输出的弱语义过程记录，不伪装成不存在的多 agent 对话。</div>
          </div>
        </div>

        {scopedProcessEntries.length > 0 && (
          <section className="jobs-runtime-panel">
            <div className="section-title">最近运行轨迹</div>
            <div className="jobs-conversation-section-body">这里按关键过程摘要展示当前范围内的 assignment → api start → upstream response → question / answer → completion 链路。</div>
            <div className="jobs-runtime-list">
              {scopedProcessEntries.map((entry) => (
                <div key={entry.id} className="jobs-runtime-row">
                  <div>
                    <div className="jobs-runtime-title">{entry.title}</div>
                    <div className="job-message-meta">{entry.detail}</div>
                    {entry.preview && <div className="jobs-runtime-preview">{entry.preview}</div>}
                  </div>
                  <div className="job-message-meta">{formatTimestamp(entry.timestamp)}</div>
                </div>
              ))}
            </div>
          </section>
        )}

        {jobs.length === 0 ? (
          <div className="jobs-conversation-empty-inline">
            当前会话还没有可展示的 Job。可以直接在下方继续发送消息创建任务。
          </div>
        ) : (
          jobs.map((job) => {
            const summary = deriveJobSummary(job);
            const fullProcessEntries = deriveProcessEntriesFromJob(job);
            const processEntries = fullProcessEntries.slice(-5).reverse();
            const decisionSummary = deriveAgentDecisionSummary(fullProcessEntries);
            const orchestration = job.orchestration ?? null;

            return (
              <div key={job.job_id} className="jobs-conversation-turn">
                <div className="job-message-row user">
                  <div className="job-message-avatar">你</div>
                  <div className="job-message-card user">
                    <div className="job-message-meta">{scopeSummary} · {formatTimestamp(job.started_at_ms)}</div>
                    <div className="job-message-text">{getUserMessage(job)}</div>
                  </div>
                </div>

                <div className="job-message-row assistant">
                  <div className="job-message-avatar assistant">迹</div>
                  <div className="job-message-card assistant">
                    <div className="job-message-header">
                      <div>
                        <div className="job-message-title">协作痕迹</div>
                        <div className="job-message-meta">job={shortId(job.job_id)} · {formatTimestamp(job.updated_at_ms)}</div>
                      </div>
                      <span className={`job-status-pill ${summary.statusTone}`}>{summary.statusLabelText}</span>
                    </div>

                    <div className="job-message-step">{summary.headline}</div>

                    <div className="job-trace-grid">
                      <span className="detail-label">层级</span>
                      <span>{job.kind === 'main' ? '主任务' : `子任务 · 属于 ${shortId(job.parent_job_id ?? null)}`}</span>
                      <span className="detail-label">当前动作</span>
                      <span>{job.current_step || summary.statusLabelText}</span>
                      <span className="detail-label">执行者</span>
                      <span>{job.worker?.worker_id ?? '-'}</span>
                      <span className="detail-label">任务类型</span>
                      <span>{job.worker?.task_type ?? '-'}</span>
                      <span className="detail-label">worker phase</span>
                      <span>{job.worker?.phase ?? '-'}</span>
                      <span className="detail-label">状态摘要</span>
                      <span>{summary.detail}</span>
                    </div>

                    {decisionSummary && (
                      <div className="job-trace-block">
                        <div className="job-trace-block-title">主 agent 决策</div>
                        <div className="job-decision-title">{decisionSummary.title}</div>
                        <div className="job-message-meta">原因：{decisionSummary.reason}</div>
                        <div className="job-message-meta">下一步：{decisionSummary.nextAction}</div>
                      </div>
                    )}

                    {job.kind === 'main' && orchestration && orchestration.total_subtasks > 0 && (
                      <div className="job-trace-block">
                        <div className="job-trace-block-title">Orchestration 概览</div>
                        <div className="job-message-meta">
                          子任务 {orchestration.completed_subtasks}/{orchestration.total_subtasks}
                          {formatActiveSubtask(orchestration.active_subtask_name, orchestration.active_subtask_type)
                            ? ` · 当前 ${formatActiveSubtask(orchestration.active_subtask_name, orchestration.active_subtask_type)}`
                            : ''}
                        </div>
                      </div>
                    )}

                    {summary.hasResponse && (
                      <div className="job-trace-block">
                        <div className="job-trace-block-title">子任务结果 / 最近输出</div>
                        <div className="job-message-text job-trace-text">{summary.latestResponseText}</div>
                      </div>
                    )}

                    {job.pending_question && (
                      <div className="job-trace-block">
                        <div className="job-trace-block-title">待处理问题</div>
                        <div className="job-status-explanation">{job.pending_question.prompt}</div>
                        <div className="job-message-meta">
                          {job.pending_question.answer_summary
                            ? `最近回答：${job.pending_question.answer_summary}`
                            : '尚未提交回答；唯一回答入口仍在任务详情区。'}
                        </div>
                      </div>
                    )}

                    {!job.pending_question && job.tool_state && (
                      <div className="job-trace-block">
                        <div className="job-trace-block-title">工具执行状态</div>
                        <div className="job-message-meta">
                          {job.tool_state.tool_name ?? '工具'} · {job.tool_state.phase}
                          {job.tool_state.permission_state ? ` · ${job.tool_state.permission_state}` : ''}
                        </div>
                        {job.tool_state.result_preview && <div className="job-message-meta">结果：{job.tool_state.result_preview}</div>}
                        {!job.tool_state.result_preview && job.tool_state.error && <div className="job-message-meta">错误：{job.tool_state.error}</div>}
                      </div>
                    )}

                    {summary.isAutoContinuing && (
                      <div className="job-status-explanation">
                        当前存在自动继续痕迹：主 agent 已安排 requirement follow-up，正在继续推进。
                      </div>
                    )}

                    {summary.isStalledWithoutQuestion && (
                      <div className="job-status-explanation">
                        当前仅显示停滞痕迹，没有第二套回答表单，也没有结构化问题可提交。
                      </div>
                    )}

                    <div className="job-trace-block">
                      <div className="job-trace-block-title">关键过程摘要</div>
                      {processEntries.length === 0 ? (
                        <div className="jobs-empty">暂无最近过程</div>
                      ) : (
                        <div className="job-events compact">
                          {processEntries.map((entry) => (
                            <div key={entry.id} className="job-event-row trace-event-row">
                              <div>
                                <div>{entry.title}</div>
                                <div className="job-message-meta">{entry.detail}</div>
                                {entry.preview && <div className="job-message-meta">{entry.preview}</div>}
                              </div>
                              <span>{formatTimestamp(entry.timestamp)}</span>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>

                    <div className="job-message-footer">
                      <span className="job-message-meta">chat={shortId(job.chat_id ?? null)} · trace={shortId(job.trace_id)}</span>
                      <button type="button" className="job-message-link" onClick={() => onOpenJobDetails(job.job_id)}>
                        在任务详情中查看
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
