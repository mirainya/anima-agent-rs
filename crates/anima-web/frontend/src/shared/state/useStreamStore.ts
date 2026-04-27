import { create } from 'zustand';

export interface StreamBlock {
  kind: string;
  text: string;
  active: boolean;
}

interface JobStream {
  blocks: Map<number, StreamBlock>;
}

interface StreamState {
  jobs: Map<string, JobStream>;
  version: number;
  appendDelta: (jobId: string, index: number, kind: string, delta: string) => void;
  setBlockPhase: (jobId: string, index: number, kind: string, phase: string) => void;
  clearJob: (jobId: string) => void;
}

let pendingUpdates: Array<(jobs: Map<string, JobStream>) => void> = [];
let rafScheduled = false;

function scheduleFlush(set: (fn: (s: StreamState) => Partial<StreamState>) => void) {
  if (rafScheduled) return;
  rafScheduled = true;
  requestAnimationFrame(() => {
    rafScheduled = false;
    const batch = pendingUpdates;
    pendingUpdates = [];
    if (batch.length === 0) return;
    set((state) => {
      const jobs = new Map(state.jobs);
      for (const fn of batch) fn(jobs);
      return { jobs, version: state.version + 1 };
    });
  });
}

export const useStreamStore = create<StreamState>((set) => ({
  jobs: new Map(),
  version: 0,

  appendDelta: (jobId, index, kind, delta) => {
    pendingUpdates.push((jobs) => {
      const job = jobs.get(jobId) ?? { blocks: new Map() };
      const blocks = new Map(job.blocks);
      const block = blocks.get(index) ?? { kind, text: '', active: true };
      blocks.set(index, { ...block, text: block.text + delta });
      jobs.set(jobId, { blocks });
    });
    scheduleFlush(set);
  },

  setBlockPhase: (jobId, index, kind, phase) => {
    pendingUpdates.push((jobs) => {
      const job = jobs.get(jobId) ?? { blocks: new Map() };
      const blocks = new Map(job.blocks);
      if (phase === 'started') {
        blocks.set(index, { kind, text: '', active: true });
      } else {
        const existing = blocks.get(index);
        if (existing) blocks.set(index, { ...existing, active: false });
      }
      jobs.set(jobId, { blocks });
    });
    scheduleFlush(set);
  },

  clearJob: (jobId) => {
    pendingUpdates.push((jobs) => { jobs.delete(jobId); });
    scheduleFlush(set);
  },
}));
