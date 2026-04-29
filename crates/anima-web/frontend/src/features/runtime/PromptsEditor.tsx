import { useState, useEffect } from 'react';
import {
  usePromptsQuery,
  usePromptsMutation,
  PROMPT_LABELS,
  type PromptsConfig,
} from '@/shared/api/prompts';

export function PromptsEditor() {
  const { data: prompts, isLoading } = usePromptsQuery();
  const mutation = usePromptsMutation();
  const [draft, setDraft] = useState<PromptsConfig | null>(null);
  const [editing, setEditing] = useState<keyof PromptsConfig | null>(null);

  useEffect(() => {
    if (prompts && !draft) setDraft(prompts);
  }, [prompts, draft]);

  if (isLoading || !draft) return <p>加载提示词配置...</p>;

  const keys = Object.keys(PROMPT_LABELS) as (keyof PromptsConfig)[];

  const handleSave = () => {
    mutation.mutate(draft, {
      onSuccess: (updated) => {
        setDraft(updated);
        setEditing(null);
      },
    });
  };

  const handleReset = () => {
    if (prompts) {
      setDraft(prompts);
      setEditing(null);
    }
  };

  return (
    <section className="settings-section">
      <h3>提示词配置</h3>
      <div className="prompts-list">
        {keys.map((key) => (
          <div key={key} className="prompt-item">
            <div className="prompt-header">
              <span className="prompt-label">{PROMPT_LABELS[key]}</span>
              <button
                className="prompt-toggle"
                onClick={() => setEditing(editing === key ? null : key)}
              >
                {editing === key ? '收起' : '编辑'}
              </button>
            </div>
            {editing === key && (
              <textarea
                className="prompt-textarea"
                value={draft[key]}
                onChange={(e) => setDraft({ ...draft, [key]: e.target.value })}
                rows={10}
              />
            )}
          </div>
        ))}
      </div>
      <div className="prompt-actions">
        <button className="prompt-btn prompt-btn-primary" onClick={handleSave} disabled={mutation.isPending}>
          {mutation.isPending ? '保存中...' : '保存'}
        </button>
        <button className="prompt-btn" onClick={handleReset}>重置</button>
      </div>
    </section>
  );
}
