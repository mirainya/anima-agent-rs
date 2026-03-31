import { Outlet } from 'react-router-dom';
import { AppShell } from '@/shared/components/AppShell';
import { useEventsSync } from '@/shared/api/events';
import { RuntimeHeader } from '@/features/runtime/RuntimeHeader';
import { SessionSidebar } from '@/features/sessions/SessionSidebar';
import { DiagnosticsInspector } from '@/features/diagnostics/DiagnosticsInspector';
import { useUiStore } from '@/shared/state/useUiStore';

export function RootLayout() {
  const sseState = useEventsSync();
  const isInspectorOpen = useUiStore((state) => state.isInspectorOpen);

  return (
    <AppShell
      header={<RuntimeHeader sseState={sseState} />}
      sidebar={<SessionSidebar />}
      rightPanel={<DiagnosticsInspector />}
      isRightPanelOpen={isInspectorOpen}
    >
      <Outlet />
    </AppShell>
  );
}
