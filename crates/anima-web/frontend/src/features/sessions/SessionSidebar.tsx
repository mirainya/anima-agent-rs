import { useState, useRef, useEffect } from 'react';
import { useJobsQuery } from '@/shared/api/jobs';
import { useSessionsQuery, useDeleteSessionMutation, useRenameSessionMutation } from '@/shared/api/sessions';
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

  const deleteMutation = useDeleteSessionMutation();
  const renameMutation = useRenameSessionMutation();

  const [searchQuery, setSearchQuery] = useState('');
  const [menuOpenId, setMenuOpenId] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const workers = status?.workers ?? [];
  const busyCount = workers.filter((w) => w.status !== 'idle').length;
  const agentRunning = status?.agent.status === 'running';

  const filteredSessions = sessions.filter((s) => {
    if (!searchQuery.trim()) return true;
    const q = searchQuery.toLowerCase();
    const title = (s.title ?? '').toLowerCase();
    const preview = s.last_user_message_preview.toLowerCase();
    const id = s.chat_id.toLowerCase();
    return title.includes(q) || preview.includes(q) || id.includes(q);
  });

  useEffect(() => {
    if (renamingId && renameInputRef.current) {
      renameInputRef.current.focus();
      renameInputRef.current.select();
    }
  }, [renamingId]);

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpenId(null);
      }
    }
    if (menuOpenId) {
      document.addEventListener('mousedown', handleClickOutside);
      return () => document.removeEventListener('mousedown', handleClickOutside);
    }
  }, [menuOpenId]);

  function handleStartRename(sessionId: string, currentTitle: string) {
    setRenamingId(sessionId);
    setRenameValue(currentTitle);
    setMenuOpenId(null);
  }

  function handleSubmitRename(sessionId: string) {
    const trimmed = renameValue.trim();
    if (trimmed) {
      renameMutation.mutate({ sessionId, title: trimmed });
    }
    setRenamingId(null);
  }

  function handleDelete(sessionId: string) {
    deleteMutation.mutate(sessionId, {
      onSuccess: () => {
        if (selectedSessionId === sessionId) {
          setSelectedSessionId(null);
        }
        setConfirmDeleteId(null);
      },
    });
  }

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
        <div className="sidebar-search">
          <input
            type="text"
            className="sidebar-search-input"
            placeholder="搜索会话..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              type="button"
              className="sidebar-search-clear"
              onClick={() => setSearchQuery('')}
            >
              ×
            </button>
          )}
        </div>
      </div>

      <div className="sidebar-sessions">
        {isLoading ? (
          <div className="sidebar-placeholder">加载中...</div>
        ) : filteredSessions.length === 0 ? (
          <div className="sidebar-placeholder">
            {searchQuery ? '无匹配会话' : '暂无会话'}
          </div>
        ) : (
          filteredSessions.map((session) => {
            const active = session.chat_id === selectedSessionId;
            const jobCount = jobs.filter((j) => j.chat_id === session.chat_id).length;
            const displayTitle = session.title || session.last_user_message_preview || shortId(session.chat_id);
            const isRenaming = renamingId === session.chat_id;

            return (
              <div key={session.chat_id} className="sidebar-session-wrapper">
                <button
                  type="button"
                  className={`sidebar-session-item ${active ? 'active' : ''}`}
                  onClick={() => setSelectedSessionId(session.chat_id)}
                >
                  <div className="sidebar-session-title">
                    <span className="sidebar-session-icon">💬</span>
                    {isRenaming ? (
                      <input
                        ref={renameInputRef}
                        className="sidebar-rename-input"
                        value={renameValue}
                        onChange={(e) => setRenameValue(e.target.value)}
                        onBlur={() => handleSubmitRename(session.chat_id)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') handleSubmitRename(session.chat_id);
                          if (e.key === 'Escape') setRenamingId(null);
                        }}
                        onClick={(e) => e.stopPropagation()}
                      />
                    ) : (
                      displayTitle
                    )}
                  </div>
                  <div className="sidebar-session-meta">
                    {shortId(session.chat_id)} · {jobCount} 任务
                    {session.last_active ? ` · ${formatTimestamp(session.last_active)}` : ''}
                  </div>
                </button>

                <div className="sidebar-session-actions">
                  <button
                    type="button"
                    className="sidebar-session-menu-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenuOpenId(menuOpenId === session.chat_id ? null : session.chat_id);
                    }}
                  >
                    ⋯
                  </button>

                  {menuOpenId === session.chat_id && (
                    <div ref={menuRef} className="sidebar-session-menu">
                      <button
                        type="button"
                        className="sidebar-session-menu-item"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleStartRename(session.chat_id, displayTitle);
                        }}
                      >
                        ✏️ 重命名
                      </button>
                      <button
                        type="button"
                        className="sidebar-session-menu-item danger"
                        onClick={(e) => {
                          e.stopPropagation();
                          setConfirmDeleteId(session.chat_id);
                          setMenuOpenId(null);
                        }}
                      >
                        🗑️ 删除
                      </button>
                    </div>
                  )}
                </div>

                {confirmDeleteId === session.chat_id && (
                  <div className="sidebar-confirm-delete">
                    <span>确认删除？</span>
                    <button
                      type="button"
                      className="sidebar-confirm-btn yes"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDelete(session.chat_id);
                      }}
                    >
                      删除
                    </button>
                    <button
                      type="button"
                      className="sidebar-confirm-btn no"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmDeleteId(null);
                      }}
                    >
                      取消
                    </button>
                  </div>
                )}
              </div>
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
