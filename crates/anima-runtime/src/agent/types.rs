//! Agent 核心类型定义
//!
//! 本模块的纯数据类型已下沉到 `anima-types::task`，此处仅做 re-export，
//! 保持 `crate::agent::types::*` 路径可用。

pub use anima_types::task::{
    make_task, make_task_result, ExecutionPlan, ExecutionPlanKind, MakeTask, MakeTaskResult, Task,
    TaskResult,
};
