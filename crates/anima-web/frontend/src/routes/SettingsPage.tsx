import { useStatusQuery } from '@/shared/api/status';
import { formatDurationShort, shortId } from '@/shared/utils/format';
import { PromptsEditor } from '@/features/runtime/PromptsEditor';

export function SettingsPage() {
  const { data: status, isLoading } = useStatusQuery();

  if (isLoading || !status) {
    return <div className="settings-page"><p>加载中...</p></div>;
  }

  const { agent, worker_pool: pool, workers, warnings } = status;

  return (
    <div className="settings-page">
      <h2>运行时状态</h2>

      <section className="settings-section">
        <h3>Agent</h3>
        <dl className="settings-dl">
          <dt>状态</dt><dd>{agent.status}</dd>
          <dt>上下文</dt><dd>{agent.context_status}</dd>
          <dt>会话数</dt><dd>{agent.sessions_count}</dd>
          <dt>缓存条目</dt><dd>{agent.cache_entries}</dd>
        </dl>
      </section>

      <section className="settings-section">
        <h3>Worker Pool</h3>
        <dl className="settings-dl">
          <dt>容量</dt><dd>{pool.active}/{pool.size}</dd>
          <dt>空闲</dt><dd>{pool.idle}</dd>
          <dt>已停止</dt><dd>{pool.stopped}</dd>
        </dl>
        {workers.length > 0 && (
          <table className="settings-table">
            <thead>
              <tr><th>ID</th><th>状态</th><th>已完成</th><th>错误</th></tr>
            </thead>
            <tbody>
              {workers.map((w) => (
                <tr key={w.id}>
                  <td>{shortId(w.id)}</td>
                  <td>{w.status}</td>
                  <td>{w.metrics.tasks_completed}</td>
                  <td>{w.metrics.errors}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className="settings-section">
        <h3>Bus</h3>
        <dl className="settings-dl">
          <dt>溢出</dt><dd>{warnings.bus_overflow_active ? '是' : '否'}</dd>
          <dt>丢弃总数</dt><dd>{warnings.bus_drop_total}</dd>
        </dl>
      </section>

      <PromptsEditor />
    </div>
  );
}
