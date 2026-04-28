import { type ReactNode, useState, useCallback } from 'react';
import type { SseConnectionState } from '@/shared/api/events';
import styles from './shell.module.css';

interface AppShellProps {
  sidebar: ReactNode;
  children: ReactNode;
  connectionState?: SseConnectionState;
}

export function AppShell({ sidebar, children, connectionState }: AppShellProps) {
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const closeSidebar = useCallback(() => setSidebarOpen(false), []);

  return (
    <div className={styles['app-root']}>
      {connectionState && connectionState !== 'connected' && (
        <div className={styles[`conn-${connectionState}`]}>
          {connectionState === 'connecting' ? '连接中...' : '连接断开，正在重连...'}
        </div>
      )}
      <button
        className={styles['mobile-menu-btn']}
        onClick={() => setSidebarOpen(true)}
        aria-label="打开菜单"
      >
        ☰
      </button>
      <aside className={`${styles['app-sidebar']} ${sidebarOpen ? styles['sidebar-open'] : ''}`}>
        <div onClick={closeSidebar}>{sidebar}</div>
      </aside>
      {sidebarOpen && (
        <div className={styles['sidebar-backdrop']} onClick={closeSidebar} />
      )}
      <main className={styles['app-main']}>{children}</main>
    </div>
  );
}
