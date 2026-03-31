import { MessageComposer } from '@/features/chat/MessageComposer';
import { JobWorkbenchDrawer } from '@/features/jobs/JobWorkbenchDrawer';
import { JobsConversation } from '@/features/jobs/JobsConversation';
import { useJobsQuery } from '@/shared/api/jobs';
import { useStatusQuery } from '@/shared/api/status';
import { useUiStore } from '@/shared/state/useUiStore';
import { shortId } from '@/shared/utils/format';
import { getWorkbenchContext } from '@/shared/utils/workbench';

export function JobsPage() {
  const { data: jobs = [] } = useJobsQuery();
  const { data: status } = useStatusQuery();
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedJobId = useUiStore((state) => state.selectedJobId);
  const jobListFilter = useUiStore((state) => state.jobListFilter);
  const setJobListFilter = useUiStore((state) => state.setJobListFilter);
  const setSelectedJobId = useUiStore((state) => state.setSelectedJobId);
  const isJobsDrawerOpen = useUiStore((state) => state.isJobsDrawerOpen);
  const setIsJobsDrawerOpen = useUiStore((state) => state.setIsJobsDrawerOpen);
  const context = getWorkbenchContext(status, jobs, selectedSessionId, selectedJobId);
  const selectedScope = context.scope;
  const selectedSession = context.selectedSession;
  const sessionJobs = context.sessionJobs;
  const visibleJobs = sessionJobs.filter((job) => {
    switch (jobListFilter) {
      case 'active':
        return !['completed', 'failed'].includes(job.status);
      case 'review':
        return job.status === 'completed' && !job.review;
      case 'failed':
        return job.status === 'failed';
      default:
        return true;
    }
  }).slice().sort((a, b) => a.updated_at_ms - b.updated_at_ms);

  const scopeSummary = selectedScope === 'job'
    ? '当前 Job'
    : selectedScope === 'session'
      ? `当前会话 ${shortId(selectedSession?.chat_id ?? null)}`
      : '全部会话';

  const openJobDetails = (jobId: string) => {
    setSelectedJobId(jobId);
    setIsJobsDrawerOpen(true);
  };

  return (
    <div className="jobs-page">
      <div className="jobs-toolbar jobs-context-bar">
        <div>
          <div className="jobs-toolbar-title">对话主舞台</div>
          <div className="jobs-toolbar-subtitle">
            {selectedSession
              ? `聚焦会话 ${shortId(selectedSession.chat_id)} · 最近输入：${selectedSession.last_user_message_preview || '暂无消息'}`
              : '当前为全局视角，请从左侧选择会话，或直接发送消息创建新任务。'}
          </div>
        </div>
        <div className="jobs-toolbar-actions jobs-context-actions">
          <div className="jobs-toolbar-meta">{scopeSummary} · {visibleJobs.length}/{sessionJobs.length} Jobs</div>
          <div className="jobs-filter-group" role="tablist" aria-label="Job 过滤器">
            {[
              { key: 'all', label: '全部' },
              { key: 'active', label: '进行中' },
              { key: 'review', label: '待确认' },
              { key: 'failed', label: '失败' },
            ].map((filter) => (
              <button
                key={filter.key}
                type="button"
                className={`jobs-filter-chip ${jobListFilter === filter.key ? 'active' : ''}`}
                onClick={() => setJobListFilter(filter.key as typeof jobListFilter)}
              >
                {filter.label}
              </button>
            ))}
          </div>
          <button type="button" className="jobs-drawer-trigger" onClick={() => setIsJobsDrawerOpen(true)}>
            打开任务面板
          </button>
        </div>
      </div>

      <div className="jobs-main jobs-conversation-stage">
        <JobsConversation
          jobs={visibleJobs}
          selectedSession={selectedSession}
          selectedSessionId={selectedSessionId}
          scopeSummary={scopeSummary}
          onOpenJobDetails={openJobDetails}
        />
      </div>

      <div className="jobs-composer-bar">
        <MessageComposer />
      </div>

      <JobWorkbenchDrawer
        isOpen={isJobsDrawerOpen}
        onClose={() => setIsJobsDrawerOpen(false)}
        jobs={visibleJobs.slice().sort((a, b) => b.updated_at_ms - a.updated_at_ms)}
        selectedSession={selectedSession}
        scopeSummary={scopeSummary}
      />
    </div>
  );
}
