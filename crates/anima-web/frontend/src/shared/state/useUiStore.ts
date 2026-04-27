import { create } from 'zustand';

export type WorkbenchScope = 'global' | 'session' | 'job';
export type Theme = 'pink' | 'blue' | 'green' | 'dark';

const THEME_KEY = 'anima-theme';

function getInitialTheme(): Theme {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved === 'pink' || saved === 'blue' || saved === 'green' || saved === 'dark') return saved;
  return 'pink';
}

function applyTheme(theme: Theme) {
  document.documentElement.setAttribute('data-theme', theme);
  localStorage.setItem(THEME_KEY, theme);
}

interface UiState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  selectedScope: WorkbenchScope;
  setSelectedScope: (scope: WorkbenchScope) => void;
  selectedSessionId: string | null;
  setSelectedSessionId: (sessionId: string | null) => void;
  selectedJobId: string | null;
  setSelectedJobId: (jobId: string | null) => void;
  selectNewestJob: boolean;
  setSelectNewestJob: (value: boolean) => void;
}

const initialTheme = getInitialTheme();
applyTheme(initialTheme);

export const useUiStore = create<UiState>((set) => ({
  theme: initialTheme,
  setTheme: (theme) => { applyTheme(theme); set({ theme }); },
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
}));
