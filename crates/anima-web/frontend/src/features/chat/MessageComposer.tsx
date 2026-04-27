import { FormEvent, useState, useRef, useEffect } from 'react';
import { useSendMessageMutation } from '@/shared/api/chat';
import { useUiStore } from '@/shared/state/useUiStore';
import './chat.css';

export function MessageComposer() {
  const [value, setValue] = useState('');
  const selectedSessionId = useUiStore((state) => state.selectedSessionId);
  const { mutate: sendMessage, isPending } = useSendMessageMutation();
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = textareaRef.current;
    if (el) {
      el.style.height = 'auto';
      el.style.height = Math.min(el.scrollHeight, 120) + 'px';
    }
  }, [value]);

  const onSubmit = (event?: FormEvent) => {
    event?.preventDefault();
    const content = value.trim();
    if (!content) return;
    sendMessage({ content, session_id: selectedSessionId });
    setValue('');
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onSubmit();
    }
  };

  return (
    <form className="chat-composer" onSubmit={onSubmit}>
      <textarea
        ref={textareaRef}
        className="chat-composer-input"
        placeholder={selectedSessionId ? '输入消息...' : '输入消息创建新任务...'}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={onKeyDown}
        disabled={isPending}
        rows={1}
      />
      <button type="submit" disabled={isPending || !value.trim()} className="chat-composer-send">
        发送
      </button>
    </form>
  );
}
