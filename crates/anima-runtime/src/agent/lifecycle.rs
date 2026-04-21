//! CoreAgent 生命周期管理：启停、状态查询、运维方法

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::core::{CoreAgent, CoreAgentStatus};
use crate::bus::ControlSignal;
use crate::hooks::HookRegistry;
use crate::permissions::PermissionChecker;
use crate::runtime::{build_projection, RuntimeProjectionView, RuntimeStateSnapshot};

impl CoreAgent {
    pub fn register_builtin_tools(&mut self) {
        let registry = Arc::get_mut(&mut self.tool_registry)
            .expect("cannot mutate shared tool_registry — ensure no other Arc references exist");
        crate::tools::builtins::register_all(registry);
    }

    pub fn set_tool_registry(&mut self, registry: crate::tools::registry::ToolRegistry) {
        self.tool_registry = Arc::new(registry);
    }

    pub fn set_permission_checker(&mut self, checker: PermissionChecker) {
        self.permission_checker = Some(Arc::new(checker));
    }

    pub fn set_hook_registry(&mut self, registry: HookRegistry) {
        self.hook_registry = Some(Arc::new(registry));
    }

    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        tracing::info!("CoreAgent starting");
        self.worker_pool.start();
        self.orchestrator.start();

        let agent_ctrl = Arc::clone(self);
        let ctrl_handle = thread::spawn(move || {
            let control_rx = agent_ctrl.bus.control_receiver();
            while agent_ctrl.running.load(Ordering::SeqCst) {
                match control_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(msg) => match msg.signal {
                        ControlSignal::Shutdown => {
                            agent_ctrl.running.store(false, Ordering::SeqCst);
                            break;
                        }
                        ControlSignal::Pause => {
                            agent_ctrl.metrics.counter_inc("agent.paused");
                        }
                        ControlSignal::Resume => {
                            agent_ctrl.metrics.counter_inc("agent.resumed");
                        }
                        _ => {}
                    },
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if agent_ctrl.bus.is_closed() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        let agent = Arc::clone(self);
        let handle = thread::spawn(move || {
            let inbound_rx = agent.bus.inbound_receiver();
            while agent.running.load(Ordering::SeqCst) {
                match inbound_rx.recv_timeout(Duration::from_millis(25)) {
                    Ok(msg) => {
                        let _span = tracing::info_span!(
                            "process_inbound",
                            channel = %msg.channel,
                            msg_id = %msg.id,
                        )
                        .entered();
                        agent.process_inbound_message(msg);
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if agent.bus.is_closed() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        *self.loop_handle.lock() = Some(handle);
        *self.control_handle.lock() = Some(ctrl_handle);
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        tracing::info!("CoreAgent stopping");
        self.worker_pool.stop();
        self.orchestrator.stop();
        if let Some(handle) = self.loop_handle.lock().take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.control_handle.lock().take() {
            let _ = handle.join();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> CoreAgentStatus {
        let memory = self.memory.lock();
        CoreAgentStatus {
            status: if self.is_running() {
                "running"
            } else {
                "stopped"
            }
            .into(),
            sessions_count: memory.len(),
            worker_pool: self.worker_pool.status(),
            context_status: self.context_manager.status().status,
            cache_entries: self.result_cache.stats().entry_count,
            metrics: self.metrics.snapshot(),
            recent_sessions: memory.values().rev().take(5).cloned().collect(),
            failures: self.emitter.failures_snapshot(),
            runtime_timeline: self.emitter.timeline_snapshot(),
            recent_execution_summaries: self.emitter.execution_summaries_snapshot(),
            tool_count: self.tool_registry.len(),
            pre_hook_count: self
                .hook_registry
                .as_ref()
                .map(|registry| registry.pre_hook_count())
                .unwrap_or(0),
            post_hook_count: self
                .hook_registry
                .as_ref()
                .map(|registry| registry.post_hook_count())
                .unwrap_or(0),
        }
    }

    pub fn runtime_state_snapshot(&self) -> RuntimeStateSnapshot {
        self.runtime_state_store.snapshot()
    }

    pub fn runtime_projection_snapshot(&self) -> RuntimeProjectionView {
        build_projection(&self.runtime_state_store.snapshot())
    }

    #[doc(hidden)]
    pub fn evict_resume_state_cache_for_testing(&self, job_id: &str) {
        self.suspension.evict_cache_for_testing(job_id);
    }
}
