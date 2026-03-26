# anima-agent-rs

`anima-agent-rs` 是 [anima-agent](https://github.com/mirainya/anima-agent-clj-main) Clojure 版本的 Rust 移植版，目标不是单纯做一个聊天壳子，而是构建一个 **可调度、可观测、可扩展** 的智能体运行时。

[English README](./README.md)

## 项目简介

这个项目更偏向 **AI Agent Runtime / 执行平台**，而不是传统的“聊天机器人应用”。

它保留了原始架构思路，同时利用 Rust 带来的：

- 更明确的并发模型
- 更好的性能表现
- 更强的类型安全
- 更适合长期演进的工程结构

当前 workspace 包含 6 个 crate：

| Crate | 说明 |
|---|---|
| `anima-runtime` | 核心运行时：Agent 生命周期、消息总线、WorkerPool、SpecialistPool、编排器、缓存、上下文、指标等 |
| `anima-sdk` | 与 OpenCode 兼容后端交互的 HTTP SDK |
| `anima-cli` | 命令行入口，适合本地调试与快速验证 |
| `anima-web` | 基于 Axum 的 Web 控制台，包含聊天界面、SSE 实时事件、Worker 状态监控 |
| `anima-types` | 共享类型定义 |
| `anima-testkit` | 测试工具与假执行器 |

## 这个项目的特点

相比很多“AI 聊天产品”或轻量 Agent Demo，这个项目更强调：

- **运行时能力**：不是只关心回答内容，还关心任务如何被调度和执行
- **系统可观测性**：可以看到 worker 状态、当前任务、指标、内部事件
- **可扩展架构**：Bus、Channel、Worker、Specialist、Orchestrator 都是解耦组件
- **适合私有部署**：更容易做本地化、内网化、自托管部署
- **Rust 工程化实现**：显式生命周期、原子状态、条件变量等待、受控并发

## 整体架构

```text
┌─────────────────────────────────────────────────────────────────────┐
│                             接入层                                  │
│                 anima-cli / anima-web / 各类 channel                │
├─────────────────────────────────────────────────────────────────────┤
│                           anima-runtime                             │
│                                                                     │
│  入站消息                                                            │
│     │                                                               │
│     ▼                                                               │
│  Bus / Channel Registry                                             │
│     │                                                               │
│     ▼                                                               │
│  CoreAgent                                                          │
│  ├─ 会话与上下文管理                                                 │
│  ├─ 请求分类                                                         │
│  ├─ 缓存判断                                                         │
│  └─ 编排执行                                                         │
│      │                                                              │
│      ├─ WorkerPool      → 通用任务执行                              │
│      ├─ ParallelPool    → 批量并行执行                              │
│      └─ SpecialistPool  → 按能力路由到专家池                        │
│                                                                     │
│  辅助能力：                                                          │
│  - Dispatcher / 路由                                                 │
│  - LRU / TTL Cache                                                   │
│  - 分层上下文存储                                                    │
│  - Metrics / 运行时指标                                              │
└─────────────────────────────────────────────────────────────────────┘
```

## 核心执行流程

1. 外部消息先进入 Bus。
2. `CoreAgent` 确保 session / context 已建立。
3. `AgentClassifier` 将请求分类成执行计划。
4. `AgentOrchestrator` 按计划执行。
5. 任务被分发到 `WorkerPool`、`ParallelPool` 或 `SpecialistPool`。
6. 结果汇总后作为出站消息返回。
7. `anima-web` 可以通过状态接口和 SSE 实时查看运行情况。

## 关键能力

### 1. Agent 运行时
- WorkerPool：Worker 忙闲状态跟踪、任务执行、指标统计
- SpecialistPool：基于 capability 的专家路由，支持最少负载 / 轮询 / 随机策略
- ParallelPool：批量任务并行执行，支持 fail-fast 和最低成功率阈值
- Orchestrator：支持 single / sequential / parallel / specialist-route 等执行模式

### 2. 可观测性
- 实时查看 worker 状态
- 查看当前正在处理的任务信息：
  - `task_id`
  - `task_type`
  - `elapsed_ms`
  - `content_preview`
- 通过 SSE 把内部事件推到 Web 控制台

### 3. 消息总线
- 基于有界 pub/sub 的解耦通信
- 入站 / 出站 / 内部事件分流
- 适合作为运行时内部组件间的消息骨架

### 4. 缓存与上下文
- LRU / TTL 缓存
- 会话历史与上下文管理
- 为后续更复杂的智能体记忆能力保留了扩展空间

### 5. 指标与诊断
- counters / gauges / histograms
- 运行时快照
- 适合后续接 Prometheus / Dashboard

## Web 控制台

`anima-web` 是当前项目一个非常实用的可视化入口，提供：

- 聊天输入界面
- 会话列表
- SSE 实时事件流
- 系统状态面板
- Worker 卡片监控

Worker 卡片目前可以显示：

- Worker 当前状态
- 已完成 / 错误 / 超时统计
- 正在执行的任务类型
- 当前任务已运行时间
- 当前任务内容预览

默认访问地址：

```text
http://localhost:3000
```

## 快速开始

### 环境要求

- Rust 1.70+（edition 2021）
- 如果要体验真实 Agent 响应，需要本地可用的 OpenCode 兼容后端

### 构建

```bash
cargo build --workspace
```

### 运行 CLI

```bash
cargo run -p anima-cli -- --help
```

### 运行 Web 控制台

```bash
cargo run -p anima-web
```

然后浏览器打开：<http://localhost:3000>

### 运行测试

```bash
cargo test --workspace
```

## 目录结构

```text
anima-agent-rs/
├── Cargo.toml
├── README.md
├── README.zh-CN.md
└── crates/
    ├── anima-cli/              # CLI 可执行入口
    ├── anima-runtime/          # 核心运行时
    │   └── src/
    │       ├── agent.rs                # Agent 门面与 CoreAgent 主循环
    │       ├── agent_worker.rs         # WorkerAgent / WorkerPool
    │       ├── agent_parallel_pool.rs  # 并行批处理执行
    │       ├── agent_specialist_pool.rs# 专家路由
    │       ├── agent_orchestrator.rs   # 执行计划与编排器
    │       ├── agent_classifier.rs     # 请求分类
    │       ├── bus/                    # 消息总线
    │       ├── cache/                  # LRU / TTL 缓存
    │       ├── channel/                # 各类 Channel 适配器
    │       ├── context/                # 分层上下文存储
    │       ├── dispatcher/             # 路由 / 熔断 / 分发
    │       ├── pipeline/               # 处理流水线
    │       └── support/                # 指标与辅助工具
    ├── anima-sdk/              # HTTP SDK
    ├── anima-testkit/          # 测试工具
    ├── anima-types/            # 共享类型定义
    └── anima-web/              # Web UI + Axum 服务
        └── src/
            ├── main.rs
            ├── routes.rs
            ├── sse.rs
            ├── web_channel.rs
            └── static/index.html
```

## 当前状态

项目已经具备：

- 可运行的 runtime
- CLI 入口
- Web 可视化控制台
- Worker 当前任务监控
- 基础的任务编排 / 专家路由 / 并行执行能力

同时代码中已经铺好了更进一步演进到复杂工作流编排与多智能体协作的基础结构。

## License

MIT
