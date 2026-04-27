//! 审批门类型定义
//!
//! 定义审批模式、计划提案和审批结果等核心数据结构。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::task::ExecutionPlan;

/// 审批模式：控制 Agent 在关键决策点是否暂停等待用户审批
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// 全自动，零干预
    #[default]
    Auto,
    /// 任务拆分需审批，高风险工具需审批
    Supervised,
    /// 所有决策点都需审批
    Manual,
}

/// 计划提案：Agent 产出的执行计划，等待用户审批
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanProposal {
    pub proposal_id: String,
    pub job_id: String,
    pub plan: ExecutionPlan,
    pub summary: String,
    pub proposed_at_ms: u64,
}

impl PlanProposal {
    pub fn from_execution_plan(plan: &ExecutionPlan) -> Self {
        let summary = format!(
            "{}（{} 个任务）",
            plan.plan_type,
            plan.tasks.len()
        );
        Self {
            proposal_id: Uuid::new_v4().to_string(),
            job_id: String::new(),
            plan: plan.clone(),
            summary,
            proposed_at_ms: 0,
        }
    }
}

/// 用户对计划提案的审批结果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum PlanVerdict {
    Approved,
    Modified { modified_plan: ExecutionPlan },
    Rejected { reason: String },
}
