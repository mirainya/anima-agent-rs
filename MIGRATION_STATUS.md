# anima-agent-rs 迁移交付说明

## 目标

本仓库是对 `F:/free/rust/miraiClaw/anima-agent-clj-main` 的 Rust 重构版本。

当前阶段的核心原则是：

- 只做重构
- 不主动增加功能
- 不主动改变行为
- 以现有 Clojure 代码与测试行为为准，而不是以 README 蓝图为准

---

## 当前完成状态

已完成：

- Phase 1：SDK 核心
- Phase 2：bus + channel 核心骨架
- Phase 3：CLI channel 与 CLI 入口
- Phase 4：agent 主路径
- Phase 5：support 层
- Phase 6：默认入口切换

当前 Rust 已经是此仓库中的默认实现入口。

---

## 当前仓库默认入口

### 默认 workspace 成员

根 `Cargo.toml` 已配置：

```toml
default-members = ["crates/anima-cli"]
```

因此在仓库根目录执行以下命令时，会默认使用 Rust CLI：

```bash
cargo run
cargo test
```

### 默认可执行程序

当前 CLI 包名与二进制名为：

- package: `anima-agent-rs`
- bin: `anima-agent-rs`

帮助命令：

```bash
cargo run -- --help
```

---

## 已实现的兼容范围

## 1. SDK / HTTP client 层

对应 crate：

- `crates/anima-sdk`
- `crates/anima-types`

### 已覆盖内容

- `client`
  - 默认请求配置
  - URL 拼接行为
  - query 参数拼接
  - JSON body 序列化
  - 响应解析
- `sessions`
- `messages`
- `files`
- `config`
- `projects`
- `facade`

### 已保持的兼容语义

- 成功状态：`200/201`
- 失败分类：
  - `400 -> bad request`
  - `404 -> not found`
  - `500 -> server error`
  - 其他 -> unknown
- 成功返回 data、失败返回错误
- `build-url` 保持 base/path 斜杠归一化拼接
- query 参数追加保持现有拼接语义：
  - 空 query 跳过
  - 已有 query 时继续追加
  - `bool/string/null/array` 按当前字符串化规则拼接
- 空响应体 `200` 保持 success 且 `data = nil/null`
- `send-prompt` 保持宽松输入标准化：
  - `string`
  - `{ text: ... }`
  - `{ parts: [...] }`
- 对无效 message format 进行拒绝
- `messages/files/config/sessions` 保持当前必填参数校验语义
- permission response 限定：
  - `once`
  - `always`
  - `reject`

### 对应关键文件

- `crates/anima-sdk/src/client.rs`
- `crates/anima-sdk/src/messages.rs`
- `crates/anima-sdk/src/sessions.rs`
- `crates/anima-sdk/src/files.rs`
- `crates/anima-sdk/src/config.rs`
- `crates/anima-sdk/src/projects.rs`
- `crates/anima-sdk/src/facade.rs`
- `crates/anima-types/src/lib.rs`

---

## 2. runtime / bus / channel 层

对应 crate：

- `crates/anima-runtime`

### 已覆盖内容

- `Bus`
  - inbound/outbound publish/consume
- routing key helpers
- `Channel` trait
- `SessionStore`
- `ChannelRegistry`
- outbound dispatch
- 轻量收口补测
  - CLI chunk/final prompt 行为
  - `ChannelRegistry` 列举接口（`channel_names` / `all_channels`）
  - `SessionStore` helper 查询（`session_exists` / `get_session_by_routing_key` / account 维度查询）

### 已保持的兼容语义

- routing key：
  - `anima.session.{id}`
  - `anima.user.{id}`
  - `anima.channel.{name}`
  - `anima.broadcast`
- `find_or_create_session` 查找顺序：
  1. session-id
  2. routing-key
  3. 从 routing-key 提取 session-id
  4. create new
- `SessionStore` 已补齐的管理面行为：
  - CRUD / history
  - `touch_session`
  - `get_stats`
  - `session_count_by_channel`
  - `close_all_sessions`
- registry fallback 顺序：
  1. 指定 account
  2. `default`
  3. first available
- `ChannelRegistry` 已补齐的管理面行为：
  - register / unregister / unregister_with_account
  - `start_all` / `stop_all`
  - `health_report`
- outbound dispatch target 优先级：
  1. `reply_target`
  2. `sender_id`
- outbound dispatch loop 已覆盖：
  - 成功派发
  - missing channel
  - send failure
  - 错误不阻塞正常消息

### 对应关键文件

- `crates/anima-runtime/src/bus.rs`
- `crates/anima-runtime/src/channel.rs`

---

## 3. CLI 层

对应 crate：

- `crates/anima-cli`
- `crates/anima-runtime/src/cli.rs`

### 已覆盖内容

- CLI 默认 URL
- CLI 默认 prompt
- stdin/stdout 交互循环
- CLI 特殊命令
- CLI history / status / clear / exit 流程

### 已保持的兼容语义

- 默认 URL：
  - `http://127.0.0.1:9711`
- 默认 prompt：
  - `anima> `
- 退出命令：
  - `exit`
  - `quit`
  - `:q`
  - `/quit`
  - `/exit`
  - `bye`
- 特殊命令：
  - `help`
  - `status`
  - `history`
  - `clear`
- 空白输入仅回显 prompt
- `send_message` 输出行为：
  - `stage = final` 或未指定时追加 prompt
  - `stage = chunk` 时不追加 prompt
- 普通消息会写入 history，并发布到 bus inbound

### 对应关键文件

- `crates/anima-runtime/src/cli.rs`
- `crates/anima-cli/src/main.rs`

---

## 4. agent 主路径

对应文件：

- `crates/anima-runtime/src/agent.rs`

### 已覆盖内容

- `Task` / `TaskResult`
- `WorkerAgent`
- `WorkerPool`
- `CoreAgent`
- 兼容 facade：`Agent`

### 当前主路径

```text
Bus inbound
  -> CoreAgent
  -> WorkerPool
  -> WorkerAgent
  -> SDK
  -> Bus outbound
```

### 已支持的任务类型

- `api-call`
- `session-create`
- `transform`
- `query`

### 已保持的兼容语义

- 默认普通消息走 `api-call`
- `system-command` 走 direct 分支
- 同一 chat/session context 会复用已创建的 opencode session
- 错误响应格式：
  - `Error: ...`
- agent outbound -> dispatch -> channel 的 round-trip 主链路已通过测试覆盖
- 多 account / 多 channel 注册下，消息会按 `account-id` 路由到指定 channel
- mixed dispatch outcome 下：
  - failing channel 与 missing channel 会记错误统计
  - 成功消息不会被阻塞

---

## 5. support 层

对应文件：

- `crates/anima-runtime/src/support.rs`

### 已覆盖内容

- `ContextManager`
- `LruCache`
- `MetricsCollector`

### 当前接入方式

在 `CoreAgent` 中已接入：

- context manager
  - 记录 session history
  - 支持 snapshot / restore
- cache
  - single `api-call` 使用 cache key 做缓存
- metrics
  - 记录消息计数、cache 命中、latency、worker/session gauge

### 当前已记录的主要指标

- `messages_received`
- `messages_processed`
- `messages_failed`
- `tasks_submitted`
- `tasks_completed`
- `tasks_failed`
- `cache_hits`
- `cache_misses`
- `cache_evictions`
- `message_latency`
- `sessions_active`
- `workers_active`
- `workers_idle`

### 本轮补测后已显式验证的 support 接入点

- task 成功/失败路径会更新 `tasks_completed` / `tasks_failed`
- session / worker gauge 会在 agent 主路径中更新
- cache 命中与 miss 已有测试覆盖

---

## 当前仍属于“最小实现”的部分

虽然 Phase 1-6 已完成，但当前仍然是“兼容优先、最小可用”的 Rust 版本，不代表 README 里所有宏大能力都已完整落地。

### 当前明确仍是最小实现/未扩展部分

- support 层目前是内存实现优先
- agent 逻辑已覆盖当前真实主路径，但没有主动扩展成更复杂调度系统
- 没有额外引入 fancy TUI、readline 增强、WebSocket、cluster 等新能力
- 没有补做 README 中那些未被真实代码/测试稳定证明的高级功能

---

## 验证方式

### 默认 CLI help

```bash
cargo run -- --help
```

### 运行默认 CLI

```bash
cargo run
```

### 运行全 workspace 测试

```bash
CARGO_TARGET_DIR=/tmp/anima-agent-rs-target cargo test --workspace --manifest-path "F:/free/rust/miraiClaw/anima-agent-rs/Cargo.toml"
```

### 已通过的测试集合

- CLI 参数与默认值测试
- SDK compat 测试
  - URL / query / response parse compat
  - messages normalize / permission / command validation
  - files / config / sessions / projects 更多参数校验与 endpoint matrix
- runtime Phase 2 测试
  - routing key / SessionStore CRUD / history / stats / close-all
  - SessionStore helper 查询与 account 维度查询
  - ChannelRegistry fallback / register / unregister / lifecycle / health
  - ChannelRegistry 列举接口（`channel_names` / `all_channels`）
  - outbound dispatch / dispatch loop / error stats
- CLI Phase 3 测试
  - help / status / history / clear / exit
  - empty input prompt 回显
  - final/chunk prompt 行为
- agent Phase 4 测试
  - worker / worker pool
  - session reuse
  - round-trip 主链路
  - error response
  - 多 channel / 多 account 路由
  - mixed dispatch outcome 不阻塞成功消息
- support Phase 5 测试
  - context snapshot / restore
  - LRU cache stats
  - metrics snapshot

### 当前最近一次整体验证结果

已执行：

```bash
cargo test --manifest-path "F:/free/rust/miraiClaw/anima-agent-rs/Cargo.toml" -p anima-sdk -p anima-runtime
```

通过情况：

- `anima-runtime`
  - `agent_phase4`: 8 passed
  - `cli_phase3`: 8 passed
  - `runtime_phase2`: 15 passed
  - `support_phase5`: 3 passed
- `anima-sdk`
  - `sdk_compat`: 24 passed

合计：`58` 个测试通过。

---

## 关键文件索引

### workspace
- `Cargo.toml`

### SDK
- `crates/anima-types/src/lib.rs`
- `crates/anima-sdk/src/client.rs`
- `crates/anima-sdk/src/messages.rs`
- `crates/anima-sdk/src/sessions.rs`
- `crates/anima-sdk/src/files.rs`
- `crates/anima-sdk/src/config.rs`
- `crates/anima-sdk/src/projects.rs`
- `crates/anima-sdk/src/facade.rs`

### runtime
- `crates/anima-runtime/src/bus.rs`
- `crates/anima-runtime/src/channel.rs`
- `crates/anima-runtime/src/cli.rs`
- `crates/anima-runtime/src/agent.rs`
- `crates/anima-runtime/src/support.rs`

### CLI
- `crates/anima-cli/src/main.rs`

### tests
- `crates/anima-sdk/tests/sdk_compat.rs`
- `crates/anima-runtime/tests/runtime_phase2.rs`
- `crates/anima-runtime/tests/cli_phase3.rs`
- `crates/anima-runtime/tests/agent_phase4.rs`
- `crates/anima-runtime/tests/support_phase5.rs`

---

## 总结

当前 `anima-agent-rs` 已经完成从 Clojure 版本抽取出的核心兼容主路径重构，并且 Rust CLI 已切换为默认入口。

当前可以视为 **Phase 1-6 + 轻量收口** 已完成：

- 已冻结主路径兼容边界
- 新增补测仅覆盖既有语义，不引入新功能
- 当前基线适合作为后续独立阶段迁移的回归基准

后续建议将 `dispatcher/*` 明确作为独立 **Phase 7** 推进，优先顺序是：

1. `dispatcher/balancer` + `dispatcher/router`
2. `dispatcher/circuit_breaker`
3. `dispatcher/core` 与现有 Rust `bus/channel/agent` 对接
