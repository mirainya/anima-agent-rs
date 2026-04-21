//! Agent 门面：对外公共 API 层

use anima_sdk::facade::Client as SdkClient;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::core::{CoreAgent, CoreAgentStatus};
use super::executor::{SdkTaskExecutor, TaskExecutor};
use super::suspension::{PendingQuestion, QuestionAnswerInput};
use super::worker::WorkerPool;
use crate::bus::{Bus, InboundMessage};
use crate::channel::SessionStore;
use crate::hooks::HookRegistry;
use crate::permissions::PermissionChecker;
use crate::runtime::{RuntimeProjectionView, SharedRuntimeStateStore};

pub struct Agent {
    pub bus: Arc<Bus>,
    pub opencode_client: SdkClient,
    pub session_manager: Arc<SessionStore>,
    running: AtomicBool,
    core_agent: Arc<CoreAgent>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentStatus {
    pub running: bool,
    pub core: CoreAgentStatus,
}

impl Agent {
    pub fn create(
        bus: Arc<Bus>,
        client: Option<SdkClient>,
        session_manager: Option<Arc<SessionStore>>,
        executor: Option<Arc<dyn TaskExecutor>>,
    ) -> Self {
        let opencode_client = client.unwrap_or_else(|| {
            SdkClient::with_options("http://127.0.0.1:9711", anima_sdk::ClientOptions::default())
        });
        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionStore::new()));
        let core_agent = Arc::new(CoreAgent::new(
            bus.clone(),
            opencode_client.clone(),
            Some(session_manager.clone()),
            executor.unwrap_or_else(|| Arc::new(SdkTaskExecutor)),
            None,
        ));
        Self {
            bus,
            opencode_client,
            session_manager,
            running: AtomicBool::new(false),
            core_agent,
        }
    }

    pub fn with_runtime_state_store(
        bus: Arc<Bus>,
        client: Option<SdkClient>,
        session_manager: Option<Arc<SessionStore>>,
        executor: Option<Arc<dyn TaskExecutor>>,
        runtime_state_store: SharedRuntimeStateStore,
    ) -> Self {
        let opencode_client = client.unwrap_or_else(|| {
            SdkClient::with_options("http://127.0.0.1:9711", anima_sdk::ClientOptions::default())
        });
        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionStore::new()));
        let core_agent = Arc::new(CoreAgent::new_with_runtime_state_store(
            bus.clone(),
            opencode_client.clone(),
            Some(session_manager.clone()),
            executor.unwrap_or_else(|| Arc::new(SdkTaskExecutor)),
            None,
            runtime_state_store,
        ));
        Self {
            bus,
            opencode_client,
            session_manager,
            running: AtomicBool::new(false),
            core_agent,
        }
    }

    pub fn start(&self) {
        if !self.running.swap(true, Ordering::SeqCst) {
            self.core_agent.start();
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.core_agent.stop();
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn process_message(&self, inbound_msg: InboundMessage) {
        let _ = self.bus.publish_inbound(inbound_msg);
    }

    pub fn register_builtin_tools(&mut self) {
        let core = Arc::get_mut(&mut self.core_agent)
            .expect("cannot mutate shared core_agent — ensure no other Arc references exist");
        core.register_builtin_tools();
    }

    pub fn set_hook_registry(&mut self, registry: HookRegistry) {
        let core = Arc::get_mut(&mut self.core_agent)
            .expect("cannot mutate shared core_agent — ensure no other Arc references exist");
        core.set_hook_registry(registry);
    }

    pub fn set_permission_checker(&mut self, checker: PermissionChecker) {
        let core = Arc::get_mut(&mut self.core_agent)
            .expect("cannot mutate shared core_agent — ensure no other Arc references exist");
        core.set_permission_checker(checker);
    }

    pub fn core_agent(&self) -> Arc<CoreAgent> {
        Arc::clone(&self.core_agent)
    }

    pub fn worker_pool(&self) -> Arc<WorkerPool> {
        Arc::clone(&self.core_agent.worker_pool)
    }

    pub fn status(&self) -> AgentStatus {
        AgentStatus {
            running: self.is_running(),
            core: self.core_agent.status(),
        }
    }

    pub fn pending_question_for(&self, job_id: &str) -> Option<PendingQuestion> {
        self.core_agent.pending_question_for(job_id)
    }

    pub fn submit_question_answer(
        &self,
        job_id: &str,
        answer: QuestionAnswerInput,
    ) -> Result<PendingQuestion, String> {
        self.core_agent.submit_question_answer(job_id, answer)
    }

    pub fn runtime_projection_snapshot(&self) -> RuntimeProjectionView {
        self.core_agent.runtime_projection_snapshot()
    }

    #[doc(hidden)]
    pub fn evict_resume_state_cache_for_testing(&self, job_id: &str) {
        self.core_agent.evict_resume_state_cache_for_testing(job_id);
    }
}
