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
