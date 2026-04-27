import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { JobView, SessionSummary } from '@/shared/utils/types';
import { formatTimestamp, shortId, formatDurationShort } from '@/shared/utils/format';
import { statusLabel, statusTone } from '@/shared/utils/jobStatus';
import { PlanApprovalPanel } from './PlanApprovalPanel';
import { useQuestionAnswerMutation, type QuestionAnswerPayload } from '@/shared/api/questionAnswer';
import { useStreamStore } from '@/shared/state/useStreamStore';
import { MarkdownContent } from '@/shared/components/MarkdownContent';
import './jobs.css';

function getUserMessage(job: JobView): string {
  return job.user_content?.trim() || '（无消息内容）';
}

interface ToolCardData {
  toolName: string;
  input?: string | null;
  result?: string | null;
  error?: string | null;
  durationMs?: number | null;
}

interface TimelineStep {
  label: string;
  time: number;
  tone: 'neutral' | 'success' | 'failed' | 'warn' | 'active';
  detail?: string | null;
  expandable?: string | null;
  toolCard?: ToolCardData | null;
}

function buildTimeline(job: JobView): TimelineStep[] {
  const steps: TimelineStep[] = [];
  const events = job.recent_events;

  for (const e of events) {
    const p = (e.payload ?? {}) as Record<string, unknown>;

    switch (e.event) {
      case 'message_received':
        steps.push({
          label: '接收任务',
          time: e.recorded_at_ms,
          tone: 'neutral',
          detail: (p.content_preview as string) || null,
        });
        break;

      case 'session_ready':
        steps.push({
          label: '会话就绪',
          time: e.recorded_at_ms,
          tone: 'neutral',
        });
        break;

      case 'plan_built': {
        const planType = (p.plan_type as string) || (p.plan_kind as string) || '未知';
        const taskCount = (p.task_count as number) ?? 0;
        const specialist = p.specialist as string | null;
        const planDesc = planType === 'single'
          ? '直接执行（单任务）'
          : `拆分为 ${taskCount} 个子任务`;
        steps.push({
          label: `决策：${planDesc}`,
          time: e.recorded_at_ms,
          tone: 'neutral',
          detail: specialist ? `专家模式：${specialist}` : null,
        });
        break;
      }

      case 'api_call_started':
        steps.push({
          label: '开始调用 AI',
          time: e.recorded_at_ms,
          tone: 'active',
          detail: (p.request_preview as string)
            ? `请求：${(p.request_preview as string).slice(0, 120)}`
            : null,
        });
        break;

      case 'worker_task_assigned': {
        const taskType = p.task_type as string;
        const execKind = p.execution_kind as string;
        steps.push({
          label: 'Worker 开始处理',
          time: e.recorded_at_ms,
          tone: 'active',
          detail: [
            taskType && `类型：${taskType}`,
            execKind && execKind !== 'initial' && `模式：${execKind}`,
          ].filter(Boolean).join(' · ') || null,
        });
        break;
      }

      case 'upstream_response_observed': {
        const preview = p.response_preview as string;
        const latency = p.message_latency_ms as number;
        steps.push({
          label: `AI 响应完成${latency ? `（${formatDurationShort(latency)}）` : ''}`,
          time: e.recorded_at_ms,
          tone: 'neutral',
          detail: preview ? extractThinkingSummary(preview) : null,
          expandable: preview || null,
        });
        break;
      }

      case 'tool_invocation_started':
        steps.push({
          label: `调用工具：${p.tool_name || '未知'}`,
          time: e.recorded_at_ms,
          tone: 'active',
          detail: (p.input_preview as string) || null,
        });
        break;

      case 'tool_result_recorded':
        steps.push({
          label: `工具返回：${p.tool_name || '未知'}`,
          time: e.recorded_at_ms,
          tone: 'neutral',
          detail: (p.result_preview as string) || null,
        });
        break;

      case 'tool_permission_requested':
        steps.push({
          label: `请求权限：${p.tool_name || '工具'}`,
          time: e.recorded_at_ms,
          tone: 'warn',
        });
        break;

      case 'question_asked':
        steps.push({
          label: '向用户提问',
          time: e.recorded_at_ms,
          tone: 'warn',
          detail: (p.prompt as string) || null,
        });
        break;

      case 'question_answer_submitted':
        steps.push({
          label: '用户已回答',
          time: e.recorded_at_ms,
          tone: 'neutral',
          detail: (p.answer_summary as string) || null,
        });
        break;

      case 'requirement_evaluation_started':
        steps.push({
          label: '评估任务完成度',
          time: e.recorded_at_ms,
          tone: 'neutral',
        });
        break;

      case 'requirement_satisfied': {
        const preview = p.response_preview as string;
        steps.push({
          label: '任务需求已满足',
          time: e.recorded_at_ms,
          tone: 'success',
          detail: preview ? extractThinkingSummary(preview) : null,
          expandable: preview || null,
        });
        break;
      }

      case 'requirement_followup_scheduled': {
        const reason = p.reason as string;
        const missing = p.missing_requirements as string[];
        steps.push({
          label: '需求未满足，自动追加处理',
          time: e.recorded_at_ms,
          tone: 'warn',
          detail: reason || null,
          expandable: missing?.length ? missing.join('\n') : null,
        });
        break;
      }

      case 'plan_proposed':
        steps.push({
          label: `执行计划待审批（${p.task_count ?? '?'} 个任务）`,
          time: e.recorded_at_ms,
          tone: 'warn',
          detail: (p.summary as string) || null,
        });
        break;

      case 'message_completed': {
        const resp = p.response_preview as string;
        steps.push({
          label: '任务完成',
          time: e.recorded_at_ms,
          tone: 'success',
          detail: resp ? extractThinkingSummary(resp) : null,
          expandable: resp || null,
        });
        break;
      }

      case 'message_failed':
        steps.push({
          label: '任务失败',
          time: e.recorded_at_ms,
          tone: 'failed',
          detail: (p.error as string) || (p.reason as string) || null,
        });
        break;

      case 'session_create_failed':
        steps.push({
          label: '会话创建失败',
          time: e.recorded_at_ms,
          tone: 'failed',
        });
        break;

      case 'tool_execution_started':
        steps.push({
          label: `工具：${(p.tool_name as string) || '未知'}`,
          time: e.recorded_at_ms,
          tone: 'active',
          toolCard: {
            toolName: (p.tool_name as string) || '未知',
            input: p.details ? JSON.stringify((p.details as Record<string, unknown>).tool_input, null, 2) : null,
          },
        });
        break;

      case 'tool_execution_finished': {
        const startedAt = p.started_at_ms as number | undefined;
        const finishedAt = p.finished_at_ms as number | undefined;
        const dur = startedAt && finishedAt ? finishedAt - startedAt : null;
        steps.push({
          label: `工具完成：${(p.tool_name as string) || '未知'}`,
          time: e.recorded_at_ms,
          tone: 'success',
          toolCard: {
            toolName: (p.tool_name as string) || '未知',
            result: (p.result_summary as string) || null,
            durationMs: dur,
          },
        });
        break;
      }

      case 'tool_execution_failed':
        steps.push({
          label: `工具失败：${(p.tool_name as string) || '未知'}`,
          time: e.recorded_at_ms,
          tone: 'failed',
          toolCard: {
            toolName: (p.tool_name as string) || '未知',
            error: (p.error_summary as string) || (p.details as Record<string, unknown>)?.error as string || null,
          },
        });
        break;

      // Skip internal events: sdk_send_prompt_started, worker_api_call_started,
      // worker_api_call_finished, sdk_send_prompt_finished, cache_miss,
      // worker_task_cleanup_finished, etc.
    }
  }

  return steps;
}

function extractThinkingSummary(text: string): string {
  // Extract reasoning block if present: 【Reasoning】...【End...】
  const reasonMatch = text.match(/【Reasoning】\s*([\s\S]*?)(?:【End|$)/);
  if (reasonMatch) {
    const reasoning = reasonMatch[1].trim();
    return reasoning.length > 300 ? reasoning.slice(0, 300) + '...' : reasoning;
  }
  return text.length > 300 ? text.slice(0, 300) + '...' : text;
}

function Collapsible({ title, defaultOpen, children }: { title: string; defaultOpen?: boolean; children: React.ReactNode }) {
  const [open, setOpen] = useState(defaultOpen ?? false);
  return (
    <div className="chat-collapsible">
      <button type="button" className="chat-collapsible-toggle" onClick={() => setOpen(!open)}>
        <span className="chat-collapsible-arrow">{open ? '▾' : '▸'}</span>
        <span>{title}</span>
      </button>
      {open && <div className="chat-collapsible-body">{children}</div>}
    </div>
  );
}

function ExpandableText({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);
  if (text.length <= 300) return <div className="chat-timeline-detail"><MarkdownContent content={text} /></div>;
  return (
    <div className="chat-timeline-detail">
      <MarkdownContent content={expanded ? text : text.slice(0, 300) + '...'} />
      <button type="button" className="chat-expand-btn" onClick={() => setExpanded(!expanded)}>
        {expanded ? '收起' : '展开全文'}
      </button>
    </div>
  );
}

function ToolCard({ data }: { data: ToolCardData }) {
  return (
    <div className="chat-tool-card">
      <div className="chat-tool-card-name">{data.toolName}</div>
      {data.input && (
        <Collapsible title="输入">
          <pre className="chat-tool-card-pre">{data.input}</pre>
        </Collapsible>
      )}
      {data.result && (
        <Collapsible title="结果">
          <pre className="chat-tool-card-pre">{data.result}</pre>
        </Collapsible>
      )}
      {data.error && <div className="chat-tool-card-error">{data.error}</div>}
      {data.durationMs != null && (
        <div className="chat-tool-card-meta">{formatDurationShort(data.durationMs)}</div>
      )}
    </div>
  );
}

function EventTimeline({ steps }: { steps: TimelineStep[] }) {
  if (steps.length === 0) return null;

  return (
    <div className="chat-timeline">
      {steps.map((step, i) => (
        <div key={i} className={`chat-timeline-item ${step.tone}`}>
          <div className="chat-timeline-dot" />
          <div className="chat-timeline-content">
            <div className="chat-timeline-header">
              <span className="chat-timeline-label">{step.label}</span>
              <span className="chat-timeline-time">{formatTimestamp(step.time)}</span>
            </div>
            {step.detail && <div className="chat-timeline-detail">{step.detail}</div>}
            {step.toolCard && <ToolCard data={step.toolCard} />}
            {step.expandable && step.expandable !== step.detail && (
              <Collapsible title="查看完整内容">
                <ExpandableText text={step.expandable} />
              </Collapsible>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

function WorkerLiveStatus({ job }: { job: JobView }) {
  const worker = job.worker;
  if (!worker) return null;

  const phaseLabels: Record<string, string> = {
    api_call_inflight: 'AI 正在思考中...',
    api_call_finished: 'AI 响应完成',
    idle: '空闲',
    executing: '执行中',
  };

  return (
    <div className="chat-flow-step chat-worker-live">
      <div className="chat-flow-step-label">Worker 实时状态</div>
      <div className="chat-worker-info">
        <span>Worker {shortId(worker.worker_id)}</span>
        <span>{phaseLabels[worker.phase ?? ''] ?? worker.phase ?? worker.status}</span>
        <span>已用时 {formatDurationShort(worker.elapsed_ms)}</span>
      </div>
      {worker.content_preview && (
        <div className="chat-worker-task-preview">{worker.content_preview}</div>
      )}
    </div>
  );
}

function StreamingOutput({ jobId }: { jobId: string }) {
  const version = useStreamStore((s) => s.version);
  const blocks = useMemo(() => {
    const job = useStreamStore.getState().jobs.get(jobId);
    if (!job) return [];
    return Array.from(job.blocks.entries())
      .sort(([a], [b]) => a - b)
      .map(([, block]) => block);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId, version]);

  if (blocks.length === 0) return null;

  return (
    <div className="chat-streaming-output">
      {blocks.map((block, i) => {
        if (!block.text && !block.active) return null;
        const kindLabel = block.kind === 'thinking' ? '思考中' : block.kind === 'tool_input' ? '工具输入' : '回复';
        return (
          <div key={i} className={`chat-stream-block chat-stream-${block.kind}`}>
            <div className="chat-stream-label">
              {kindLabel}
              {block.active && <span className="chat-stream-cursor" />}
            </div>
            {block.kind === 'text' ? (
              <MarkdownContent content={block.text} />
            ) : (
              <div className="chat-stream-text">{block.text}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function QuestionAnswerForm({ job }: { job: JobView }) {
  const q = job.pending_question;
  if (!q || q.answer_summary) return null;

  const mutation = useQuestionAnswerMutation(job.job_id);
  const [text, setText] = useState('');
  const options = q.options ?? [];

  const submit = (answer: string, answerType: QuestionAnswerPayload['answer_type']) => {
    mutation.mutate({
      question_id: q.question_id,
      source: 'user',
      answer_type: answerType,
      answer,
    });
  };

  if (options.length > 0) {
    return (
      <div className="chat-question-actions">
        {options.map((opt, i) => (
          <button
            key={i}
            type="button"
            className="chat-question-option-btn"
            disabled={mutation.isPending}
            onClick={() => submit(opt, 'choice')}
          >
            {opt}
          </button>
        ))}
      </div>
    );
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (text.trim()) submit(text.trim(), 'text');
  };

  return (
    <form className="chat-question-input-row" onSubmit={onSubmit}>
      <input
        className="chat-question-input"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="输入回答..."
        disabled={mutation.isPending}
      />
      <button type="submit" className="chat-question-submit" disabled={mutation.isPending || !text.trim()}>
        回答
      </button>
    </form>
  );
}

function AgentResponseCard({ job, childJobs }: { job: JobView; childJobs: JobView[] }) {
  const tone = statusTone(job.status);
  const label = statusLabel(job.status);
  const orchestration = job.orchestration ?? null;
  const isRunning = ['executing', 'planning', 'preparing_context', 'creating_session'].includes(job.status);
  const timeline = buildTimeline(job);

  return (
    <div className="chat-agent-card">
      {/* Header */}
      <div className="chat-agent-header">
        <span className={`chat-status-dot ${tone}`} />
        <span className="chat-agent-label">
          {job.kind === 'main' ? 'Agent' : `子任务 ${shortId(job.job_id)}`}
        </span>
        <span className={`job-status-pill ${tone}`}>{label}</span>
        <span className="chat-meta">{formatDurationShort(job.elapsed_ms)}</span>
      </div>

      {/* Current step */}
      {job.current_step && (
        <div className="chat-flow-step">
          <div className="chat-flow-step-label">当前阶段</div>
          <div className="chat-flow-step-content">{job.current_step}</div>
        </div>
      )}

      {/* Orchestration: subtask overview */}
      {orchestration && orchestration.total_subtasks > 0 && (
        <div className="chat-flow-step">
          <div className="chat-flow-step-label">任务拆分</div>
          <div className="chat-flow-step-content">
            共 {orchestration.total_subtasks} 个子任务 ·
            完成 {orchestration.completed_subtasks} ·
            进行中 {orchestration.active_subtasks} ·
            失败 {orchestration.failed_subtasks}
          </div>
          {childJobs.length > 0 && (
            <div className="chat-subtask-list">
              {childJobs.map((child) => (
                <div key={child.job_id} className={`chat-subtask-row ${statusTone(child.status)}`}>
                  <span className={`chat-status-dot ${statusTone(child.status)}`} />
                  <span className="chat-subtask-id">{shortId(child.job_id)}</span>
                  <span className="chat-subtask-step">{child.current_step || statusLabel(child.status)}</span>
                  <span className="chat-meta">{formatDurationShort(child.elapsed_ms)}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Plan approval */}
      {job.pending_plan && <PlanApprovalPanel job={job} />}

      {/* Worker live status (only when running) */}
      {isRunning && <WorkerLiveStatus job={job} />}

      {/* Streaming output (live token display) */}
      {isRunning && <StreamingOutput jobId={job.job_id} />}

      {/* Pending question */}
      {job.pending_question && (
        <div className="chat-flow-step">
          <div className="chat-flow-step-label">等待回答</div>
          <div className="chat-question-box">
            <div className="chat-question-prompt">{job.pending_question.prompt}</div>
            {job.pending_question.answer_summary ? (
              <div className="chat-meta">已回答：{job.pending_question.answer_summary}</div>
            ) : (
              <QuestionAnswerForm job={job} />
            )}
          </div>
        </div>
      )}

      {/* Tool state (when actively using a tool) */}
      {job.tool_state && job.tool_state.phase !== 'idle' && (
        <div className="chat-flow-step">
          <div className="chat-flow-step-label">工具执行</div>
          <div className="chat-tool-state">
            <div className="chat-tool-header">
              <span className="chat-tool-name">{job.tool_state.tool_name ?? '工具'}</span>
              <span className="chat-tool-phase">{job.tool_state.status_text || job.tool_state.phase}</span>
            </div>
            {job.tool_state.input_preview && (
              <div className="chat-tool-content">输入：{job.tool_state.input_preview}</div>
            )}
            {job.tool_state.result_preview && (
              <div className="chat-tool-content">结果：{job.tool_state.result_preview}</div>
            )}
            {job.tool_state.error && (
              <div className="chat-tool-content chat-tool-error">错误：{job.tool_state.error}</div>
            )}
          </div>
        </div>
      )}

      {/* Execution timeline — the core flow */}
      {timeline.length > 0 && (
        <Collapsible title={`执行流程（${timeline.length} 步）`} defaultOpen={timeline.length <= 8}>
          <EventTimeline steps={timeline} />
        </Collapsible>
      )}
    </div>
  );
}

interface ChatMessageListProps {
  jobs: JobView[];
  selectedSession: SessionSummary | null;
  selectedSessionId: string | null;
}

export function ChatMessageList({ jobs, selectedSession, selectedSessionId }: ChatMessageListProps) {
  const endRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const userScrolledUp = useRef(false);
  const streamVersion = useStreamStore((s) => s.version);

  const onScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    userScrolledUp.current = el.scrollHeight - el.scrollTop - el.clientHeight > 100;
  }, []);

  useEffect(() => {
    if (!userScrolledUp.current) {
      endRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [jobs, streamVersion]);

  if (!selectedSessionId) {
    return (
      <div className="chat-empty-state">
        <div className="chat-empty-title">开始新对话</div>
        <div className="chat-empty-body">从左侧选择一个会话，或在下方输入消息创建新任务。</div>
      </div>
    );
  }

  const mainJobs = jobs.filter((j) => j.kind === 'main');
  const displayJobs = mainJobs.length > 0 ? mainJobs : jobs;

  if (displayJobs.length === 0) {
    return (
      <div className="chat-empty-state">
        <div className="chat-empty-title">暂无消息</div>
        <div className="chat-empty-body">在下方输入消息开始对话。</div>
      </div>
    );
  }

  return (
    <div className="chat-message-list" ref={containerRef} onScroll={onScroll}>
      {displayJobs.map((job) => {
        const childJobs = jobs.filter((j) => j.parent_job_id === job.job_id);
        return (
          <div key={job.job_id} className="chat-turn">
            <div className="chat-bubble-row user">
              <div className="chat-bubble user">{getUserMessage(job)}</div>
            </div>
            <div className="chat-bubble-row agent">
              <AgentResponseCard job={job} childJobs={childJobs} />
            </div>
            {childJobs.map((child) => (
              <div key={child.job_id} className="chat-bubble-row agent">
                <AgentResponseCard job={child} childJobs={[]} />
              </div>
            ))}
          </div>
        );
      })}
      <div ref={endRef} />
    </div>
  );
}
