use crate::agent::types::{make_task, ExecutionPlan, ExecutionPlanKind, MakeTask};
use crate::bus::InboundMessage;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::VecDeque;

lazy_static! {
    static ref ORCHESTRATION_V1_COMPLEX_REQUEST_RULES: Vec<Regex> = vec![
        Regex::new(r"(?i)web.?app|website|frontend").unwrap(),
        Regex::new(r"(?i)api|endpoint|rest").unwrap(),
        Regex::new(r"(?i)data.?analy|report|dashboard").unwrap(),
        Regex::new(r"(?i)refactor|restructure|clean.?up").unwrap(),
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationKind {
    Direct,
    Single,
    Sequential,
    Parallel,
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
        let is_multi_step = inbound_msg
            .metadata
            .get("multi-step")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_parallel = inbound_msg
            .metadata
            .get("parallel")
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
        } else if is_parallel {
            ClassificationDecision {
                kind: ClassificationKind::Parallel,
            }
        } else if is_multi_step {
            ClassificationDecision {
                kind: ClassificationKind::Sequential,
            }
        } else {
            ClassificationDecision {
                kind: ClassificationKind::Single,
            }
        }
    }

    pub fn should_upgrade_to_orchestration_v1(inbound_msg: &InboundMessage) -> bool {
        let force = inbound_msg
            .metadata
            .get("orchestration")
            .and_then(Value::as_str)
            == Some("v1")
            || inbound_msg
                .metadata
                .get("orchestration-v1")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if force {
            return true;
        }

        if inbound_msg.channel != "web" {
            return false;
        }

        let disabled = inbound_msg
            .metadata
            .get("disable-orchestration")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if disabled {
            return false;
        }

        ORCHESTRATION_V1_COMPLEX_REQUEST_RULES
            .iter()
            .any(|rule| rule.is_match(&inbound_msg.content))
    }

    pub fn build_plan(inbound_msg: &InboundMessage) -> ExecutionPlan {
        match Self::classify(inbound_msg).kind {
            ClassificationKind::Direct => ExecutionPlan {
                kind: ExecutionPlanKind::Direct,
                plan_type: "direct".into(),
                tasks: VecDeque::new(),
                specialist: None,
            },
            ClassificationKind::Sequential => ExecutionPlan {
                kind: ExecutionPlanKind::Sequential,
                plan_type: "sequential".into(),
                tasks: VecDeque::from(vec![
                    make_task(MakeTask {
                        trace_id: Some(inbound_msg.id.clone()),
                        task_type: "session-create".into(),
                        ..Default::default()
                    }),
                    make_task(MakeTask {
                        trace_id: Some(inbound_msg.id.clone()),
                        task_type: "api-call".into(),
                        payload: Some(json!({"content": inbound_msg.content.clone()})),
                        ..Default::default()
                    }),
                ]),
                specialist: None,
            },
            ClassificationKind::Parallel => ExecutionPlan {
                kind: ExecutionPlanKind::Parallel,
                plan_type: "parallel".into(),
                tasks: VecDeque::from(vec![
                    make_task(MakeTask {
                        trace_id: Some(inbound_msg.id.clone()),
                        task_type: "transform".into(),
                        payload: Some(
                            json!({"data": {"content": inbound_msg.content.clone(), "branch": 1}}),
                        ),
                        ..Default::default()
                    }),
                    make_task(MakeTask {
                        trace_id: Some(inbound_msg.id.clone()),
                        task_type: "transform".into(),
                        payload: Some(
                            json!({"data": {"content": inbound_msg.content.clone(), "branch": 2}}),
                        ),
                        ..Default::default()
                    }),
                ]),
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
