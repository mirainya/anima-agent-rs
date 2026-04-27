import type { ReactNode } from 'react';
import styles from './shell.module.css';

interface AppShellProps {
  sidebar: ReactNode;
  children: ReactNode;
}

export function AppShell({ sidebar, children }: AppShellProps) {
  return (
    <div className={styles['app-root']}>
      <aside className={styles['app-sidebar']}>{sidebar}</aside>
      <main className={styles['app-main']}>{children}</main>
    </div>
  );
}
