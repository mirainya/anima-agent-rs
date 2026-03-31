export function formatDurationShort(ms?: number | null): string {
  if (ms == null || Number.isNaN(Number(ms))) return '-';
  const n = Number(ms);
  if (n < 1000) return `${n}ms`;
  if (n < 60_000) return `${(n / 1000).toFixed(1)}s`;
  return `${(n / 60_000).toFixed(1)}m`;
}

export function formatTimestamp(ms?: number | null): string {
  if (!ms) return '-';
  const date = new Date(Number(ms));
  if (Number.isNaN(date.getTime())) return '-';
  return date.toLocaleTimeString('zh-CN', { hour12: false });
}

export function shortId(value?: string | null): string {
  if (!value) return '-';
  return value.length > 10 ? value.slice(0, 8) : value;
}
