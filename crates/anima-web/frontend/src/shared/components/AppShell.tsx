import type { ReactNode } from 'react';
import styles from './shell.module.css';

interface AppShellProps {
  header: ReactNode;
  sidebar: ReactNode;
  rightPanel: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  isRightPanelOpen?: boolean;
}

export function AppShell({ header, sidebar, rightPanel, footer, children, isRightPanelOpen = true }: AppShellProps) {
  return (
    <div className={`${styles.appRoot} ${footer ? styles.withFooter : styles.withoutFooter} ${isRightPanelOpen ? styles.inspectorOpen : styles.inspectorClosed}`}>
      <aside className={styles.sidebar}>{sidebar}</aside>
      <header className={styles.header}>{header}</header>
      <main className={styles.main}>{children}</main>
      <aside className={styles.right}>{rightPanel}</aside>
      {footer ? <footer className={styles.footer}>{footer}</footer> : null}
    </div>
  );
}
