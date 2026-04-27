import type { JobView } from '@/shared/utils/types';
import { formatDurationShort } from '@/shared/utils/format';
import './jobs.css';

interface WorkerActivityPanelProps {
  job: JobView;
}

export function WorkerActivityPanel({ job }: WorkerActivityPanelProps) {
  const worker = job.worker;
  if (!worker) return null;

  const busy = worker.status !== 'idle';
  const workerEvents = job.recent_events.filter(
    (e) => e.event === 'worker_task_assigned' || e.event === 'upstream_response_observed',
  );

  return (
    <div className="worker-activity-panel">
      <div className="worker-activity-header">
        <span className="worker-activity-id">{worker.worker_id}</span>
        <span className={`job-status-pill ${busy ? 'busy' : 'neutral'}`}>{worker.status}</span>
      </div>
      <div className="worker-activity-meta">
        <span>{worker.task_type}</span>
        <span>{worker.phase ?? '-'}</span>
        <span>{formatDurationShort(worker.elapsed_ms)}</span>
      </div>
      {worker.content_preview && (
        <div className="worker-activity-preview">{worker.content_preview}</div>
      )}
      {workerEvents.length > 0 && (
        <div className="worker-activity-events">
          {workerEvents.slice(-3).map((evt, i) => {
            const p = evt.payload as Record<string, unknown>;
            const label = evt.event === 'worker_task_assigned' ? '派发' : '响应';
            const preview = (p.task_preview ?? p.response_preview ?? '') as string;
            return (
              <div key={i} className="worker-activity-event-row">
                <span className="worker-activity-event-label">{label}</span>
                <span className="worker-activity-event-text">{preview || '-'}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
