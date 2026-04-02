import { JobDetail } from './JobDetail';
import { JobsList } from './JobsList';
import type { JobView, StatusSnapshot } from '@/shared/utils/types';
import { shortId } from '@/shared/utils/format';
import './jobs.css';

interface JobWorkbenchDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  jobs: JobView[];
  selectedSession: StatusSnapshot['recent_sessions'][number] | null;
  scopeSummary: string;
}

export function JobWorkbenchDrawer({ isOpen, onClose, jobs, selectedSession, scopeSummary }: JobWorkbenchDrawerProps) {
  return (
    <>
      <div className={`jobs-drawer-backdrop ${isOpen ? 'open' : ''}`} onClick={onClose} aria-hidden={!isOpen} />
      <aside className={`jobs-drawer ${isOpen ? 'open' : ''}`} aria-hidden={!isOpen}>
        <div className="jobs-drawer-header">
          <div>
            <div className="pane-title">任务面板</div>
            <div className="pane-subtitle">
              {scopeSummary} · {selectedSession ? `会话 ${shortId(selectedSession.chat_id)}` : '全部会话'} · {jobs.length} Jobs
            </div>
          </div>
          <button type="button" className="jobs-drawer-close" onClick={onClose}>
            关闭
          </button>
        </div>

        <div className="jobs-drawer-body">
          <section className="jobs-drawer-section jobs-list-pane">
            <div className="pane-header">
              <div>
                <div className="pane-title">任务列表</div>
                <div className="pane-subtitle">切换当前范围内的任务。</div>
              </div>
            </div>
            <JobsList jobs={jobs} selectedSessionChatId={selectedSession?.chat_id ?? null} />
          </section>

          <section className="jobs-drawer-section jobs-detail-pane">
            <div className="pane-header">
              <div>
                <div className="pane-title">任务详情</div>
                <div className="pane-subtitle">选中任务后在抽屉中查看详情与交互入口。</div>
              </div>
            </div>
            <JobDetail jobs={jobs} selectedSessionChatId={selectedSession?.chat_id ?? null} />
          </section>
        </div>
      </aside>
    </>
  );
}
