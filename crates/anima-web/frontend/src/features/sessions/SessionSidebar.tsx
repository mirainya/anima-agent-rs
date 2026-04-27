import { useJobsQuery } from '@/shared/api/jobs';
import { useSessionsQuery } from '@/shared/api/sessions';
import { useStatusQuery } from '@/shared/api/status';
import { useUiStore, type Theme } from '@/shared/state/useUiStore';
import { shortId, formatTimestamp } from '@/shared/utils/format';
import { ApprovalModeSelector } from '@/features/runtime/ApprovalModeSelector';
import './sessions.css';

const THEMES: { value: Theme; label: string }[] = [
  { value: 'pink', label: '🌸' },
  { value: 'blue', label: '🌊' },
  { value: 'green', label: '🌿' },
  { value: 'dark', label: '🌙' },
];

export function SessionSidebar() {
  const { data: sessions = [], isLoading } = useSessionsQuery();
  const { data: jobs = [] } = useJobsQuery();
  const { data: status } = useStatusQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const setSelectedSessionId = useUiStore((state) => state.setSelectedSessionId);
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);

  const workers = status?.workers ?? [];
  const busyCount = workers.filter((w) => w.status !== 'idle').length;
  const agentRunning = status?.agent.status === 'running';

  return (
    <div className="sidebar">
      <div className="sidebar-top">
        <div className="sidebar-brand">
          <span className="sidebar-brand-icon">🌸</span>
          <span className="sidebar-brand-text">Anima</span>
        </div>
        <button
          type="button"
          className="sidebar-new-chat-btn"
          onClick={() => setSelectedSessionId(null)}
        >
          ✨ 新对话
        </button>
      </div>

      <div className="sidebar-sessions">
        {isLoading ? (
          <div className="sidebar-placeholder">加载中...</div>
        ) : sessions.length === 0 ? (
          <div className="sidebar-placeholder">暂无会话</div>
        ) : (
          sessions.map((session) => {
            const active = session.chat_id === selectedSessionId;
            const jobCount = jobs.filter((j) => j.chat_id === session.chat_id).length;
            return (
              <button
                key={session.chat_id}
                type="button"
                className={`sidebar-session-item ${active ? 'active' : ''}`}
                onClick={() => setSelectedSessionId(session.chat_id)}
              >
                <div className="sidebar-session-title">
                  <span className="sidebar-session-icon">💬</span>
                  {session.last_user_message_preview || shortId(session.chat_id)}
                </div>
                <div className="sidebar-session-meta">
                  {shortId(session.chat_id)} · {jobCount} 任务
                  {session.last_active ? ` · ${formatTimestamp(session.last_active)}` : ''}
                </div>
              </button>
            );
          })
        )}
      </div>

      <div className="sidebar-bottom">
        <div className="sidebar-runtime-status">
          <div className="sidebar-runtime-row">
            <span className="sidebar-runtime-label">
              <span className={`sidebar-runtime-dot ${agentRunning ? 'running' : 'idle'}`} />
              Agent
            </span>
            <span>{status?.agent.status ?? '...'}</span>
          </div>
          <div className="sidebar-runtime-row">
            <span className="sidebar-runtime-label">
              <span className={`sidebar-runtime-dot ${busyCount > 0 ? 'running' : 'idle'}`} />
              Workers
            </span>
            <span>{busyCount}/{workers.length}</span>
          </div>
        </div>
        <div className="sidebar-theme-switcher">
          {THEMES.map((t) => (
            <button
              key={t.value}
              type="button"
              className={`sidebar-theme-btn ${theme === t.value ? 'active' : ''}`}
              onClick={() => setTheme(t.value)}
              title={t.value}
            >
              {t.label}
            </button>
          ))}
        </div>
        <ApprovalModeSelector />
      </div>
    </div>
  );
}
