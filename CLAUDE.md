# Anima-Agent-RS 开发指导

## 项目定位
Rust 实现的可观测、可调度、可扩展的 AI Agent 运行时框架。对标 claude-code 的 runtime 架构。

## 构建与验证
```bash
cargo build --workspace          # 全量构建
cargo test --workspace           # 全量测试（回归验证）
cargo test -p anima-runtime      # runtime 单独测试
cargo clippy --workspace         # lint 检查
```

## Workspace 结构
```
anima-types     → 共享类型（最底层，零业务逻辑）
anima-sdk       → HTTP SDK，对接 OpenCode 后端
anima-runtime   → 核心运行时（agent/bus/channel/classifier/orchestrator/execution/tools/streaming）
anima-cli       → CLI 入口
anima-web       → Axum Web + React 前端 + SSE
anima-testkit   → 测试工具
```

## 架构约定
- Runtime 层使用同步模型：`std::thread` + `crossbeam-channel`
- Web 层使用 `tokio` + `axum`，通过 `block_on` 桥接
- 锁统一使用 `parking_lot::Mutex`
- 消息通信走 Bus 四通道（inbound/outbound/internal/control）
- 新模块放 `crates/anima-runtime/src/<domain>/` 下

## 代码规范
- 可见性：内部类型用 `pub(crate)`，仅对外契约用 `pub`
- 错误：统一使用 `RuntimeError`（runtime 层）/ `AnimaError`（types 层）
- 命名：模块用蛇形，类型用大驼峰，常量用全大写
- 注释：仅在 WHY 不明显时添加，不解释 WHAT
- commit：Conventional Commits 格式，中文描述

## 当前技术债（按优先级）

### P0 — 立即处理
1. 清理 lib.rs 中 14 个兼容旧路径 re-export
2. 收紧 `pub use *` glob 导出为显式导出
3. 将内部类型降为 `pub(crate)`

### P1 — 本周
4. 引入 `tracing` 替代手动事件发射
5. 拆分 agent 模块（7782 行过重）

### P2 — 本月
6. 异步化 SDK HTTP 调用
7. 引入 SQLite 本地持久化
8. 配置系统（TOML）

## 测试约定
- 集成测试放 `crates/<crate>/tests/`
- 单元测试用 `#[cfg(test)]` 内联
- 新功能必须有对应测试
- 回归命令：`cargo test --workspace`
