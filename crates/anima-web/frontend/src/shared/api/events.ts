import { useEffect, useMemo, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { sseEventSchema, type SseEvent } from '@/shared/utils/types';
import { useStreamStore } from '@/shared/state/useStreamStore';

const TERMINAL_RUNTIME_EVENTS = new Set([
  'message_completed',
  'message_failed',
  'session_create_failed',
  'plan_built',
  'session_ready',
  'message_received',
]);

export type SseConnectionState = 'connecting' | 'connected' | 'disconnected';

function shouldRefresh(event: SseEvent): boolean {
  switch (event.type) {
    case 'message':
      return true;
    case 'metrics':
      return true;
    case 'worker_status':
      return true;
    case 'runtime_event':
      return TERMINAL_RUNTIME_EVENTS.has(event.event);
    default:
      return false;
  }
}

export function useEventsSync() {
  const queryClient = useQueryClient();
  const [connectionState, setConnectionState] = useState<SseConnectionState>('connecting');
  const refreshTimerRef = useRef<number | null>(null);
  const lastRefreshAtRef = useRef(0);

  const scheduleRefresh = useMemo(() => {
    return () => {
      if (refreshTimerRef.current !== null) {
        return;
      }

      const elapsed = Date.now() - lastRefreshAtRef.current;
      const delay = elapsed >= 1000 ? 0 : 1000 - elapsed;

      refreshTimerRef.current = window.setTimeout(() => {
        refreshTimerRef.current = null;
        lastRefreshAtRef.current = Date.now();
        queryClient.invalidateQueries({ queryKey: ['status'] });
        queryClient.invalidateQueries({ queryKey: ['jobs'] });
      }, delay);
    };
  }, [queryClient]);

  useEffect(() => {
    const eventSource = new EventSource('/api/events');

    eventSource.onopen = () => {
      setConnectionState('connected');
    };

    eventSource.onmessage = (message) => {
      try {
        const raw = JSON.parse(message.data) as unknown;
        const event = sseEventSchema.parse(raw);
        if (event.type === 'stream_delta') {
          useStreamStore.getState().appendDelta(event.job_id, event.index, event.kind, event.delta);
        } else if (event.type === 'stream_block_lifecycle') {
          useStreamStore.getState().setBlockPhase(event.job_id, event.index, event.kind, event.phase);
        } else if (shouldRefresh(event)) {
          scheduleRefresh();
        }
        if (event.type === 'runtime_event' && event.event === 'message_completed') {
          const jobId = (event.payload as Record<string, unknown>)?.job_id as string | undefined;
          if (jobId) useStreamStore.getState().clearJob(jobId);
        }
      } catch {
        // 忽略无法解析的事件，避免中断 SSE 主流程
      }
    };

    eventSource.onerror = () => {
      setConnectionState('disconnected');
    };

    return () => {
      eventSource.close();
      if (refreshTimerRef.current !== null) {
        window.clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
    };
  }, [scheduleRefresh]);

  return connectionState;
}
