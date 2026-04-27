import { Outlet } from 'react-router-dom';
import { AppShell } from '@/shared/components/AppShell';
import { useEventsSync } from '@/shared/api/events';
import { SessionSidebar } from '@/features/sessions/SessionSidebar';

export function RootLayout() {
  useEventsSync();

  return (
    <AppShell sidebar={<SessionSidebar />}>
      <Outlet />
    </AppShell>
  );
}
