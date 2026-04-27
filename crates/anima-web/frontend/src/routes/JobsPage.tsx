import { ChatMessageList } from '@/features/jobs/ChatMessageList';
import { MessageComposer } from '@/features/chat/MessageComposer';
import { useJobsPageViewModel } from '@/features/jobs/useJobsPageViewModel';
import '@/styles/layout.css';

export function JobsPage() {
  const {
    selectedSessionId,
    selectedSession,
    visibleJobs,
  } = useJobsPageViewModel();

  return (
    <div className="chat-page">
      <div className="chat-message-area">
        <ChatMessageList
          jobs={visibleJobs}
          selectedSession={selectedSession}
          selectedSessionId={selectedSessionId}
        />
      </div>

      <div className="chat-composer-area">
        <MessageComposer />
      </div>
    </div>
  );
}
