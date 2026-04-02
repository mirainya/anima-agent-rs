use crate::classifier::rule::{AgentClassifier, ClassificationDecision, ClassificationKind};
use crate::orchestrator::specialist_pool::SpecialistPool;
use crate::agent::types::{make_task, MakeTask, TaskResult};
use crate::bus::InboundMessage;
use crate::support::now_ms;
use crate::classifier::task::{classify_task, ClassificationResult};
use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

// ── Dialog Context ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DialogContext {
    pub session_id: String,
    pub channel: String,
    pub sender_id: String,
    pub message_history: Vec<Value>,
    pub task_history: Vec<Value>,
    pub created_at_ms: u64,
    pub last_active_ms: u64,
    pub metadata: Value,
}

impl DialogContext {
    pub fn new(session_id: &str, channel: &str, sender_id: &str) -> Self {
        let now = now_ms();
        Self {
            session_id: session_id.to_string(),
            channel: channel.to_string(),
            sender_id: sender_id.to_string(),
            message_history: Vec::new(),
            task_history: Vec::new(),
            created_at_ms: now,
            last_active_ms: now,
            metadata: json!({}),
        }
    }

    pub fn record_message(&mut self, message: Value, max_history: usize) {
        self.message_history.push(message);
        if self.message_history.len() > max_history {
            let excess = self.message_history.len() - max_history;
            self.message_history.drain(..excess);
        }
        self.last_active_ms = now_ms();
    }

    /// Record message with classification, trimming to max_history.
    pub fn record_message_with_classification(
        &mut self,
        message: &str,
        classification: &ClassificationResult,
        max_history: usize,
    ) {
        self.message_history.push(json!({
            "content": message,
            "classification": classification.task_type.as_str(),
            "confidence": classification.confidence,
            "timestamp_ms": now_ms(),
        }));
        if self.message_history.len() > max_history {
            let excess = self.message_history.len() - max_history;
            self.message_history.drain(..excess);
        }
        self.last_active_ms = now_ms();
    }

    pub fn record_task(&mut self, task_result: Value, max_history: usize) {
        self.task_history.push(task_result);
        if self.task_history.len() > max_history {
            let excess = self.task_history.len() - max_history;
            self.task_history.drain(..excess);
        }
        self.last_active_ms = now_ms();
    }
}

// ── Routed Dialog ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RoutedDialog {
    pub id: String,
    pub trace_id: String,
    pub session_id: String,
    pub classification: ClassificationKind,
    pub specialist: Option<String>,
    pub status: RoutedDialogStatus,
    pub result: Option<TaskResult>,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoutedDialogStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

// ── Router Config ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct IntelligentRouterConfig {
    pub max_context_age_ms: u64,
    pub max_message_history: usize,
    pub max_task_history: usize,
    pub default_timeout_ms: u64,
    pub classification_threshold: f64,
    pub fallback_to_general: bool,
}

impl Default for IntelligentRouterConfig {
    fn default() -> Self {
        Self {
            max_context_age_ms: 3_600_000,
            max_message_history: 100,
            max_task_history: 100,
            default_timeout_ms: 60_000,
            classification_threshold: 0.3,
            fallback_to_general: true,
        }
    }
}

// ── Router Metrics ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct RouterMetrics {
    pub total_routed: u64,
    pub total_completed: u64,
    pub total_errors: u64,
    pub total_timeouts: u64,
    pub avg_latency_ms: f64,
}

// ── Process Result ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub status: ProcessStatus,
    pub classification: Option<ClassificationResult>,
    pub specialist_result: Option<TaskResult>,
    pub error: Option<String>,
    pub processing_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Success,
    Error,
    Timeout,
}

// ── Formatted Response ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FormattedResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
    pub task_type: Option<String>,
    pub confidence: Option<f64>,
    pub processing_time_ms: Option<u64>,
}

// ── Default Specialist Definitions ──────────────────────────────────

pub struct SpecialistDef {
    pub id: &'static str,
    pub name: &'static str,
    pub capabilities: &'static [&'static str],
}

pub const DEFAULT_SPECIALISTS: &[SpecialistDef] = &[
    SpecialistDef {
        id: "code-specialist",
        name: "Code Specialist",
        capabilities: &[
            "code-generation", "code-review", "code-debug",
            "code-refactor", "test-generation", "api-call",
        ],
    },
    SpecialistDef {
        id: "search-specialist",
        name: "Search Specialist",
        capabilities: &["web-search"],
    },
    SpecialistDef {
        id: "doc-specialist",
        name: "Documentation Specialist",
        capabilities: &["documentation"],
    },
    SpecialistDef {
        id: "analysis-specialist",
        name: "Data Analysis Specialist",
        capabilities: &["data-analysis"],
    },
    SpecialistDef {
        id: "chat-specialist",
        name: "Chat Specialist",
        capabilities: &["general-chat"],
    },
];

// ── Intelligent Router ──────────────────────────────────────────────

pub struct IntelligentRouter {
    id: String,
    specialist_pool: Arc<SpecialistPool>,
    contexts: Mutex<IndexMap<String, DialogContext>>,
    config: IntelligentRouterConfig,
    metrics: Mutex<RouterMetrics>,
    running: AtomicBool,
}

impl IntelligentRouter {
    pub fn new(
        specialist_pool: Arc<SpecialistPool>,
        config: IntelligentRouterConfig,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            specialist_pool,
            contexts: Mutex::new(IndexMap::new()),
            config,
            metrics: Mutex::new(RouterMetrics::default()),
            running: AtomicBool::new(false),
        }
    }

    // ── Lifecycle ───────────────────────────────────────────────

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    // ── Context Management ──────────────────────────────────────

    pub fn cleanup_old_contexts(&self) -> usize {
        let max_age = self.config.max_context_age_ms;
        let now = now_ms();
        let mut contexts = self.contexts.lock();
        let before = contexts.len();
        contexts.retain(|_, ctx| now.saturating_sub(ctx.last_active_ms) < max_age);
        before - contexts.len()
    }

    pub fn get_context(&self, session_id: &str) -> Option<DialogContext> {
        self.contexts.lock().get(session_id).cloned()
    }

    pub fn context_count(&self) -> usize {
        self.contexts.lock().len()
    }

    // ── Classification ──────────────────────────────────────────

    fn classify_message(&self, content: &str) -> ClassificationResult {
        classify_task(content)
    }

    // ── Routing ─────────────────────────────────────────────────

    fn route_to_specialist(
        &self,
        classification: &ClassificationResult,
        content: &str,
        session_id: &str,
    ) -> Result<TaskResult, String> {
        let confidence = classification.confidence;
        let threshold = self.config.classification_threshold;

        if confidence < threshold && !self.config.fallback_to_general {
            return Err(format!(
                "Classification confidence {:.2} below threshold {:.2}",
                confidence, threshold
            ));
        }

        let specialist_id = classification.task_type.specialist();
        let task_type = classification.task_type.as_str();
        let trace_id = Uuid::new_v4().to_string();

        let task = make_task(MakeTask {
            task_type: task_type.into(),
            payload: Some(json!({
                "content": content,
                "specialist": specialist_id,
                "session_id": session_id,
                "confidence": confidence,
            })),
            trace_id: Some(trace_id),
            ..Default::default()
        });

        Ok(self.specialist_pool.execute(specialist_id, task))
    }

    // ── Full Pipeline ───────────────────────────────────────────

    pub fn process_message(
        &self,
        content: &str,
        session_id: &str,
        channel: &str,
        sender: &str,
    ) -> ProcessResult {
        let start = now_ms();

        // Step 1: Classify
        let classification = self.classify_message(content);

        // Step 2: Update context
        {
            let mut contexts = self.contexts.lock();
            let ctx = contexts
                .entry(session_id.to_string())
                .or_insert_with(|| DialogContext::new(session_id, channel, sender));
            ctx.record_message_with_classification(
                content,
                &classification,
                self.config.max_message_history,
            );
        }

        // Step 3: Route to specialist
        match self.route_to_specialist(&classification, content, session_id) {
            Ok(result) => {
                // Record task in context
                {
                    let mut contexts = self.contexts.lock();
                    if let Some(ctx) = contexts.get_mut(session_id) {
                        ctx.record_task(json!({
                            "classification": classification.task_type.as_str(),
                            "status": &result.status,
                            "timestamp_ms": now_ms(),
                        }), self.config.max_task_history);
                    }
                }

                let mut metrics = self.metrics.lock();
                metrics.total_routed += 1;
                metrics.total_completed += 1;

                ProcessResult {
                    status: ProcessStatus::Success,
                    classification: Some(classification),
                    specialist_result: Some(result),
                    error: None,
                    processing_time_ms: now_ms().saturating_sub(start),
                }
            }
            Err(e) => {
                self.metrics.lock().total_errors += 1;
                ProcessResult {
                    status: ProcessStatus::Error,
                    classification: Some(classification),
                    specialist_result: None,
                    error: Some(e),
                    processing_time_ms: now_ms().saturating_sub(start),
                }
            }
        }
    }

    // ── Batch Processing ────────────────────────────────────────

    pub fn process_batch(
        &self,
        messages: &[(&str, &str, &str, &str)], // (content, session_id, channel, sender)
    ) -> Vec<ProcessResult> {
        messages
            .iter()
            .map(|(content, session_id, channel, sender)| {
                self.process_message(content, session_id, channel, sender)
            })
            .collect()
    }

    // ── Route (legacy API, kept for backward compat) ────────────

    pub fn route(&self, message: &InboundMessage) -> RoutedDialog {
        let decision = AgentClassifier::classify(message);
        let specialist = extract_specialist(message);
        let kind = decision.kind.clone();
        let specialist_name = specialist.clone();
        let task_type = task_type_for(&decision);
        let trace_id = Uuid::new_v4().to_string();

        let session_id = message
            .metadata
            .get("session-id")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let channel = &message.channel;
        let sender = message
            .metadata
            .get("sender")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        // Update context
        {
            let mut contexts = self.contexts.lock();
            let ctx = contexts
                .entry(session_id.to_string())
                .or_insert_with(|| DialogContext::new(session_id, channel, sender));
            ctx.record_message(json!({
                "content": message.content,
                "channel": message.channel,
            }), self.config.max_message_history);
        }

        let task = make_task(MakeTask {
            task_type,
            payload: Some(json!({
                "content": message.content,
                "channel": message.channel,
                "specialist": specialist_name,
            })),
            trace_id: Some(trace_id.clone()),
            ..Default::default()
        });

        let target = specialist.as_deref().unwrap_or("default");
        let result = self.specialist_pool.execute(target, task);
        let status = RoutedDialogStatus::Completed;

        {
            let mut m = self.metrics.lock();
            m.total_routed += 1;
            m.total_completed += 1;
        }

        RoutedDialog {
            id: Uuid::new_v4().to_string(),
            trace_id,
            session_id: session_id.to_string(),
            classification: kind,
            specialist: specialist_name,
            status,
            result: Some(result),
            started_at_ms: now_ms(),
            completed_at_ms: Some(now_ms()),
        }
    }

    // ── Status & Metrics ────────────────────────────────────────

    pub fn metrics(&self) -> RouterMetrics {
        self.metrics.lock().clone()
    }

    pub fn status(&self) -> Value {
        let m = self.metrics.lock().clone();
        json!({
            "id": self.id,
            "running": self.is_running(),
            "context_count": self.context_count(),
            "metrics": {
                "total_routed": m.total_routed,
                "total_completed": m.total_completed,
                "total_errors": m.total_errors,
            }
        })
    }
}

// ── Response Formatting ─────────────────────────────────────────────

pub fn format_response(result: &ProcessResult) -> FormattedResponse {
    match result.status {
        ProcessStatus::Error => FormattedResponse {
            success: false,
            message: format!("处理失败: {}", result.error.as_deref().unwrap_or("unknown")),
            error: result.error.clone(),
            task_type: result.classification.as_ref().map(|c| c.task_type.as_str().to_string()),
            confidence: result.classification.as_ref().map(|c| c.confidence),
            processing_time_ms: Some(result.processing_time_ms),
        },
        ProcessStatus::Timeout => FormattedResponse {
            success: false,
            message: "处理超时，请稍后重试".to_string(),
            error: Some("timeout".to_string()),
            task_type: result.classification.as_ref().map(|c| c.task_type.as_str().to_string()),
            confidence: result.classification.as_ref().map(|c| c.confidence),
            processing_time_ms: Some(result.processing_time_ms),
        },
        ProcessStatus::Success => FormattedResponse {
            success: true,
            message: "处理完成".to_string(),
            error: None,
            task_type: result.classification.as_ref().map(|c| c.task_type.as_str().to_string()),
            confidence: result.classification.as_ref().map(|c| c.confidence),
            processing_time_ms: Some(result.processing_time_ms),
        },
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn extract_specialist(message: &InboundMessage) -> Option<String> {
    message
        .metadata
        .get("specialist")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn task_type_for(decision: &ClassificationDecision) -> String {
    match decision.kind {
        ClassificationKind::Direct => "direct".into(),
        ClassificationKind::Single => "api-call".into(),
        ClassificationKind::Sequential => "sequential".into(),
        ClassificationKind::Parallel => "parallel".into(),
        ClassificationKind::SpecialistRoute => "specialist".into(),
    }
}
