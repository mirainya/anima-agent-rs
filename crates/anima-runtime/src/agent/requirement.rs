//! Requirement 协调器：管理 RequirementProgressState 的生命周期

use parking_lot::Mutex;
use std::collections::HashMap;

use crate::bus::InboundMessage;
use crate::execution::requirement_judge::RequirementProgressState;
use crate::runtime::{RuntimeDomainEvent, RuntimeStateSnapshot, SharedRuntimeStateStore};
use crate::support::now_ms;
use crate::tasks::{RequirementRecord, RequirementStatus, TaskStatus};

use super::context_types::RuntimeTaskPhase;
use super::event_emitter::RuntimeEventEmitter;
use super::runtime_ids::{runtime_requirement_id, runtime_run_id};

pub struct RequirementCoordinator {
    runtime_state_store: SharedRuntimeStateStore,
    emitter: std::sync::Arc<RuntimeEventEmitter>,
    requirement_progress: Mutex<HashMap<String, RequirementProgressState>>,
}

impl RequirementCoordinator {
    pub fn new(
        runtime_state_store: SharedRuntimeStateStore,
        emitter: std::sync::Arc<RuntimeEventEmitter>,
    ) -> Self {
        Self {
            runtime_state_store,
            emitter,
            requirement_progress: Mutex::new(HashMap::new()),
        }
    }

    pub fn rebuild(
        snapshot: &RuntimeStateSnapshot,
        job_id: &str,
    ) -> Option<RequirementProgressState> {
        let requirement = snapshot.requirements.get(&runtime_requirement_id(job_id))?;
        Some(RequirementProgressState {
            original_user_request: requirement.original_user_request.clone(),
            attempted_rounds: requirement.attempted_rounds,
            max_rounds: requirement.max_rounds,
            last_result_fingerprint: requirement.last_result_fingerprint.clone(),
            last_reason: requirement.last_reason.clone(),
        })
    }

    pub fn state(&self, job_id: &str, hydrate_cache: bool) -> Option<RequirementProgressState> {
        let snapshot = self.runtime_state_store.snapshot();
        if snapshot
            .requirements
            .contains_key(&runtime_requirement_id(job_id))
        {
            let rebuilt = Self::rebuild(&snapshot, job_id);
            let mut requirement_progress = self.requirement_progress.lock();
            match (&rebuilt, hydrate_cache) {
                (Some(progress), true) => {
                    requirement_progress.insert(job_id.to_string(), progress.clone());
                }
                (Some(_), false) => {}
                (None, _) => {
                    requirement_progress.remove(job_id);
                }
            }
            return rebuilt;
        }
        self.requirement_progress.lock().get(job_id).cloned()
    }

    pub fn ensure(
        &self,
        inbound_msg: &InboundMessage,
        default_max_rounds: usize,
    ) -> RequirementProgressState {
        let entry = self
            .state(&inbound_msg.id, true)
            .unwrap_or_else(|| RequirementProgressState {
                original_user_request: inbound_msg.content.clone(),
                attempted_rounds: 0,
                max_rounds: default_max_rounds,
                last_result_fingerprint: None,
                last_reason: None,
            });
        self.requirement_progress
            .lock()
            .insert(inbound_msg.id.clone(), entry.clone());
        self.emitter.upsert_task(
            inbound_msg,
            RuntimeTaskPhase::Requirement,
            TaskStatus::Running,
            "evaluating requirement progress",
            None,
        );
        self.upsert_runtime_requirement(inbound_msg, &entry, RequirementStatus::Active);
        entry
    }

    pub fn update(
        &self,
        job_id: &str,
        attempted_rounds: usize,
        last_result_fingerprint: Option<String>,
        last_reason: Option<String>,
    ) {
        if let Some(mut progress) = self.state(job_id, true) {
            progress.attempted_rounds = attempted_rounds;
            progress.last_result_fingerprint = last_result_fingerprint.clone();
            progress.last_reason = last_reason.clone();
            self.requirement_progress
                .lock()
                .insert(job_id.to_string(), progress.clone());
            if let Some(inbound) = self.rebuild_inbound_from_snapshot(job_id, &progress) {
                self.upsert_runtime_requirement(
                    &inbound,
                    &progress,
                    RequirementStatus::FollowupScheduled,
                );
            }
        }
    }

    pub fn clear(&self, job_id: &str) {
        self.requirement_progress.lock().remove(job_id);
    }

    pub fn upsert_runtime_requirement(
        &self,
        inbound_msg: &InboundMessage,
        progress: &RequirementProgressState,
        status: RequirementStatus,
    ) -> String {
        let requirement_id = runtime_requirement_id(&inbound_msg.id);
        let snapshot = self.runtime_state_store.snapshot();
        let created_at_ms = snapshot
            .requirements
            .get(&requirement_id)
            .map(|record| record.created_at_ms)
            .unwrap_or_else(now_ms);
        self.runtime_state_store
            .append(RuntimeDomainEvent::RequirementUpserted {
                requirement: RequirementRecord {
                    requirement_id: requirement_id.clone(),
                    run_id: runtime_run_id(&inbound_msg.id),
                    turn_id: snapshot
                        .runs
                        .get(&runtime_run_id(&inbound_msg.id))
                        .and_then(|run| run.current_turn_id.clone()),
                    job_id: inbound_msg.id.clone(),
                    original_user_request: progress.original_user_request.clone(),
                    attempted_rounds: progress.attempted_rounds,
                    max_rounds: progress.max_rounds,
                    last_result_fingerprint: progress.last_result_fingerprint.clone(),
                    last_reason: progress.last_reason.clone(),
                    status,
                    created_at_ms,
                    updated_at_ms: now_ms(),
                },
            });
        requirement_id
    }

    fn rebuild_inbound_from_snapshot(
        &self,
        job_id: &str,
        progress: &RequirementProgressState,
    ) -> Option<InboundMessage> {
        let snapshot = self.runtime_state_store.snapshot();
        let run = snapshot.runs.get(&runtime_run_id(job_id))?;
        Some(InboundMessage {
            id: job_id.to_string(),
            channel: run.channel.clone(),
            sender_id: String::new(),
            chat_id: run.chat_id.clone(),
            content: progress.original_user_request.clone(),
            session_key: None,
            media: vec![],
            metadata: serde_json::json!({}),
        })
    }
}
