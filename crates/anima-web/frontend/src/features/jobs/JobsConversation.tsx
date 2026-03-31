import type { JobView, StatusSnapshot } from '@/shared/utils/types';
import { formatTimestamp, shortId } from '@/shared/utils/format';
import { statusLabel, statusTone } from '@/shared/utils/jobStatus';
import './jobs.css';

interface JobsConversationProps {
  jobs: JobView[];
  selectedSession: StatusSnapshot['recent_sessions'][number] | null;
  selectedSessionId: string | null;
  scopeSummary: string;
  onOpenJobDetails: (jobId: string) => void;
}

interface JobEventPayloadView {
  response_preview?: string;
  response_text?: string;
}

function getUserMessage(job: JobView): string {
  return job.user_content?.trim() || '这条用户消息暂时没有可展示的原文。';
}

function getLatestMessage(job: JobView): JobEventPayloadView | null {
  const event = job.recent_events
    .slice()
    .reverse()
    .find((item) => item.event === 'message_completed');

  if (!event || typeof event.payload !== 'object' || event.payload === null) {
    return null;
  }

  return event.payload as JobEventPayloadView;
}

function summarizeStatus(job: JobView): string {
  if (job.status === 'failed') {
    const failure = job.failure as { error_code?: string; error_stage?: string } | null;
    if (failure?.error_code || failure?.error_stage) {
      return `失败于 ${failure.error_stage ?? '未知阶段'} / ${failure.error_code ?? '未知错误'}。`;
    }
    return '任务执行失败，建议打开任务面板查看失败细节。';
  }
  if (job.status === 'completed') {
    if (job.review?.verdict === 'accepted') return '任务已完成，用户已接受这次结果。';
    if (job.review?.verdict === 'rejected') return '任务已完成，但用户拒绝了这次结果。';
    return '任务已完成，等待用户反馈结果。';
  }
  if (job.status === 'executing') return job.current_step || '任务正在执行中。';
  if (job.status === 'planning') return job.current_step || '系统正在规划处理路径。';
  if (job.status === 'creating_session') return '正在创建上游会话。';
  if (job.status === 'preparing_context') return '正在准备上下文。';
  if (job.status === 'waiting_upstream_input') return '正在等待上游输入。';
  if (job.status === 'queued') return '任务已进入队列，等待处理。';
  return job.current_step || '状态推导中。';
}

export function JobsConversation({ jobs, selectedSession, selectedSessionId, scopeSummary, onOpenJobDetails }: JobsConversationProps) {
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
            {jobs.slice(0, 3).map((job) => (
              <div key={job.job_id} className="jobs-global-summary-card">
                <div className="jobs-global-summary-top">
                  <span className={`job-status-pill ${statusTone(job.status)}`}>{statusLabel(job.status)}</span>
                  <span className="job-message-meta">{formatTimestamp(job.updated_at_ms)}</span>
                </div>
                <div className="jobs-global-summary-title">最近任务 {shortId(job.job_id)} · chat={shortId(job.chat_id ?? null)}</div>
                <div className="jobs-global-summary-body">{summarizeStatus(job)}</div>
                <button type="button" className="job-message-link" onClick={() => onOpenJobDetails(job.job_id)}>
                  打开任务面板
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="jobs-conversation-scroller">
      <div className="jobs-conversation-track">
        {jobs.length === 0 ? (
          <div className="jobs-conversation-empty-inline">
            当前会话还没有可展示的 Job。可以直接在下方继续发送消息创建任务。
          </div>
        ) : (
          jobs.map((job) => {
            const latestMessage = getLatestMessage(job);
            const responseText = latestMessage?.response_text ?? latestMessage?.response_preview ?? '';
            const hasResponse = Boolean(responseText);

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
                  <div className="job-message-avatar assistant">AI</div>
                  <div className={`job-message-card ${hasResponse ? 'assistant' : 'system'}`}>
                    <div className="job-message-header">
                      <div>
                        <div className="job-message-title">
                          {hasResponse ? 'Assistant 回复' : '系统状态'}
                        </div>
                        <div className="job-message-meta">
                          job={shortId(job.job_id)} · {formatTimestamp(job.updated_at_ms)}
                        </div>
                      </div>
                      <span className={`job-status-pill ${statusTone(job.status)}`}>{statusLabel(job.status)}</span>
                    </div>

                    <div className="job-message-step">{job.current_step}</div>
                    <div className="job-message-text">{hasResponse ? responseText : summarizeStatus(job)}</div>

                    <div className="job-message-footer">
                      <span className="job-message-meta">chat={shortId(job.chat_id ?? null)} · trace={shortId(job.trace_id)}</span>
                      <button type="button" className="job-message-link" onClick={() => onOpenJobDetails(job.job_id)}>
                        查看任务详情
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
