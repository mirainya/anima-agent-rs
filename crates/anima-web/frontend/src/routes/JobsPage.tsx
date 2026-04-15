import { MessageComposer } from '@/features/chat/MessageComposer';
import { JobWorkbenchDrawer } from '@/features/jobs/JobWorkbenchDrawer';
import { JobsConversation } from '@/features/jobs/JobsConversation';
import { useJobsPageViewModel } from '@/features/jobs/useJobsPageViewModel';
import { shortId } from '@/shared/utils/format';

export function JobsPage() {
  const {
    status,
    selectedSessionId,
    selectedSession,
    visibleJobs,
    detailJob,
    drawerJobs,
    scopeSummary,
    hierarchySummary,
    jobListFilter,
    setJobListFilter,
    isJobsDrawerOpen,
    setIsJobsDrawerOpen,
    openJobDetails,
    sessionJobsCount,
  } = useJobsPageViewModel();

  return (
    <div className="jobs-page">
      <div className="jobs-workbench-surface">
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
            <div className="jobs-toolbar-meta">
              {scopeSummary} · {visibleJobs.length}/{sessionJobsCount} Jobs · 主任务 {hierarchySummary.main} · 子任务 {hierarchySummary.subtasks}
            </div>
            <div className="jobs-filter-group" role="tablist" aria-label="Job 过滤器">
              {[
                { key: 'all', label: '全部' },
                { key: 'active', label: '进行中' },
                { key: 'review', label: '待反馈' },
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

        <div className="jobs-main jobs-conversation-stage jobs-main-stage">
          <section className="jobs-main-panel jobs-secondary-panel">
            <JobsConversation
              jobs={visibleJobs}
              selectedJob={detailJob}
              selectedSession={selectedSession}
              selectedSessionId={selectedSessionId}
              scopeSummary={scopeSummary}
              runtimeTimeline={status?.runtime_timeline ?? []}
              onOpenJobDetails={openJobDetails}
            />
          </section>
        </div>

        <div className="jobs-composer-floating">
          <MessageComposer />
        </div>
      </div>

      <JobWorkbenchDrawer
        isOpen={isJobsDrawerOpen}
        onClose={() => setIsJobsDrawerOpen(false)}
        jobs={drawerJobs}
        selectedSession={selectedSession}
        scopeSummary={scopeSummary}
      />
    </div>
  );
}
