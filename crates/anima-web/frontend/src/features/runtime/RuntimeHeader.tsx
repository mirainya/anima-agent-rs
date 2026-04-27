import { useStatusQuery } from '@/shared/api/status';

export function RuntimeHeader() {
  const { data: status } = useStatusQuery();

  return (
    <div className="runtime-header">
      <span>Agent {status?.agent.status ?? '...'}</span>
      <span>Pool {status?.worker_pool.active ?? 0}/{status?.worker_pool.size ?? 0}</span>
    </div>
  );
}
