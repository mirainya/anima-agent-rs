use crate::agent::types::{make_task, ExecutionPlan, ExecutionPlanKind, MakeTask};
use crate::bus::InboundMessage;
use serde_json::{json, Value};
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationKind {
    Direct,
    Single,
    SpecialistRoute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationDecision {
    pub kind: ClassificationKind,
}

pub struct AgentClassifier;

impl AgentClassifier {
    pub fn classify(inbound_msg: &InboundMessage) -> ClassificationDecision {
        let is_system = inbound_msg
            .metadata
            .get("system-command")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let specialist = inbound_msg
            .metadata
            .get("specialist")
            .and_then(Value::as_str);

        if is_system {
            ClassificationDecision {
                kind: ClassificationKind::Direct,
            }
        } else if specialist.is_some() {
            ClassificationDecision {
                kind: ClassificationKind::SpecialistRoute,
            }
        } else {
            ClassificationDecision {
                kind: ClassificationKind::Single,
            }
        }
    }

    pub fn build_plan(inbound_msg: &InboundMessage) -> ExecutionPlan {
        match Self::classify(inbound_msg).kind {
            ClassificationKind::Direct => ExecutionPlan {
                kind: ExecutionPlanKind::Direct,
                plan_type: "direct".into(),
                tasks: VecDeque::new(),
                specialist: None,
            },
            ClassificationKind::SpecialistRoute => ExecutionPlan {
                kind: ExecutionPlanKind::SpecialistRoute,
                plan_type: "specialist-route".into(),
                tasks: VecDeque::from(vec![make_task(MakeTask {
                    trace_id: Some(inbound_msg.id.clone()),
                    task_type: "api-call".into(),
                    payload: Some(json!({"content": inbound_msg.content.clone()})),
                    metadata: Some(json!({
                        "specialist": inbound_msg
                            .metadata
                            .get("specialist")
                            .and_then(Value::as_str)
                            .unwrap_or("default")
                    })),
                    ..Default::default()
                })]),
                specialist: inbound_msg
                    .metadata
                    .get("specialist")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            },
            ClassificationKind::Single => ExecutionPlan {
                kind: ExecutionPlanKind::Single,
                plan_type: "single".into(),
                tasks: VecDeque::from(vec![make_task(MakeTask {
                    trace_id: Some(inbound_msg.id.clone()),
                    task_type: "api-call".into(),
                    payload: Some(json!({"content": inbound_msg.content.clone()})),
                    ..Default::default()
                })]),
                specialist: None,
            },
        }
    }
}
