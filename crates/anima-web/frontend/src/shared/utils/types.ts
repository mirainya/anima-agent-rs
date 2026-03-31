import { z } from 'zod';

export const jobStatusSchema = z.enum([
  'queued',
  'preparing_context',
  'creating_session',
  'planning',
  'executing',
  'waiting_upstream_input',
  'completed',
  'failed',
]);

export const jobEventSchema = z.object({
  event: z.string(),
  recorded_at_ms: z.number(),
  payload: z.unknown(),
});

export const jobReviewSchema = z.object({
  verdict: z.enum(['accepted', 'rejected']),
  reason: z.string().nullable().optional(),
  note: z.string().nullable().optional(),
  reviewed_at_ms: z.number(),
});

export const workerTaskSchema = z.object({
  worker_id: z.string(),
  status: z.string(),
  task_id: z.string(),
  trace_id: z.string(),
  task_type: z.string(),
  elapsed_ms: z.number(),
  content_preview: z.string(),
});

export const jobViewSchema = z.object({
  job_id: z.string(),
  trace_id: z.string(),
  message_id: z.string(),
  channel: z.string(),
  chat_id: z.string().nullable().optional(),
  sender_id: z.string(),
  user_content: z.string().nullable().optional(),
  status: jobStatusSchema,
  status_label: z.string(),
  accepted: z.boolean(),
  started_at_ms: z.number(),
  updated_at_ms: z.number(),
  elapsed_ms: z.number(),
  current_step: z.string(),
  recent_events: z.array(jobEventSchema),
  worker: workerTaskSchema.nullable().optional(),
  execution_summary: z.unknown().nullable().optional(),
  failure: z.unknown().nullable().optional(),
  review: jobReviewSchema.nullable().optional(),
});

export const jobsResponseSchema = z.object({
  ok: z.boolean(),
  jobs: z.array(jobViewSchema),
});

export const statusSnapshotSchema = z.object({
  agent: z.object({
    running: z.boolean(),
    status: z.string(),
    context_status: z.string(),
    sessions_count: z.number(),
    cache_entries: z.number(),
  }),
  workers: z.array(z.object({
    id: z.string(),
    status: z.string(),
    metrics: z.object({
      tasks_completed: z.number(),
      errors: z.number(),
      timeouts: z.number(),
      total_duration_ms: z.number(),
    }),
    current_task: z.object({
      task_id: z.string(),
      trace_id: z.string(),
      task_type: z.string(),
      elapsed_ms: z.number(),
      content_preview: z.string(),
    }).optional(),
  })),
  worker_pool: z.object({
    status: z.string(),
    size: z.number(),
    active: z.number(),
    idle: z.number(),
    stopped: z.number(),
  }),
  recent_sessions: z.array(z.object({
    chat_id: z.string(),
    channel: z.string(),
    session_id: z.string().nullable().optional(),
    history_len: z.number(),
    last_user_message_preview: z.string(),
  })),
  failures: z.object({
    last_failure: z.object({
      error_code: z.string(),
      error_stage: z.string(),
      message_id: z.string(),
      channel: z.string(),
      chat_id: z.string().nullable().optional(),
      occurred_at_ms: z.number(),
      internal_message: z.string(),
    }).nullable(),
    counts_by_error_code: z.record(z.string(), z.number()),
  }),
  runtime_timeline: z.array(z.object({
    event: z.string(),
    trace_id: z.string(),
    message_id: z.string(),
    channel: z.string(),
    chat_id: z.string().nullable().optional(),
    sender_id: z.string(),
    recorded_at_ms: z.number(),
    payload: z.unknown(),
  })),
  recent_execution_summaries: z.array(z.object({
    trace_id: z.string(),
    message_id: z.string(),
    channel: z.string(),
    chat_id: z.string().nullable().optional(),
    plan_type: z.string(),
    status: z.string(),
    cache_hit: z.boolean(),
    worker_id: z.string().nullable().optional(),
    error_code: z.string().nullable().optional(),
    error_stage: z.string().nullable().optional(),
    task_duration_ms: z.number(),
    stages: z.object({
      context_ms: z.number(),
      session_ms: z.number(),
      classify_ms: z.number(),
      execute_ms: z.number(),
      total_ms: z.number(),
    }),
  })),
  metrics: z.object({
    counters: z.record(z.string(), z.number()),
    gauges: z.record(z.string(), z.number()),
    histograms: z.record(z.string(), z.unknown()),
  }),
  jobs: z.array(jobViewSchema),
});

export const sseMessageEventSchema = z.object({
  type: z.literal('message'),
  content: z.string(),
  stage: z.string(),
  target: z.string(),
});

export const sseWorkerStatusEventSchema = z.object({
  type: z.literal('worker_status'),
  worker_id: z.string(),
  status: z.string(),
  task_type: z.string().nullable().optional(),
});

export const sseRuntimeEventSchema = z.object({
  type: z.literal('runtime_event'),
  event: z.string(),
  message_id: z.string(),
  channel: z.string(),
  chat_id: z.string().nullable().optional(),
  sender_id: z.string(),
  trace_id: z.string(),
  payload: z.unknown(),
});

export const sseMetricsEventSchema = z.object({
  type: z.literal('metrics'),
  data: z.unknown(),
});

export const sseEventSchema = z.union([
  sseMessageEventSchema,
  sseWorkerStatusEventSchema,
  sseRuntimeEventSchema,
  sseMetricsEventSchema,
]);

export type JobStatus = z.infer<typeof jobStatusSchema>;
export type JobView = z.infer<typeof jobViewSchema>;
export type StatusSnapshot = z.infer<typeof statusSnapshotSchema>;
export type SseEvent = z.infer<typeof sseEventSchema>;
