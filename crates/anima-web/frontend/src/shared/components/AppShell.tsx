import type { ReactNode } from 'react';
import type { SseConnectionState } from '@/shared/api/events';
import styles from './shell.module.css';

interface AppShellProps {
  sidebar: ReactNode;
  children: ReactNode;
  connectionState?: SseConnectionState;
}

export function AppShell({ sidebar, children, connectionState }: AppShellProps) {
  return (
    <div className={styles['app-root']}>
      {connectionState && connectionState !== 'connected' && (
        <div className={styles[`conn-${connectionState}`]}>
          {connectionState === 'connecting' ? '连接中...' : '连接断开，正在重连...'}
        </div>
      )}
      <aside className={styles['app-sidebar']}>{sidebar}</aside>
      <main className={styles['app-main']}>{children}</main>
    </div>
  );
}
