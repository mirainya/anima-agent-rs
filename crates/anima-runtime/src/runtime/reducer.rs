use crate::runtime::events::RuntimeDomainEvent;
use crate::runtime::snapshot::{RuntimeDomainEventEnvelope, RuntimeStateSnapshot};

pub fn reduce_event(
    state: &mut RuntimeStateSnapshot,
    event: RuntimeDomainEvent,
    sequence: u64,
    recorded_at_ms: u64,
) {
    let event_type = match &event {
        RuntimeDomainEvent::RunUpserted { .. } => "run_upserted",
        RuntimeDomainEvent::TurnUpserted { .. } => "turn_upserted",
        RuntimeDomainEvent::TaskUpserted { .. } => "task_upserted",
        RuntimeDomainEvent::SuspensionUpserted { .. } => "suspension_upserted",
        RuntimeDomainEvent::ToolInvocationUpserted { .. } => "tool_invocation_upserted",
        RuntimeDomainEvent::RequirementUpserted { .. } => "requirement_upserted",
        RuntimeDomainEvent::MessageAppended { .. } => "message_appended",
        RuntimeDomainEvent::ProjectionHintRecorded { .. } => "projection_hint_recorded",
    }
    .to_string();

    match event {
        RuntimeDomainEvent::RunUpserted { run } => {
            state
                .index
                .run_ids_by_job_id
                .insert(run.job_id.clone(), run.run_id.clone());
            state.runs.insert(run.run_id.clone(), run);
        }
        RuntimeDomainEvent::TurnUpserted { turn } => {
            if let Some(run) = state.runs.get_mut(&turn.run_id) {
                run.current_turn_id = Some(turn.turn_id.clone());
                run.updated_at_ms = turn.updated_at_ms;
            }
            state.turns.insert(turn.turn_id.clone(), turn);
        }
        RuntimeDomainEvent::TaskUpserted { task } => {
            if let Some(plan_id) = task.plan_id.clone() {
                let task_ids = state.index.task_ids_by_plan_id.entry(plan_id).or_default();
                if !task_ids.iter().any(|id| id == &task.task_id) {
                    task_ids.push(task.task_id.clone());
                }
            }
            state.tasks.insert(task.task_id.clone(), task);
        }
        RuntimeDomainEvent::SuspensionUpserted { suspension } => {
            if let Some(question_id) = suspension.question_id.clone() {
                state
                    .index
                    .suspension_ids_by_question_id
                    .insert(question_id.clone(), suspension.suspension_id.clone());
                if let Some(invocation_id) = suspension.invocation_id.clone() {
                    state
                        .index
                        .invocation_ids_by_question_id
                        .insert(question_id, invocation_id);
                }
            }
            state
                .suspensions
                .insert(suspension.suspension_id.clone(), suspension);
        }
        RuntimeDomainEvent::ToolInvocationUpserted { invocation } => {
            state
                .tool_invocations
                .insert(invocation.invocation_id.clone(), invocation);
        }
        RuntimeDomainEvent::RequirementUpserted { requirement } => {
            state
                .requirements
                .insert(requirement.requirement_id.clone(), requirement);
        }
        RuntimeDomainEvent::MessageAppended { message } => {
            state.transcript.push(message);
        }
        RuntimeDomainEvent::ProjectionHintRecorded {
            run_id,
            scope,
            key,
            value,
        } => {
            let scoped = state.projection_hints.entry(run_id).or_default();
            scoped.insert(format!("{scope}.{key}"), value);
        }
    }

    state.recent_events.push(RuntimeDomainEventEnvelope {
        sequence,
        recorded_at_ms,
        event_type,
    });
    if state.recent_events.len() > 200 {
        let excess = state.recent_events.len() - 200;
        state.recent_events.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::types::ContentBlock;
    use crate::tasks::model::*;
    use crate::transcript::model::MessageRecord;
    use serde_json::json;

    fn make_run(run_id: &str, job_id: &str) -> RunRecord {
        RunRecord {
            run_id: run_id.into(),
            trace_id: "t".into(),
            job_id: job_id.into(),
            chat_id: None,
            channel: "ch".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: 100,
            updated_at_ms: 100,
            completed_at_ms: None,
        }
    }

    fn make_turn(turn_id: &str, run_id: &str) -> TurnRecord {
        TurnRecord {
            turn_id: turn_id.into(),
            run_id: run_id.into(),
            source: "test".into(),
            status: TurnStatus::Running,
            transcript_checkpoint: 0,
            requirement_id: None,
            suspension_id: None,
            started_at_ms: 200,
            updated_at_ms: 200,
            completed_at_ms: None,
        }
    }

    fn make_task(task_id: &str, plan_id: Option<&str>) -> TaskRecord {
        TaskRecord {
            task_id: task_id.into(),
            run_id: "r1".into(),
            turn_id: None,
            parent_task_id: None,
            trace_id: "t".into(),
            job_id: "j".into(),
            parent_job_id: None,
            plan_id: plan_id.map(Into::into),
            kind: TaskKind::Main,
            name: "n".into(),
            task_type: "main".into(),
            description: "d".into(),
            status: TaskStatus::Pending,
            execution_mode: None,
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: None,
            updated_at_ms: 0,
            completed_at_ms: None,
            error: None,
        }
    }

    fn apply(state: &mut RuntimeStateSnapshot, event: RuntimeDomainEvent) {
        reduce_event(state, event, 1, 1000);
    }

    #[test]
    fn run_upserted_inserts_run_and_index() {
        let mut state = RuntimeStateSnapshot::default();
        apply(
            &mut state,
            RuntimeDomainEvent::RunUpserted {
                run: make_run("r1", "j1"),
            },
        );
        assert!(state.runs.contains_key("r1"));
        assert_eq!(state.index.run_ids_by_job_id.get("j1"), Some(&"r1".into()));
    }

    #[test]
    fn turn_upserted_updates_parent_run() {
        let mut state = RuntimeStateSnapshot::default();
        apply(
            &mut state,
            RuntimeDomainEvent::RunUpserted {
                run: make_run("r1", "j1"),
            },
        );
        apply(
            &mut state,
            RuntimeDomainEvent::TurnUpserted {
                turn: make_turn("t1", "r1"),
            },
        );
        assert_eq!(state.runs["r1"].current_turn_id, Some("t1".into()));
        assert!(state.turns.contains_key("t1"));
    }

    #[test]
    fn task_upserted_maintains_plan_index_dedup() {
        let mut state = RuntimeStateSnapshot::default();
        let task = make_task("tk1", Some("plan_a"));
        apply(
            &mut state,
            RuntimeDomainEvent::TaskUpserted { task: task.clone() },
        );
        apply(&mut state, RuntimeDomainEvent::TaskUpserted { task });
        let ids = &state.index.task_ids_by_plan_id["plan_a"];
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "tk1");
    }

    #[test]
    fn task_without_plan_id_skips_index() {
        let mut state = RuntimeStateSnapshot::default();
        apply(
            &mut state,
            RuntimeDomainEvent::TaskUpserted {
                task: make_task("tk1", None),
            },
        );
        assert!(state.index.task_ids_by_plan_id.is_empty());
        assert!(state.tasks.contains_key("tk1"));
    }

    #[test]
    fn suspension_upserted_indexes_question_and_invocation() {
        let mut state = RuntimeStateSnapshot::default();
        let suspension = SuspensionRecord {
            suspension_id: "s1".into(),
            run_id: "r1".into(),
            turn_id: "t1".into(),
            task_id: None,
            question_id: Some("q1".into()),
            invocation_id: Some("inv1".into()),
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Active,
            prompt: None,
            options: vec![],
            raw_payload: json!({}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            resolved_at_ms: None,
            cleared_at_ms: None,
        };
        apply(
            &mut state,
            RuntimeDomainEvent::SuspensionUpserted { suspension },
        );
        assert_eq!(
            state.index.suspension_ids_by_question_id.get("q1"),
            Some(&"s1".into())
        );
        assert_eq!(
            state.index.invocation_ids_by_question_id.get("q1"),
            Some(&"inv1".into())
        );
    }

    #[test]
    fn tool_invocation_upserted() {
        let mut state = RuntimeStateSnapshot::default();
        let inv = ToolInvocationRuntimeRecord {
            invocation_id: "inv1".into(),
            run_id: "r1".into(),
            turn_id: None,
            task_id: None,
            tool_name: "bash".into(),
            tool_use_id: None,
            phase: "running".into(),
            permission_state: "allowed".into(),
            input_preview: None,
            result_summary: None,
            error_summary: None,
            started_at_ms: 0,
            finished_at_ms: None,
        };
        apply(
            &mut state,
            RuntimeDomainEvent::ToolInvocationUpserted { invocation: inv },
        );
        assert!(state.tool_invocations.contains_key("inv1"));
    }

    #[test]
    fn requirement_upserted() {
        let mut state = RuntimeStateSnapshot::default();
        let req = RequirementRecord {
            requirement_id: "req1".into(),
            run_id: "r1".into(),
            turn_id: None,
            job_id: "j1".into(),
            original_user_request: "hello".into(),
            attempted_rounds: 0,
            max_rounds: 3,
            last_result_fingerprint: None,
            last_reason: None,
            status: RequirementStatus::Active,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        apply(
            &mut state,
            RuntimeDomainEvent::RequirementUpserted { requirement: req },
        );
        assert!(state.requirements.contains_key("req1"));
    }

    #[test]
    fn message_appended() {
        let mut state = RuntimeStateSnapshot::default();
        let msg = MessageRecord {
            message_id: "m1".into(),
            run_id: "r1".into(),
            turn_id: None,
            role: crate::messages::types::MessageRole::User,
            blocks: vec![ContentBlock::Text { text: "hi".into() }],
            tool_use_id: None,
            metadata: json!({}),
            filtered: false,
            appended_at_ms: 0,
        };
        apply(
            &mut state,
            RuntimeDomainEvent::MessageAppended { message: msg },
        );
        assert_eq!(state.transcript.len(), 1);
        assert_eq!(state.transcript[0].message_id, "m1");
    }

    #[test]
    fn projection_hint_recorded_uses_scope_dot_key() {
        let mut state = RuntimeStateSnapshot::default();
        apply(
            &mut state,
            RuntimeDomainEvent::ProjectionHintRecorded {
                run_id: "r1".into(),
                scope: "job".into(),
                key: "lifecycle".into(),
                value: json!({"status": "done"}),
            },
        );
        let hints = &state.projection_hints["r1"];
        assert!(hints.contains_key("job.lifecycle"));
    }

    #[test]
    fn recent_events_capped_at_200() {
        let mut state = RuntimeStateSnapshot::default();
        for i in 0..210 {
            let msg = MessageRecord {
                message_id: format!("m{i}"),
                run_id: "r1".into(),
                turn_id: None,
                role: crate::messages::types::MessageRole::User,
                blocks: vec![],
                tool_use_id: None,
                metadata: json!({}),
                filtered: false,
                appended_at_ms: 0,
            };
            reduce_event(
                &mut state,
                RuntimeDomainEvent::MessageAppended { message: msg },
                i as u64,
                1000,
            );
        }
        assert_eq!(state.recent_events.len(), 200);
        assert_eq!(state.recent_events[0].sequence, 10);
    }
}
