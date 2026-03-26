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

/// 核心智能体，负责消息循环、会话管理和任务调度
pub mod agent;
/// 任务分类器，根据任务内容决定执行策略
pub mod agent_classifier;
/// 任务执行器 trait 及 SDK 默认实现
pub mod agent_executor;
/// 智能路由器，根据上下文动态选择处理路径
pub mod agent_intelligent_router;
/// 编排器，管理多步骤任务的执行流程
pub mod agent_orchestrator;
/// 并行任务池，支持多任务并发执行与结果聚合
pub mod agent_parallel_pool;
/// 专家池，管理不同领域的专业化 Worker
pub mod agent_specialist_pool;
/// 任务与结果的核心数据类型定义
pub mod agent_types;
/// Worker 及 WorkerPool，负责任务的实际执行
pub mod agent_worker;
/// AI 分类器，基于 LLM 的意图识别
pub mod ai_classifier;
/// 启动引导，初始化运行时各组件
pub mod bootstrap;
/// 消息总线，入站/出站消息的发布订阅机制
pub mod bus;
/// 缓存模块，提供 LRU 缓存等基础能力
pub mod cache;
/// 会话通道，管理多渠道的会话连接
pub mod channel;
/// CLI 命令行接口
pub mod cli;
/// 上下文管理，维护对话上下文与变量
pub mod context;
/// 消息分发器，将入站消息路由到对应处理器
pub mod dispatcher;
/// 指标采集与上报
pub mod metrics;
/// 处理管线，定义消息处理的流水线
pub mod pipeline;
/// 通用工具集：时间、缓存、指标等
pub mod support;
/// 任务分类器，基于规则的任务类型判定
pub mod task_classifier;

pub use agent::*;
pub use cache::*;
pub use channel::*;
pub use cli::*;
pub use support::*;
