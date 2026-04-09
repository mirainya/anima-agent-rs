export function formatActiveSubtask(activeSubtaskName?: string | null, activeSubtaskType?: string | null): string | null {
  if (!activeSubtaskName) {
    return null;
  }

  return activeSubtaskType ? `${activeSubtaskName}（${activeSubtaskType}）` : activeSubtaskName;
}
