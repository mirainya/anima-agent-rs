import { z } from 'zod';

export const jobStatusSchema = z.enum([
  'queued',
  'preparing_context',
  'creating_session',
  'planning',
  'executing',
  'waiting_user_input',
  'stalled',
  'completed',
  'failed',
]);

const workerTaskAssignedPayloadSchema = z.object({
  task_id: z.string().optional(),
  task_type: z.string().optional(),
  execution_kind: z.string().optional(),
  task_summary: z.string().optional(),
  task_preview: z.string().nullable().optional(),
  opencode_session_id: z.string().nullable().optional(),
  question_id: z.string().optional(),
}).passthrough();

const apiCallStartedPayloadSchema = z.object({
  task_id: z.string().optional(),
  task_type: z.string().optional(),
  execution_kind: z.string().optional(),
  request_preview: z.string().nullable().optional(),
  opencode_session_id: z.string().nullable().optional(),
  question_id: z.string().optional(),
}).passthrough();

const upstreamResponseObservedPayloadSchema = z.object({
  worker_id: z.string().nullable().optional(),
  task_type: z.string().optional(),
  provider: z.string().nullable().optional(),
  operation: z.string().nullable().optional(),
  opencode_session_id: z.string().nullable().optional(),
  response_preview: z.string().optional(),
  raw_result: z.unknown().optional(),
}).passthrough();

const questionAskedPayloadSchema = z.object({
  question_id: z.string(),
  prompt: z.string().optional(),
  options: z.array(z.string()).optional(),
  raw_question: z.unknown().optional(),
  opencode_session_id: z.string().nullable().optional(),
  requires_user_confirmation: z.boolean().optional(),
}).passthrough();

const questionAnswerSubmittedPayloadSchema = z.object({
  question_id: z.string(),
  answer: z.string().optional(),
  answer_summary: z.string().nullable().optional(),
  resolution_source: z.string().nullable().optional(),
  opencode_session_id: z.string().nullable().optional(),
}).passthrough();

const questionResolvedPayloadSchema = z.object({
  question_id: z.string(),
  answer_summary: z.string().nullable().optional(),
  resolution_source: z.string().nullable().optional(),
  opencode_session_id: z.string().nullable().optional(),
}).passthrough();

export const jobEventSchema = z.union([
  z.object({
    event: z.literal('worker_task_assigned'),
    recorded_at_ms: z.number(),
    payload: workerTaskAssignedPayloadSchema,
  }),
  z.object({
    event: z.literal('api_call_started'),
    recorded_at_ms: z.number(),
    payload: apiCallStartedPayloadSchema,
  }),
  z.object({
    event: z.literal('upstream_response_observed'),
    recorded_at_ms: z.number(),
    payload: upstreamResponseObservedPayloadSchema,
  }),
  z.object({
    event: z.literal('question_asked'),
    recorded_at_ms: z.number(),
    payload: questionAskedPayloadSchema,
  }),
  z.object({
    event: z.literal('question_answer_submitted'),
    recorded_at_ms: z.number(),
    payload: questionAnswerSubmittedPayloadSchema,
  }),
  z.object({
    event: z.literal('question_resolved'),
    recorded_at_ms: z.number(),
    payload: questionResolvedPayloadSchema,
  }),
  z.object({
    event: z.string(),
    recorded_at_ms: z.number(),
    payload: z.unknown(),
  }),
]);

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
  phase: z.string().nullable().optional(),
});

export const toolStateSchema = z.object({
  invocation_id: z.string().nullable().optional(),
  tool_name: z.string().nullable().optional(),
  tool_use_id: z.string().nullable().optional(),
  phase: z.string(),
  permission_state: z.string().nullable().optional(),
  invocation_status: z.string(),
  status_text: z.string(),
  input_preview: z.string().nullable().optional(),
  result_preview: z.string().nullable().optional(),
  error: z.string().nullable().optional(),
  awaits_user_confirmation: z.boolean(),
});

const toolPermissionRawQuestionSchema = z.object({
  type: z.literal('tool_permission'),
  tool_name: z.string().optional(),
  tool_use_id: z.string().optional(),
  tool_input: z.unknown().optional(),
  prompt: z.string().optional(),
  input_preview: z.string().optional(),
}).passthrough();

export const pendingQuestionSchema = z.object({
  question_id: z.string(),
  question_kind: z.string(),
  prompt: z.string(),
  options: z.array(z.string()),
  raw_question: z.union([toolPermissionRawQuestionSchema, z.unknown()]),
  decision_mode: z.string(),
  risk_level: z.string(),
  requires_user_confirmation: z.boolean(),
  opencode_session_id: z.string().nullable().optional(),
  answer_summary: z.string().nullable().optional(),
  resolution_source: z.string().nullable().optional(),
});

const orchestrationSchema = z.object({
  plan_id: z.string().nullable().optional(),
  active_subtask_name: z.string().nullable().optional(),
  active_subtask_type: z.string().nullable().optional(),
  active_subtask_id: z.string().nullable().optional(),
  total_subtasks: z.number(),
  active_subtasks: z.number(),
  completed_subtasks: z.number(),
  failed_subtasks: z.number(),
  child_job_ids: z.array(z.string()),
});

const executionSummarySchema = z.object({
  plan_type: z.string(),
  status: z.string(),
  cache_hit: z.boolean(),
  worker_id: z.string().nullable().optional(),
  error_code: z.string().nullable().optional(),
  error_stage: z.string().nullable().optional(),
  task_duration_ms: z.number(),
  stages: z.unknown(),
});

const failureSchema = z.object({
  error_code: z.string(),
  error_stage: z.string(),
  message_id: z.string(),
  channel: z.string(),
  chat_id: z.string().nullable().optional(),
  occurred_at_ms: z.number(),
  internal_message: z.string(),
});

export const jobViewSchema = z.object({
  job_id: z.string(),
  trace_id: z.string(),
  message_id: z.string(),
  kind: z.enum(['main', 'subtask']),
  parent_job_id: z.string().nullable().optional(),
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
  pending_question: pendingQuestionSchema.nullable().optional(),
  recent_events: z.array(jobEventSchema),
  worker: workerTaskSchema.nullable().optional(),
  tool_state: toolStateSchema.nullable().optional(),
  execution_summary: executionSummarySchema.nullable().optional(),
  failure: failureSchema.nullable().optional(),
  review: jobReviewSchema.nullable().optional(),
  orchestration: orchestrationSchema.nullable().optional(),
});

export const jobsResponseSchema = z.object({
  ok: z.boolean(),
  jobs: z.array(jobViewSchema),
});

export const sessionSummarySchema = z.object({
  chat_id: z.string(),
  channel: z.string(),
  session_id: z.string(),
  history_len: z.number(),
  last_user_message_preview: z.string(),
  last_active: z.number(),
});

export const sessionHistoryItemSchema = z.object({
  role: z.string().nullable().optional(),
  content: z.unknown(),
  recorded_at: z.number().nullable().optional(),
  raw: z.unknown(),
});

export const sessionsResponseSchema = z.object({
  ok: z.boolean(),
  sessions: z.array(sessionSummarySchema),
});

export const sessionHistoryResponseSchema = z.object({
  ok: z.boolean(),
  session_id: z.string(),
  history: z.array(sessionHistoryItemSchema),
});

const unifiedRuntimeSchema = z.object({
  runs: z.array(z.unknown()),
  turns: z.array(z.unknown()),
  tasks: z.array(z.unknown()),
  suspensions: z.array(z.unknown()),
  tool_invocations: z.array(z.unknown()),
  requirements: z.array(z.unknown()),
  transcript: z.array(z.unknown()),
  execution_summaries: z.record(z.string(), z.unknown()),
  failures: z.record(z.string(), z.unknown()),
  orchestration: z.record(z.string(), z.unknown()),
  pending_questions: z.record(z.string(), z.unknown()),
  tool_states: z.record(z.string(), z.unknown()),
  job_statuses: z.record(z.string(), z.unknown()),
  recent_events: z.array(z.object({
    sequence: z.number(),
    recorded_at_ms: z.number(),
    event_type: z.string(),
  })),
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
  recent_sessions: z.array(sessionSummarySchema.omit({ last_active: true })),
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
  warnings: z.object({
    bus_overflow_active: z.boolean(),
    bus_drop_total: z.number(),
    bus_inbound_dropped_total: z.number().optional(),
    bus_outbound_dropped_total: z.number().optional(),
    bus_internal_dropped_total: z.number().optional(),
    bus_control_dropped_total: z.number().optional(),
    bus_inbound_last_drop_at_ms: z.number().optional(),
    bus_outbound_last_drop_at_ms: z.number().optional(),
    bus_internal_last_drop_at_ms: z.number().optional(),
    bus_control_last_drop_at_ms: z.number().optional(),
  }),
  unified_runtime: unifiedRuntimeSchema,
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
export type SessionSummary = z.infer<typeof sessionSummarySchema>;
export type SessionHistoryItem = z.infer<typeof sessionHistoryItemSchema>;
export type StatusSnapshot = z.infer<typeof statusSnapshotSchema>;
export type SseEvent = z.infer<typeof sseEventSchema>;
