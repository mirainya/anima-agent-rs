import { FormEvent, useState } from 'react';
import { useSendMessageMutation } from '@/shared/api/chat';
import { useUiStore } from '@/shared/state/useUiStore';
import './chat.css';

export function MessageComposer() {
  const [value, setValue] = useState('');
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const selectedScope = useUiStore((state) => state.selectedScope);
  const { mutate: sendMessage, isPending } = useSendMessageMutation();

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    const content = value.trim();
    if (!content) return;
    sendMessage({ content, session_id: selectedSessionId });
    setValue('');
  };

  return (
    <form className="app-message-input-bar" onSubmit={onSubmit}>
      <div className="message-composer-meta">
        发送到{selectedScope === 'job' || selectedScope === 'session' ? '当前会话' : '新任务入口'}
      </div>
      <input
        className="message-input"
        placeholder={selectedSessionId ? '输入消息并发送到当前会话...' : '输入消息以创建 Job...'}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        disabled={isPending}
      />
      <button type="submit" disabled={isPending || !value.trim()} className="message-send-btn">
        发送
      </button>
    </form>
  );
}
