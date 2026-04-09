# anima-agent-rs

`anima-agent-rs` 是 [anima-agent](https://github.com/mirainya/anima-agent-clj-main) Clojure 版本的 Rust 移植版。这个 workspace 的重点不是做一个单纯聊天壳子，而是构建一个 **可观测、可调度、可扩展** 的智能体运行时。

[English README](./README.md)

## 项目概览

`anima-agent-rs` 延续了原始框架偏运行时的设计思路，并结合 Rust 的显式并发模型与类型系统来组织实现。

当前仓库包含：

- 一个可复用的运行时 crate，负责 agent 执行、分类、编排、权限、hooks 与工具执行
- 一个用于本地交互与调试的 CLI 入口
- 一个基于 Axum 的 Web 应用，用来承载 React + Vite 工作台并通过 SSE 推送运行时事件
- 若干共享类型、HTTP SDK 与测试辅助 crate

## Workspace Crates

workspace 成员以根目录 `Cargo.toml` 为准：

| Crate | 说明 |
| --- | --- |
| `anima-types` | 运行时、Web、SDK、测试共用的类型定义 |
| `anima-sdk` | 与 OpenCode 兼容后端交互的 HTTP SDK |
| `anima-runtime` | 核心运行时：agent 执行、分类、编排、工具、流式处理、权限、hooks、prompt、指标 |
| `anima-cli` | 本地交互与调试用 CLI 可执行入口 |
| `anima-web` | Axum Web 服务与本地工作台 UI、SSE API |
| `anima-testkit` | 测试辅助工具与假执行器 |

## 整体架构

从整体上看，这个仓库是一个分层的运行时系统：

```text
┌──────────────────────────────────────────────────────────────────────┐
│ 接入层                                                               │
│ anima-cli / anima-web / 自定义 channel                              │
├──────────────────────────────────────────────────────────────────────┤
│ Web 应用层                                                           │
│ Axum routes + React/Vite 工作台 + SSE 事件流                         │
├──────────────────────────────────────────────────────────────────────┤
│ Runtime 引导层                                                       │
│ Bus + ChannelRegistry + Dispatcher + Agent + 可选 CLI channel       │
├──────────────────────────────────────────────────────────────────────┤
│ Runtime 核心领域                                                     │
│ agent / classifier / orchestrator / execution                        │
├──────────────────────────────────────────────────────────────────────┤
│ Runtime 基础设施                                                     │
│ tools / streaming / permissions / hooks / prompt / messages          │
│ bus / channel / context / dispatcher / cache / pipeline / metrics    │
├──────────────────────────────────────────────────────────────────────┤
│ 外部后端                                                             │
│ 通过 anima-sdk 访问 OpenCode 兼容模型后端                           │
└──────────────────────────────────────────────────────────────────────┘
```

落到职责上，可以把它理解为：`anima-runtime` 是系统核心，`anima-cli` 和 `anima-web` 是入口层，`anima-sdk` 是与上游模型 / 后端通信的边界。

## 核心执行链路

一次典型请求大致会按下面的顺序流转：

1. 用户消息从 CLI、Web 或其他 channel 进入系统。
2. 消息被发布到 runtime bus。
3. `CoreAgent` 准备 session / context，并组装 prompt 与输入数据。
4. 分类与编排层决定本次请求应该如何执行。
5. 执行层可能运行 agentic loop、调用工具、做 requirement 判断，并继续推进回合。
6. 工具调用会进入工具注册与执行层，并在需要时经过权限检查与 hook 处理。
7. 结果被写回出站 / 内部事件通道。
8. dispatcher 将对外消息投递到各个 channel，而 Web 工作台则通过 SSE 观察内部运行时事件。

所以这个项目并不只是“聊天页面调一下模型接口”，而是把消息传输、执行控制、工具调用、权限治理和可观测性拆成了独立层次的运行时系统。

## 更细的系统架构图

上面的分层图更适合快速建立全局印象；下面这张图把运行时内部的关键关系再展开一层：

```text
用户 / 操作者
   │
   ├─ CLI 输入
   ├─ Web 工作台操作
   └─ 其他 channel 输入
   │
   ▼
Channel 实现层
(CliChannel / WebChannel / custom Channel)
   │
   ▼
Bus（inbound / outbound / internal）
   │
   ├──────────────► internal events ──────────────► SSE 转发器 ───────► Web UI
   │
   ▼
CoreAgent
   │
   ├─ session + context 准备
   ├─ prompt 组装
   ├─ classifier
   ├─ orchestrator
   └─ execution driver / turn coordination
   │
   ▼
Agentic loop
   │
   ├─ 通过 anima-sdk 发起模型请求
   ├─ streaming parser / 流式事件
   ├─ tool 检测与执行
   ├─ requirement 判断
   └─ 持续迭代直到完成或挂起
   │
   ├─ tool call ─► ToolRegistry ─► 内建 / 自定义工具
   │                    │
   │                    ├─ PermissionChecker
   │                    └─ HookRegistry
   │
   ▼
出站响应 + runtime 状态更新
   │
   ├─ Dispatcher ─────────────────────────────────────────────────────► 已注册 channels
   └─ status / timeline / metrics / jobs ───────────────────────────► Web APIs
```

几个关键关系可以这样理解：

- **bus** 是运行时的消息骨架，用来隔离 inbound work、outbound response 和 internal events。
- **agent** 持有有状态执行生命周期，而不是让各个 channel 直接去调用模型。
- **execution loop** 是模型推理、工具调用、权限、hooks 和继续推进逻辑汇合的地方。
- **web 层** 既负责提交工作，也负责观察运行时内部状态，并不只是一个静态仪表盘。
- **dispatcher** 是对外投递边界，而 SSE 更偏向可观测性，而不是最终回复传输本身。

## 设计边界与非目标

这个仓库更适合被理解为一个 **runtime-first 的 agent system**，而不是单纯的聊天前端。

它是什么：

- 一个把 transport、execution control、tool use、permissions、hooks、observability 拆开的运行时系统
- 一个把 agent 执行视为有状态、可检查过程的代码库
- 一个面向“可治理工具工作流”的基础设施，而不只是文本生成壳子

它不是什么：

- 不是单页聊天应用外面包一层很薄的模型调用
- 不是把所有行为都隐藏在 prompt 里的黑盒 autonomous agent
- 不是让各个 channel 直接调模型、绕过 runtime 控制边界的设计
- 不是一个所有抽象都已经完全稳定下来的最终编排平台

这一层边界很重要：只有把 runtime 本身当成产品来理解，很多设计选择才会显得合理。

## 模块导航索引

如果是第一次进入仓库，可以按关注点快速定位：

- **看 runtime 如何装配起来**：`crates/anima-runtime/src/bootstrap.rs`、`crates/anima-runtime/src/lib.rs`
- **看 agent 生命周期**：`crates/anima-runtime/src/agent/`
- **看规划 / 路由 / 编排**：`crates/anima-runtime/src/classifier/`、`crates/anima-runtime/src/orchestrator/`
- **看 agentic loop**：`crates/anima-runtime/src/execution/agentic_loop.rs`、`crates/anima-runtime/src/execution/`
- **看工具执行闭环**：`crates/anima-runtime/src/tools/definition.rs`、`crates/anima-runtime/src/tools/registry.rs`、`crates/anima-runtime/src/tools/execution.rs`
- **看权限系统**：`crates/anima-runtime/src/permissions/`
- **看 hooks 机制**：`crates/anima-runtime/src/hooks/`
- **看消息整形 / 压缩 / 规范化**：`crates/anima-runtime/src/messages/`
- **看 prompt 组装**：`crates/anima-runtime/src/prompt/`
- **看流式响应处理**：`crates/anima-runtime/src/streaming/`
- **看 Web 集成**：`crates/anima-web/src/main.rs`、`crates/anima-web/src/routes.rs`、`crates/anima-web/src/sse.rs`
- **看前端工作台**：`crates/anima-web/frontend/`

## Runtime 模块结构

`crates/anima-runtime` 已经重组为目录化模块，公开模块树以 `crates/anima-runtime/src/lib.rs` 为准。

### 核心领域

- `agent/` — agent 核心引擎（CoreAgent facade）、任务类型、执行器、Worker 池、SuspensionCoordinator（挂起状态协调）、TaskRegistry
- `classifier/` — 规则分类、AI 分类、任务分类与智能路由
- `orchestrator/` — 编排核心、并行池、专家池
- `execution/` — 执行驱动、回合协调、上下文装配、需求判定
- `runtime/` — Event-sourced 统一运行时状态核心（RuntimeDomainEvent → reducer → RuntimeStateSnapshot → projection）
- `tasks/` — 领域模型定义（Run / Turn / Task / Suspension / Requirement / ToolInvocation 记录）、生命周期、调度、查询
- `transcript/` — 消息记录模型（MessageRecord）、追加、规范化、配对

### 基础设施与支撑模块

- `bus/` — 入站 / 出站 / 内部事件发布订阅总线
- `cache/` — LRU 与 TTL 缓存能力
- `channel/` — channel 适配器与 session 路由
- `context/` — 会话与上下文存储 / 管理
- `dispatcher/` — 路由、控制、负载均衡、诊断与队列处理
- `pipeline/` — 处理流水线构件
- `tools/` — 工具定义、注册、执行与内建工具实现
- `streaming/` — 流式 API 解析与执行辅助
- `permissions/` — 权限策略、类型与校验
- `messages/` — 消息规范化、查找、配对、压缩与协议辅助
- `hooks/` — hook 注册、执行器与类型定义
- `prompt/` — system prompt 段落化组装
- `bootstrap` / `cli` / `metrics` / `support` — 运行时引导、CLI 接线、指标与通用工具

目前 crate 仍保留旧路径的兼容 re-export，但新的文档说明应以当前目录化结构为准，不再继续使用旧的单文件主结构叙述。

## Runtime 职责划分

当前 runtime 的组织方式，不再是少数几个大文件，而是按职责拆成多个领域：

- **Agent 层**：负责核心 agent 生命周期、Worker 池、任务执行入口与运行时状态。
- **Classifier 层**：负责解释或路由入站请求。
- **Orchestrator 层**：负责协调串行、并行、专家路由等不同执行路径。
- **Execution 层**：负责回合循环、上下文装配、requirement 判断，以及模型 / 工具迭代。
- **工具与流式层**：负责内建工具、工具执行、流式响应解析与流式事件回调。
- **控制层**：通过 permissions、hooks、prompt 组装、messages 规范化来约束运行时行为。
- **基础设施层**：bus、channel、dispatcher、cache、context、pipeline、metrics、bootstrap 为整个运行时提供外围支撑。

一个比较好记的理解方式是：

- `bootstrap.rs` 负责把系统装配起来
- `bus` 负责承载消息和内部事件
- `dispatcher` 负责把出站消息投递到已注册 channel
- `agent` + `classifier` + `orchestrator` + `execution` 负责决定“具体做什么”
- `tools` + `permissions` + `hooks` 负责决定“工具辅助执行如何被允许、拦截和记录”
- `metrics`、状态快照和内部事件负责可观测性

## Web 工作台

`anima-web` 是一个基于 Axum 的本地工作台服务，负责承载前端页面与运行时 API。

当前 Web 技术栈：

- 后端：Rust + Axum
- 前端：React 18 + Vite
- 实时通信：SSE（`/api/events`）
- 本地默认地址：<http://localhost:3000>

前端构建产物默认位于 `crates/anima-web/src/static/dist`。如果该目录缺少构建结果，服务端会返回一个提示页面，要求先构建前端资源。

`anima-web` 并不是一个和 runtime 完全分离的后端。它会直接引导 `anima-runtime`，注册 `WebChannel`，启动内部 bus 事件转发，然后对外提供：

- SPA 首页与前端静态资源
- `/api/send`：提交用户入站消息
- `/api/events`：订阅 SSE 运行时事件
- `/api/status`：查看 runtime / worker / session 快照
- 面向工作台的 job review 与 question-answer 接口

因此，`anima-web` 既是操作工作台，也是 runtime 的一个集成入口：既能把任务送进运行时，也能近实时观察运行时内部状态。

前端脚本定义在 `crates/anima-web/frontend/package.json`：

| 命令 | 作用 |
| --- | --- |
| `npm run dev` | 启动 Vite 开发服务器 |
| `npm run build` | 构建生产前端资源 |
| `npm run preview` | 本地预览构建结果 |
| `npm run test` | 使用 Vitest 运行前端测试 |
| `npm run lint` | 对前端源码执行 ESLint |

## 关键运行时机制

### 上下文压缩

runtime 内置了自动上下文压缩机制，目的是让长时间运行的 agent loop 不必把无限增长的完整历史一直回传给模型。

当前实现采用两阶段压缩：

1. **Microcompact**：先清理受保护边界之前的旧 `tool_result` 内容，替换成更短的占位内容。
2. **Snip Compact**：如果还不够，再从最旧消息开始把符合条件的消息标记为 `filtered`。

当前设计里几个关键点：

- token 使用量是基于消息序列化后的大小做估算，不是调用模型提供方的精确 tokenizer
- 只有在达到可配置的上下文窗口阈值后才触发压缩
- 最近若干轮会被保护，不参与压缩
- 初始用户消息会被保留
- 被标记为 `filtered` 的消息，在发往上游 API 前会被规范化流程跳过

换句话说，这套机制优先保住最近几轮的推理连续性，再逐步收缩更早、尤其是工具结果较重的历史。

### 消息规范化

在调用上游模型 API 之前，内部消息会先被规范化：

- 已标记 `filtered` 的消息会被跳过
- 连续同角色消息会尝试合并
- 必要时会强制让 API 消息列表以 `user` 角色开头

这一步是 runtime 内部富状态消息结构，与上游相对简单的对话消息格式之间的桥梁。

### Prompt 组装

system prompt 不是一整段写死的大字符串，而是由多个有顺序的 section 组装而成。这样可以让 runtime 按关注点组合提示词，同时保持最终结果可预测、可排序。

### 工具执行流水线

一次工具调用会经历一个结构化闭环：

1. 输入 schema 校验
2. pre-tool hook 执行
3. 权限检查
4. 实际工具调用
5. post-tool hook 执行
6. 转换为 loop 可继续消费的 `tool_result` 消息

这一层结构很重要，因为它让工具调用变成一个可治理的执行过程，而不是随处散落的临时逻辑。

### 权限与挂起恢复

权限系统并不只是 allow / deny 两种结果，它还支持显式的“询问用户”分支。

当出现这种情况时：

- 当前工具调用会被挂起
- runtime 保存待执行调用状态
- 用户被要求确认
- 用户回应后 runtime 再恢复执行

这是整个系统里非常关键的一道控制边界。

### Hooks

当前 hooks 会包裹一些关键运行时时刻，例如 pre/post tool use、pre/post send。它们提供了日志、策略控制、附加副作用等扩展点，而不需要把所有逻辑直接硬编码进执行主循环。

### Streaming 与 SSE

仓库里其实有两种不同语义的“流式”：

- **模型流式**：位于 `anima-runtime/streaming`，负责解析上游 SSE 风格响应、累积文本 / tool JSON 增量，并反馈给 agentic loop
- **Web SSE**：位于 `anima-web`，负责把 runtime internal events 转发给浏览器，让工作台观察运行进度

前者是“消费模型流”，后者是“暴露运行时状态给操作者”。

## 消息模型

runtime 在不同层级使用不同消息表示，以适应不同职责：

- **InternalMsg**：最完整的运行时内部消息，带有 `message_id`、可选 `tool_use_id`、`filtered` 状态和 metadata
- **ApiMsg**：发给上游 LLM API 的简化消息格式
- **SdkMsg**：通过 `anima-sdk` 与上游通信时使用的 SDK 层消息封装

可以把它理解成：

```text
用户输入 / tool result / assistant 输出
            │
            ▼
InternalMsg（runtime 内部状态）
- 完整内容
- tool 关联
- filtered 标记
- metadata
            │
            ├─ compact / normalize / pair / transform
            ▼
ApiMsg（上游请求格式）
- 更简单的 role/content 结构
            │
            ▼
SdkMsg / provider request layer
```

之所以要这样拆层，是因为 runtime 需要比上游模型 API 更多的控制元数据。

### 实际消息生命周期

1. 用户输入或工具结果先进入运行时内部消息结构
2. 旧消息可能先经过压缩和过滤
3. 规范化流程把内部消息整理成可发往上游的格式
4. 上游响应再被解析回运行时结构
5. 工具输出带着 tool linkage 重新进入循环

这样既能让 runtime 保持有状态、可观测，又不会把同样复杂的结构直接暴露给外部 API 边界。

## 压缩、工具循环与权限边界流程图

```text
入站请求
   │
   ▼
组装 internal messages + prompt sections
   │
   ▼
检查上下文大小阈值
   │
   ├─ 未超过阈值 ─────────────────────────────────────────────────────► 继续执行
   │
   └─ 超过阈值
        │
        ├─ 阶段 1：压缩旧 tool_result 内容
        ├─ 阶段 2：将最旧且符合条件的消息标记为 filtered
        └─ 对剩余消息做 normalize，生成上游 API 输入
   │
   ▼
发送模型请求
   │
   ▼
接收 text / tool_use / streaming deltas
   │
   ├─ 只有最终文本 ───────────────────────────────────────────────────► 返回响应
   │
   └─ 检测到 tool_use
        │
        ▼
     校验输入
        │
        ▼
     执行 pre-tool hooks
        │
        ▼
     权限检查
        │
        ├─ allow ─► 执行工具 ─► post-tool hooks ─► 追加 tool_result ─► 继续 loop
        ├─ deny  ─► 进入工具拒绝 / 错误路径
        └─ ask   ─► 挂起调用 ─► 等待用户回答 ─► 恢复或拒绝
```

这就是整个 runtime 最核心的一条控制循环：既能自动推进，又能保留治理边界。

## 典型场景流程

### 1. 普通用户提问

这是最简单的一条端到端路径：

1. 用户从 CLI 或 Web 工作台提交消息。
2. 消息被转换为 inbound bus message。
3. `CoreAgent` 恢复 session / context，并准备模型输入。
4. 执行路径在不需要工具的情况下完成。
5. 最终回复进入 outbound 投递流程。
6. 用户通过 CLI 输出或 Web channel 看到结果。
7. 同时 runtime 仍可能发出内部事件，用于 tracing、metrics 和状态展示。

### 2. 模型触发 `tool_use`

当模型判断需要外部能力时，流程会变成：

1. 执行层收到包含文本和 / 或 `tool_use` 请求的模型响应。
2. 工具执行层从 `ToolRegistry` 中解析目标工具。
3. 工具输入被放入运行时 `ToolContext` 中准备执行。
4. 工具运行并返回结构化 `ToolResult`。
5. 工具结果再被写回消息流。
6. agentic loop 带着更新后的对话状态继续推进。
7. 模型可能继续调用更多工具，也可能直接产出最终答案。

这也是它为什么是 agentic runtime，而不是单次请求-单次回复的原因。

### 3. 工具调用因权限而等待确认

对于带权限约束的工具，流程会进入交互式暂停：

1. agentic loop 中检测到一个工具请求。
2. runtime 构造 permission request，并交给 `PermissionChecker` 判断。
3. 如果需要审批，当前 loop 会被挂起，而不是继续盲目执行。
4. runtime 保存待执行的工具调用和相关问题状态。
5. 用户被要求做出批准、拒绝或其他回答。
6. 用户回应后，runtime 恢复之前挂起的执行路径。
7. 工具要么被执行，要么被拒绝，然后 loop 再继续推进到完成。

这样可以把高风险动作放在显式审批边界内，而不是把它藏在模型输出后面。

### 4. Web 工作台通过 SSE 观察运行时进度

Web UI 可以在任务执行时持续观察 runtime：

1. 浏览器订阅 `/api/events`。
2. `anima-web` 监听 runtime internal bus。
3. Worker 状态变化和 runtime 事件被转发为 SSE 事件。
4. 前端近实时更新 worker 卡片、timeline 和 job 视图。
5. 浏览器也可以调用 `/api/status` 拉取 workers、sessions、metrics、jobs 的快照。

这让整个项目更像一个 operator console：用户看到的不只是最后答案，也能看到运行时是如何推进任务的。


## 状态与观测面

项目里暴露了多种不同的运行时“观察视图”，它们彼此相关，但语义不完全一样：

- **worker status**：每个 worker 当前正在做什么，包括 busy/idle 状态与当前任务预览
- **runtime timeline**：按时间排列的内部事件，例如 message received、planning、cache hit、requirement check、followup 调度、failure 等
- **recent execution summaries**：按消息聚合后的简化执行摘要，包含 plan type、status、阶段耗时、worker 分配、error code
- **jobs**：面向 Web 的聚合视图，由 accepted work、timeline events、summaries、failures、worker state、reviews、pending questions 等拼装而成
- **metrics**：counters、gauges、histograms 与运行时快照
- **recent sessions**：按 chat/session 维度整理的最近交互切片

它们分别回答的问题大致是：

- “系统现在在干什么？” → worker status / jobs
- “一路发生了什么？” → timeline
- “某次执行最后怎么结束的？” → execution summaries / failures
- “前端要渲染什么？” → jobs + status snapshot APIs
- “系统整体健康状况如何？” → metrics

## 术语表

- **job**：面向 Web 的工作单元，通常从一个入站请求演化而来，并被补充上状态、事件、执行摘要、worker 信息、review / question 状态等
- **trace_id**：端到端关联标识，用来把同一条逻辑执行链上的 runtime events、execution summaries、followup 活动串起来
- **message_id**：某个具体入站请求或运行时事件记录对应的消息标识
- **tool_use_id**：把模型发出的工具调用请求与后续 `tool_result` 对应起来的标识
- **filtered**：internal message 上的一个标志，表示这条消息会在发往上游 API 前被刻意排除，通常由上下文压缩触发
- **followup**：当 runtime 判断需求仍未满足时，自动安排的补充执行轮次
- **runtime timeline**：按时间记录执行过程内部事件的有序事件流
- **execution summary**：按单次执行聚合后的摘要记录，包含状态、阶段耗时、worker 分配与错误信息
- **pending question**：当前仍在等待用户回答的问题，常见于权限确认或澄清流程
- **job kind**：在带编排语义的视图里，一个 job 是主任务还是子任务

## FAQ

### 为什么 `anima-web` 必须先构建前端？
因为 Rust Web 服务实际提供的是 `crates/anima-web/src/static/dist` 下的构建产物。没有这份输出时，服务虽然能启动，但会主动返回提示页面要求先构建前端。

### 如果 `cargo build --workspace` 或 `cargo test --workspace` 可能失败，为什么 README 里还要写？
因为它们仍然是 workspace 顶层正确的标准命令。失败的原因是仓库当前存在进行中的代码，而不是命令本身写错了。

### 为什么既有 `InternalMsg`，又有 `ApiMsg`，不直接共用一种消息结构？
因为 runtime 内部需要比上游模型 API 更多的状态信息。内部消息需要携带 filtered、tool linkage、metadata 等信息，而 API 消息只需要满足对外请求格式。

### 既然模块结构已经重组，为什么还保留旧路径的 re-export？
因为代码库仍处于过渡期。新的目录化结构已经是当前事实来源，而兼容 re-export 可以在迁移过程中减少旧导入路径被一次性打断的范围。

### 为什么上下文压缩是启发式的？
因为当前压缩逻辑是基于消息序列化后的大小去估算 token，这种方案更容易在 runtime 内本地实现，但它并不等价于上游模型提供方的精确 token 计数。

### runtime events 和最终返回给用户的响应有什么区别？
runtime events 是内部可观测性信号，例如 planning、worker activity、followup scheduling、approval requests、failure 等；最终用户响应则是通过 channel 对外投递的回答内容。

## 架构对比：anima-agent-rs vs Claude Code vs OpenClaw

本项目在设计上参考了 Claude Code 和 OpenClaw 两个开源项目，但在定位和架构选择上有明确差异。下面从几个关键维度做横向对比。

### 定位差异

| 项目 | 定位 | 语言 |
| --- | --- | --- |
| Claude Code | 开发者 CLI / IDE 编程助手 | TypeScript (Bun) |
| OpenClaw | 多渠道 AI 消息 Gateway（Telegram / Discord / Slack / WhatsApp 等） | TypeScript (Node.js pnpm monorepo) |
| anima-agent-rs | 可观测、可调度的 Agent Runtime 引擎 | Rust |

Claude Code 的核心价值是"让开发者在终端里高效写代码"；OpenClaw 的核心价值是"把 AI agent 接入各种 IM 渠道"；anima-agent-rs 的核心价值是"把 agent 执行本身当成一个可治理、可观测的运行时系统来构建"。

### Agentic Loop 对比

**Claude Code** 的 agentic loop 在 `src/query.ts`（~1700 行），是一个 `async function*` 异步生成器，每轮迭代：压缩上下文 → 调用模型 → 消费流式响应 → 执行工具 → 继续循环。状态通过不可变 `State` 结构体每轮重建，用 `continue` 跳转而非递归。工具权限通过 React 状态队列 + Promise 挂起实现 UI 级别的交互式审批。

**OpenClaw** 的 agentic loop 分三层：外层 `runEmbeddedPiAgent` 负责 session lane 排队和 failover 重试；中层 `runEmbeddedAttempt`（~2000 行）负责单次 attempt 的完整生命周期（构建工具集、组装 prompt、创建 session、包装 streamFn decorator 链）；底层的 tool-use 循环由私有包 `@mariozechner/pi-coding-agent` 处理。

**anima-agent-rs** 的 agentic loop 在 `execution/agentic_loop.rs`，是一个同步 `loop {}`：检查迭代上限 → 消息配对修复 → 可选压缩 → 规范化 → 构建 API payload → 调用 executor → 解析响应 → 工具检测与执行 → 继续循环。挂起时返回 `AgenticLoopSuspension`，恢复时通过 `resume_suspended_tool_invocation` 重新进入循环。

### 状态管理对比

| 维度 | Claude Code | OpenClaw | anima-agent-rs |
| --- | --- | --- | --- |
| 主状态 | React AppState（类 Redux） | JSON session 文件 + SQLite task registry | Event-sourced RuntimeStateStore |
| 持久化 | 无（进程内） | 文件 + SQLite | 进程内（snapshot clone） |
| 查询 | 直接读 state | SQLite query | snapshot().xxx 全量 clone |
| 事件溯源 | 无 | 无 | 有（RuntimeDomainEvent → reducer → snapshot） |

anima-agent-rs 的 event-sourced 设计是三者中最结构化的，但当前全量 clone snapshot 的方式在长对话场景下有性能隐患。

### 工具执行对比

| 维度 | Claude Code | OpenClaw | anima-agent-rs |
| --- | --- | --- | --- |
| 工具定义 | Zod schema + React 渲染组件 | 通过 `createOpenClawCodingTools` 动态构建 | `ToolDefinition` trait + `ToolRegistry` |
| 执行模式 | 并发（`StreamingToolExecutor`）或串行 | 由底层 pi-agent 处理 | 串行（逐个执行） |
| 权限 | 多来源规则合并（session / settings / hooks） | exec security + sandbox + host 三维度 | `PermissionChecker` + 策略规则 |
| Hooks | stop hooks + pre/post tool hooks | 完整插件 hook 生态（~10 种生命周期 hook） | `HookRegistry` pre/post tool + pre/post send |

### 并发模型对比

| 维度 | Claude Code | OpenClaw | anima-agent-rs |
| --- | --- | --- | --- |
| 运行时 | Bun 单线程 + async/await | Node.js 单线程 + Promise lane 队列 | std::thread + parking_lot::Mutex + Condvar |
| 子 agent | 递归调用 `query()` + agentId 隔离 | ACP 协议 spawn/session 模式 | WorkerPool + Orchestrator 分发 |
| 取消 | AbortController | AbortController + runAbortController | AtomicBool flag |

anima-agent-rs 使用同步线程模型，好处是无 async runtime 依赖、调试简单；代价是无法做细粒度 timeout/cancellation，且与 async web 层（axum/tokio）之间存在边界摩擦。

### anima-agent-rs 的优势

1. **类型安全与编译期保证**：Rust 的所有权系统和类型系统在编译期就能捕获数据竞争、空指针、生命周期错误等问题，这是 TypeScript 项目无法做到的。
2. **Event-sourced 状态管理**：三者中唯一采用事件溯源的项目。所有状态变更都通过 `RuntimeDomainEvent` 记录，可以做到完整的状态回放和审计。
3. **结构化的运行时可观测性**：timeline、execution summary、failure snapshot、job view 等多维度观测面，比 Claude Code 的 React 状态和 OpenClaw 的 JSON 文件都更系统化。
4. **显式的领域分层**：agent / classifier / orchestrator / execution / tools / permissions / hooks / prompt / messages 各自独立，职责边界清晰。Claude Code 的核心逻辑集中在 `query.ts` 一个文件里，OpenClaw 的核心逻辑分散在 `attempt.ts` + 私有包里。
5. **多 Worker 并行执行**：通过 WorkerPool + Orchestrator 支持真正的多任务并行，而 Claude Code 和 OpenClaw 本质上都是单线程串行。
6. **独立的消息三层映射**：InternalMsg / ApiMsg / SdkMsg 的分层让 runtime 内部状态和外部 API 格式彻底解耦。

### anima-agent-rs 的不足

1. **CoreAgent 仍然过大**：虽然已经提取了 `SuspensionCoordinator`，但 `core.rs` 仍有 ~3200 行，承担了消息循环、会话管理、事件发布、状态 upsert、执行入口等多重职责。Claude Code 的 `query.ts` 虽然也有 ~1700 行，但它只负责 agentic loop 本身，其他职责分散在独立模块中。
2. **双重状态追踪**：`RuntimeStateStore` 和 `TaskRegistry` 追踪同一批数据，每次变更要写两遍，一致性靠人工保证。这是当前最大的架构债务。
3. **Snapshot 全量 clone**：每次读取状态都 clone 整个 snapshot（包含所有 runs、turns、tasks、transcript），随着对话变长开销线性增长。
4. **缺少插件生态**：OpenClaw 有完整的 npm 插件系统和 ~10 种生命周期 hook；Claude Code 有 MCP server 集成和 feature gate 机制；anima-agent-rs 的 hooks 系统相对基础。
5. **缺少上下文自动压缩的精细控制**：Claude Code 有 snip / microcompact / autocompact 三级压缩 + token budget 追踪；OpenClaw 有 context engine post-turn 维护；anima-agent-rs 的压缩机制相对简单。
6. **同步线程模型的局限**：无法做细粒度 timeout、graceful shutdown 和 cancellation，与 async web 层之间需要额外的桥接。
7. **兼容 re-export 层未清理**：lib.rs 中仍有 12 个旧路径兼容模块，增加了认知负担和编译警告。

### 演进方向

基于以上对比，后续优先级建议：

1. 继续拆分 CoreAgent（提取 RequirementCoordinator、RuntimeEventEmitter）
2. 统一 RuntimeStateStore 和 TaskRegistry，消除双写
3. 优化 snapshot 读取（targeted query 或 Arc copy-on-write）
4. 统一事件发布模式（消除 3 处重复的 timeline + bus publish 逻辑）
5. 清理兼容 re-export 层
6. 丰富 hooks / 插件机制

## 当前限制

有几个重要现状，值得在 README 里明确说明：

- 文档已经和当前源码结构对齐，但 runtime 某些区域仍处于持续重构中
- `anima-runtime` 里仍保留兼容旧路径的 re-export，说明代码仍处于新旧模块路径过渡期
- `anima-web` 在提供完整工作台前，必须先构建前端静态产物
- Rust workspace 的构建 / 测试命令本身是正确入口，但在本次更新时仓库里仍有进行中的代码，可能导致 `cargo build --workspace` 和 `cargo test --workspace` 失败
- 上下文压缩里的 token 估算是启发式近似，而不是上游模型提供方的精确 token 计数
- 编排、提问 / 审批流、agent loop 等能力已经存在，但仍在快速演进

## 开发者上手路径

如果你想改某个特定区域，可以这样开始：

- **改 Web UI 或工作台交互**
  - 先看 `crates/anima-web/frontend/`
  - 再看 `crates/anima-web/src/routes.rs` 和 `crates/anima-web/src/sse.rs`

- **改 runtime 的 HTTP/Web 集成**
  - 先看 `crates/anima-web/src/main.rs` 和 `crates/anima-web/src/routes.rs`
  - 再顺着 `WebChannel`、status/job API 往下追

- **改工具行为或新增内建工具**
  - 先看 `crates/anima-runtime/src/tools/definition.rs`
  - 再看 `registry.rs`、`execution.rs` 和 `tools/builtins/`

- **改权限行为**
  - 先看 `crates/anima-runtime/src/permissions/types.rs` 和 `checker.rs`
  - 再顺着 agent loop 与 agent core 看挂起 / 恢复执行

- **改 agent loop 行为**
  - 先看 `crates/anima-runtime/src/execution/agentic_loop.rs`
  - 再看消息规范化、压缩、streaming、tool execution

- **改消息 / 上下文处理**
  - 先看 `crates/anima-runtime/src/messages/` 和 `crates/anima-runtime/src/context/`

- **改编排或 follow-up 逻辑**
  - 先看 `crates/anima-runtime/src/classifier/`、`crates/anima-runtime/src/orchestrator/` 和 `crates/anima-runtime/src/agent/core.rs`

- **从 UI 侧调试 runtime 状态**
  - 重点看 `anima-web` 里的 `/api/status`、`/api/events`、job views、worker status、runtime timeline 处理

对新贡献者来说，一个比较实用的阅读顺序是：

1. `README.md`
2. `crates/anima-runtime/src/lib.rs`
3. `crates/anima-runtime/src/bootstrap.rs`
4. `crates/anima-runtime/src/execution/agentic_loop.rs`
5. `crates/anima-web/src/main.rs`
6. `crates/anima-web/src/routes.rs`

## 快速开始

### 环境要求

- Rust toolchain（workspace 使用 edition 2021）
- Node.js + npm，用于构建 Web 前端
- 如果要体验真实 Agent 响应，需要可用的 OpenCode 兼容后端

### 构建 Rust Workspace

```bash
cargo build --workspace
```

### 运行 CLI

```bash
cargo run -p anima-cli -- --help
```

### 构建 Web 前端

```bash
cd crates/anima-web/frontend
npm install
npm run build
```

### 运行 Web 应用

```bash
cargo run -p anima-web
```

然后在浏览器打开 <http://localhost:3000>。

## 开发与测试命令

### Rust Workspace

```bash
cargo build --workspace
cargo test --workspace
```

### 前端（`crates/anima-web/frontend`）

```bash
npm install
npm run dev
npm run build
npm run test
npm run lint
```

## 仓库结构

```text
anima-agent-rs/
├── Cargo.toml
├── README.md
├── README.zh-CN.md
├── docs/
│   └── main-agent-orchestration-design.md
└── crates/
    ├── anima-cli/
    ├── anima-runtime/
    │   └── src/
    │       ├── agent/
    │       │   ├── core.rs
    │       │   ├── types.rs
    │       │   ├── executor.rs
    │       │   ├── worker.rs
    │       │   ├── registry.rs
    │       │   └── suspension.rs
    │       ├── classifier/
    │       ├── orchestrator/
    │       ├── execution/
    │       ├── runtime/
    │       │   ├── events.rs
    │       │   ├── reducer.rs
    │       │   ├── snapshot.rs
    │       │   ├── projection.rs
    │       │   └── state_store.rs
    │       ├── tasks/
    │       │   ├── model.rs
    │       │   ├── lifecycle.rs
    │       │   ├── scheduler.rs
    │       │   └── query.rs
    │       ├── transcript/
    │       │   ├── model.rs
    │       │   ├── append.rs
    │       │   ├── normalizer.rs
    │       │   └── pairing.rs
    │       ├── tools/
    │       ├── streaming/
    │       ├── permissions/
    │       ├── messages/
    │       ├── hooks/
    │       ├── prompt/
    │       ├── bus/
    │       ├── cache/
    │       ├── channel/
    │       ├── context/
    │       ├── dispatcher/
    │       ├── pipeline/
    │       ├── bootstrap.rs
    │       ├── cli.rs
    │       ├── lib.rs
    │       ├── metrics.rs
    │       └── support.rs
    ├── anima-sdk/
    ├── anima-testkit/
    ├── anima-types/
    └── anima-web/
        ├── frontend/
        └── src/
            ├── main.rs
            ├── routes.rs
            ├── sse.rs
            └── static/
                └── dist/
```

## 进一步阅读

- [Main Agent / Subagent Orchestration 设计说明](./docs/main-agent-orchestration-design.md)

## 当前状态

当前仓库已经包含相当完整的 Rust runtime、CLI 与本地 Web 工作台。Runtime 已引入 event-sourced 状态管理（`runtime/`）、结构化领域模型（`tasks/`、`transcript/`），并开始将 CoreAgent 的职责拆分为独立 coordinator（已完成 `SuspensionCoordinator`，`RequirementCoordinator` 和 `RuntimeEventEmitter` 在计划中）。整体上它已经是一个可用的 agent runtime 代码库，但部分子系统仍在持续演进。

## License

MIT
