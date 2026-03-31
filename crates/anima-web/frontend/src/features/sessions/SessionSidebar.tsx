import { useJobsQuery } from '@/shared/api/jobs';
import { useStatusQuery } from '@/shared/api/status';
import { useUiStore } from '@/shared/state/useUiStore';
import { shortId } from '@/shared/utils/format';
import { getWorkbenchContext } from '@/shared/utils/workbench';
import './sessions.css';

export function SessionSidebar() {
  const { data: status, isLoading } = useStatusQuery();
  const { data: jobs = [] } = useJobsQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const setSelectedSessionId = useUiStore((state) => state.setSelectedSessionId);
  const context = getWorkbenchContext(status, jobs, selectedSessionId, selectedJobId);
  const sessions = status?.recent_sessions ?? [];

  return (
    <div className="session-sidebar">
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
        <div className="session-sidebar-line">session={context.selectedSession ? shortId(context.selectedSession.chat_id) : '全部'}</div>
        <div className="session-sidebar-line">job={context.selectedJob ? shortId(context.selectedJob.job_id) : '未选中'}</div>
      </div>
    </div>
  );
}
