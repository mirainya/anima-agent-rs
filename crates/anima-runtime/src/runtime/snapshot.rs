use crate::tasks::model::{
    RequirementRecord, RunRecord, RuntimeTaskIndex, SuspensionRecord, TaskRecord,
    ToolInvocationRuntimeRecord, TurnRecord,
};
use crate::transcript::model::MessageRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RuntimeStateSnapshot {
    pub runs: HashMap<String, RunRecord>,
    pub turns: HashMap<String, TurnRecord>,
    pub tasks: HashMap<String, TaskRecord>,
    pub suspensions: HashMap<String, SuspensionRecord>,
    pub tool_invocations: HashMap<String, ToolInvocationRuntimeRecord>,
    pub requirements: HashMap<String, RequirementRecord>,
    pub transcript: Vec<MessageRecord>,
    pub recent_events: Vec<RuntimeDomainEventEnvelope>,
    pub projection_hints: HashMap<String, HashMap<String, Value>>,
    pub index: RuntimeTaskIndex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDomainEventEnvelope {
    pub sequence: u64,
    pub recorded_at_ms: u64,
    pub event_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedRuntimeState {
    pub version: u32,
    pub next_sequence: u64,
    pub snapshot: RuntimeStateSnapshot,
}
