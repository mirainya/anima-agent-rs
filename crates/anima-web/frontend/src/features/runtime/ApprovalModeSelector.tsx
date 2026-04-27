import { useApprovalModeQuery, useApprovalModeMutation, type ApprovalMode } from '@/shared/api/approval';

const modes: { value: ApprovalMode; label: string }[] = [
  { value: 'auto', label: '自动' },
  { value: 'supervised', label: '监督' },
  { value: 'manual', label: '手动' },
];

export function ApprovalModeSelector() {
  const { data: current } = useApprovalModeQuery();
  const mutation = useApprovalModeMutation();

  return (
    <div className="approval-mode-selector" role="radiogroup" aria-label="审批模式">
      {modes.map((m) => (
        <button
          key={m.value}
          type="button"
          className={`approval-mode-btn ${current === m.value ? 'active' : ''}`}
          aria-pressed={current === m.value}
          disabled={mutation.isPending}
          onClick={() => mutation.mutate(m.value)}
        >
          {m.label}
        </button>
      ))}
    </div>
  );
}
