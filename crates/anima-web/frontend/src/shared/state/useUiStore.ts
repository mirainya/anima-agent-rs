import { create } from 'zustand';

export type WorkbenchScope = 'global' | 'session' | 'job';
export type InspectorTab = 'overview' | 'assignments' | 'timeline' | 'failure' | 'execution';
export type JobListFilter = 'all' | 'active' | 'review' | 'failed';

interface UiState {
  selectedScope: WorkbenchScope;
  setSelectedScope: (scope: WorkbenchScope) => void;
  selectedSessionId: string | null;
  setSelectedSessionId: (sessionId: string | null) => void;
  selectedJobId: string | null;
  setSelectedJobId: (jobId: string | null) => void;
  selectNewestJob: boolean;
  setSelectNewestJob: (value: boolean) => void;
  activeInspectorTab: InspectorTab;
  setActiveInspectorTab: (tab: InspectorTab) => void;
  isInspectorOpen: boolean;
  setIsInspectorOpen: (open: boolean) => void;
  toggleInspector: () => void;
  isJobsDrawerOpen: boolean;
  setIsJobsDrawerOpen: (open: boolean) => void;
  toggleJobsDrawer: () => void;
  jobListFilter: JobListFilter;
  setJobListFilter: (filter: JobListFilter) => void;
}

export const useUiStore = create<UiState>((set) => ({
  selectedScope: 'global',
  setSelectedScope: (selectedScope) => set({ selectedScope }),
  selectedSessionId: null,
  setSelectedSessionId: (selectedSessionId) => set({
    selectedSessionId,
    selectedJobId: null,
    selectNewestJob: true,
    selectedScope: selectedSessionId ? 'session' : 'global',
  }),
  selectedJobId: null,
  setSelectedJobId: (selectedJobId) => set((state) => ({
    selectedJobId,
    selectNewestJob: !selectedJobId,
    selectedScope: selectedJobId ? 'job' : state.selectedSessionId ? 'session' : 'global',
  })),
  selectNewestJob: true,
  setSelectNewestJob: (selectNewestJob) => set({ selectNewestJob }),
  activeInspectorTab: 'overview',
  setActiveInspectorTab: (activeInspectorTab) => set({ activeInspectorTab }),
  isInspectorOpen: true,
  setIsInspectorOpen: (isInspectorOpen) => set({ isInspectorOpen }),
  toggleInspector: () => set((state) => ({ isInspectorOpen: !state.isInspectorOpen })),
  isJobsDrawerOpen: false,
  setIsJobsDrawerOpen: (isJobsDrawerOpen) => set({ isJobsDrawerOpen }),
  toggleJobsDrawer: () => set((state) => ({ isJobsDrawerOpen: !state.isJobsDrawerOpen })),
  jobListFilter: 'all',
  setJobListFilter: (jobListFilter) => set({ jobListFilter }),
}));
