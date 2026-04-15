import { useMemo } from 'react';
import { useJobsQuery } from '@/shared/api/jobs';
import { useSessionsQuery } from '@/shared/api/sessions';
import { useUiStore } from '@/shared/state/useUiStore';
import { shortId } from '@/shared/utils/format';
import { getWorkbenchContext } from '@/shared/utils/workbench';
import './sessions.css';

interface RailItem {
  id: string;
  label: string;
  icon: string;
  active: boolean;
  onClick?: () => void;
}

export function SessionSidebar() {
  const { data: sessions = [], isLoading } = useSessionsQuery();
  const { data: jobs = [] } = useJobsQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const isInspectorOpen = useUiStore((state) => state.isInspectorOpen);
  const isJobsDrawerOpen = useUiStore((state) => state.isJobsDrawerOpen);
  const setSelectedSessionId = useUiStore((state) => state.setSelectedSessionId);
  const setIsInspectorOpen = useUiStore((state) => state.setIsInspectorOpen);
  const setIsJobsDrawerOpen = useUiStore((state) => state.setIsJobsDrawerOpen);
  const context = getWorkbenchContext(sessions, jobs, selectedSessionId, selectedJobId);

  const railItems = useMemo<RailItem[]>(() => [
    {
      id: 'workbench',
      label: '工作台',
      icon: '⌘',
      active: true,
      onClick: () => setSelectedSessionId(selectedSessionId),
    },
    {
      id: 'sessions',
      label: '会话',
      icon: '◉',
      active: selectedSessionId !== null,
      onClick: () => {
        if (selectedSessionId === null && sessions[0]) {
          setSelectedSessionId(sessions[0].chat_id);
        }
      },
    },
    {
      id: 'jobs',
      label: '任务',
      icon: '▣',
      active: isJobsDrawerOpen,
      onClick: () => setIsJobsDrawerOpen(!isJobsDrawerOpen),
    },
    {
      id: 'inspector',
      label: '诊断',
      icon: '◫',
      active: isInspectorOpen,
      onClick: () => setIsInspectorOpen(!isInspectorOpen),
    },
    {
      id: 'history',
      label: '历史',
      icon: '◌',
      active: false,
    },
  ], [isInspectorOpen, isJobsDrawerOpen, selectedSessionId, sessions, setIsInspectorOpen, setIsJobsDrawerOpen, setSelectedSessionId]);

  return (
    <div className="session-sidebar">
      <div className="session-rail-brand">A</div>

      <div className="session-rail-icons" role="tablist" aria-label="Workbench rail">
        {railItems.map((item) => (
          <button
            key={item.id}
            type="button"
            className={`session-rail-icon ${item.active ? 'active' : ''}`}
            onClick={item.onClick}
            title={item.label}
          >
            <span>{item.icon}</span>
          </button>
        ))}
      </div>

      <div className="session-rail-bottom">
        <div className="session-rail-divider" />
        <div className="session-rail-theme">◐</div>
      </div>

      <div className="session-rail-flyout">
        <div className="sidebar-section-title">会话视角</div>
        <button
          type="button"
          className={`session-react-item ${selectedSessionId === null ? 'active' : ''}`}
          onClick={() => setSelectedSessionId(null)}
        >
          <div className="session-react-header">
            <div className="session-react-title">全部会话</div>
            <div className="session-react-count">{jobs.length}</div>
          </div>
          <div className="session-react-preview">查看全局 Jobs、分配情况与系统摘要</div>
        </button>

        {isLoading ? (
          <div className="sidebar-placeholder">正在加载会话...</div>
        ) : sessions.length === 0 ? (
          <div className="sidebar-placeholder">暂无会话</div>
        ) : (
          <div className="session-list-react">
            {sessions.map((session) => {
              const sessionKey = session.chat_id;
              const active = sessionKey === selectedSessionId;
              const jobCount = jobs.filter((job) => job.chat_id === session.chat_id).length;
              return (
                <button
                  key={sessionKey}
                  type="button"
                  className={`session-react-item ${active ? 'active' : ''}`}
                  onClick={() => setSelectedSessionId(sessionKey)}
                >
                  <div className="session-react-header">
                    <div className="session-react-title">{shortId(session.chat_id)}</div>
                    <div className="session-react-count">{jobCount}</div>
                  </div>
                  <div className="session-react-preview">{session.last_user_message_preview || '暂无消息'}</div>
                </button>
              );
            })}
          </div>
        )}

        <div className="sidebar-section-title session-sidebar-summary-title">当前视角</div>
        <div className="session-sidebar-summary">
          <div className="session-sidebar-line">scope={context.scope}</div>
          <div className="session-sidebar-line">session={context.selectedSession ? shortId(context.selectedSession.session_id) : '全部'}</div>
          <div className="session-sidebar-line">job={context.selectedJob ? shortId(context.selectedJob.job_id) : '未选中'}</div>
        </div>
      </div>
    </div>
  );
}
