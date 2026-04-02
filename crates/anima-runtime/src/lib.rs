//! # Anima Runtime
//!
//! Anima 智能体运行时核心库。
//!
//! 提供智能体（Agent）的完整生命周期管理，包括：
//! - 核心智能体（CoreAgent）：消息收发、会话管理、任务调度
//! - 工作池（WorkerPool）：多 Worker 并发执行任务
//! - 消息总线（Bus）：入站/出站消息的发布订阅
//! - 分类与路由：任务分类、智能路由、编排调度
//! - 基础设施：缓存、指标采集、上下文管理

/// Agent 核心域：智能体引擎、任务类型、执行器、Worker 池
pub mod agent;
/// 分类与路由域：规则分类、AI 分类、任务分类、智能路由
pub mod classifier;
/// 编排与调度域：编排引擎、并行池、专家池
pub mod orchestrator;
/// 执行循环域：执行驱动、回合协调、上下文装配、需求判定
pub mod execution;

/// 消息总线，入站/出站消息的发布订阅机制
pub mod bus;
/// 缓存模块，提供 LRU 缓存等基础能力
pub mod cache;
/// 会话通道，管理多渠道的会话连接
pub mod channel;
/// 上下文管理，维护对话上下文与变量
pub mod context;
/// 消息分发器，将入站消息路由到对应处理器
pub mod dispatcher;
/// 处理管线，定义消息处理的流水线
pub mod pipeline;

/// 工具注册与执行闭环
pub mod tools;
/// 流式 API 解析与流式工具执行
pub mod streaming;
/// 权限判定系统
pub mod permissions;
/// 消息协议三层映射
pub mod messages;
/// Pre/Post 钩子机制
pub mod hooks;

/// 启动引导，初始化运行时各组件
pub mod bootstrap;
/// CLI 命令行接口
pub mod cli;
/// 指标采集与上报
pub mod metrics;
/// 通用工具集：时间、缓存、指标等
pub mod support;

// ---- 向后兼容的 re-export（过渡期） ----

/// 兼容旧路径 `anima_runtime::agent_types::*`
pub mod agent_types {
    pub use crate::agent::types::*;
}
/// 兼容旧路径 `anima_runtime::agent_executor::*`
pub mod agent_executor {
    pub use crate::agent::executor::*;
}
/// 兼容旧路径 `anima_runtime::agent_worker::*`
pub mod agent_worker {
    pub use crate::agent::worker::*;
}
/// 兼容旧路径 `anima_runtime::agent_classifier::*`
pub mod agent_classifier {
    pub use crate::classifier::rule::*;
}
/// 兼容旧路径 `anima_runtime::ai_classifier::*`
pub mod ai_classifier {
    pub use crate::classifier::ai::*;
}
/// 兼容旧路径 `anima_runtime::task_classifier::*`
pub mod task_classifier {
    pub use crate::classifier::task::*;
}
/// 兼容旧路径 `anima_runtime::agent_intelligent_router::*`
pub mod agent_intelligent_router {
    pub use crate::classifier::router::*;
}
/// 兼容旧路径 `anima_runtime::agent_orchestrator::*`
pub mod agent_orchestrator {
    pub use crate::orchestrator::core::*;
}
/// 兼容旧路径 `anima_runtime::agent_parallel_pool::*`
pub mod agent_parallel_pool {
    pub use crate::orchestrator::parallel_pool::*;
}
/// 兼容旧路径 `anima_runtime::agent_specialist_pool::*`
pub mod agent_specialist_pool {
    pub use crate::orchestrator::specialist_pool::*;
}
/// 兼容旧路径 `anima_runtime::execution_driver::*`
pub mod execution_driver {
    pub use crate::execution::driver::*;
}
/// 兼容旧路径 `anima_runtime::turn_coordinator::*`
pub mod turn_coordinator {
    pub use crate::execution::turn_coordinator::*;
}
/// 兼容旧路径 `anima_runtime::context_assembly::*`
pub mod context_assembly {
    pub use crate::execution::context_assembly::*;
}
/// 兼容旧路径 `anima_runtime::requirement_judge::*`
pub mod requirement_judge {
    pub use crate::execution::requirement_judge::*;
}

pub use agent::*;
pub use cache::*;
pub use channel::*;
pub use cli::*;
pub use support::*;
