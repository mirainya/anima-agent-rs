use crate::runtime::events::RuntimeDomainEvent;
use crate::runtime::reducer::reduce_event;
use crate::runtime::snapshot::{PersistedRuntimeState, RuntimeStateSnapshot};
use crate::support::now_ms;
use crate::tasks::model::{
    RequirementRecord, RunRecord, SuspensionRecord, TaskRecord, ToolInvocationRuntimeRecord,
    TurnRecord,
};
use crate::transcript::model::MessageRecord;
use anima_store::SqliteStore;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RUNTIME_STATE_PERSISTENCE_VERSION: u32 = 1;

fn purge_session_from_snapshot(
    snapshot: &mut RuntimeStateSnapshot,
    chat_id: &str,
) -> (Vec<String>, Vec<String>) {
    let run_ids: Vec<String> = snapshot
        .runs
        .iter()
        .filter(|(_, r)| r.chat_id.as_deref() == Some(chat_id))
        .map(|(id, _)| id.clone())
        .collect();

    let mut turn_ids = Vec::new();
    for run_id in &run_ids {
        snapshot.runs.remove(run_id);
        snapshot.index.run_ids_by_job_id.retain(|_, v| v != run_id);

        turn_ids.extend(
            snapshot
                .turns
                .iter()
                .filter(|(_, t)| &t.run_id == run_id)
                .map(|(id, _)| id.clone()),
        );
    }

    let mut task_ids = Vec::new();
    let mut suspension_ids = Vec::new();
    let mut tool_ids = Vec::new();
    let mut requirement_ids = Vec::new();

    for turn_id in &turn_ids {
        snapshot.turns.remove(turn_id);
    }

    for (id, task) in &snapshot.tasks {
        if run_ids.contains(&task.run_id) {
            task_ids.push(id.clone());
        }
    }
    for id in &task_ids {
        snapshot.tasks.remove(id);
    }

    for (id, s) in &snapshot.suspensions {
        if run_ids.contains(&s.run_id) {
            suspension_ids.push(id.clone());
        }
    }
    for id in &suspension_ids {
        snapshot.suspensions.remove(id);
    }

    for (id, t) in &snapshot.tool_invocations {
        if run_ids.contains(&t.run_id) {
            tool_ids.push(id.clone());
        }
    }
    for id in &tool_ids {
        snapshot.tool_invocations.remove(id);
    }

    for (id, r) in &snapshot.requirements {
        if run_ids.contains(&r.run_id) {
            requirement_ids.push(id.clone());
        }
    }
    for id in &requirement_ids {
        snapshot.requirements.remove(id);
    }

    snapshot.transcript.retain(|m| !run_ids.contains(&m.run_id));
    snapshot.session_titles.remove(chat_id);

    (run_ids, turn_ids)
}

pub trait StateStore: Send + Sync {
    fn append(&self, event: RuntimeDomainEvent) -> u64;
    fn snapshot(&self) -> RuntimeStateSnapshot;
    fn next_sequence(&self) -> u64;

    fn delete_session(&self, chat_id: &str) -> usize;
    fn set_session_title(&self, chat_id: &str, title: String);
    fn delete_session_title(&self, chat_id: &str);
}

// ---------------------------------------------------------------------------
// JSON file backend
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct JsonStoreInner {
    next_sequence: u64,
    snapshot: RuntimeStateSnapshot,
}

#[derive(Debug)]
pub struct JsonStateStore {
    inner: Mutex<JsonStoreInner>,
    persistence_path: Option<PathBuf>,
}

impl Default for JsonStateStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(JsonStoreInner::default()),
            persistence_path: None,
        }
    }
}

impl JsonStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_persistence(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let restored = Self::load_persisted_state(&path).unwrap_or_else(|| PersistedRuntimeState {
            version: RUNTIME_STATE_PERSISTENCE_VERSION,
            next_sequence: 0,
            snapshot: RuntimeStateSnapshot::default(),
        });
        Self {
            inner: Mutex::new(JsonStoreInner {
                next_sequence: restored.next_sequence,
                snapshot: restored.snapshot,
            }),
            persistence_path: Some(path),
        }
    }

    fn load_persisted_state(path: &Path) -> Option<PersistedRuntimeState> {
        let content = std::fs::read_to_string(path).ok()?;
        let persisted: PersistedRuntimeState = serde_json::from_str(&content).ok()?;
        if persisted.version != RUNTIME_STATE_PERSISTENCE_VERSION {
            return None;
        }
        Some(persisted)
    }

    fn persist_state(path: &Path, persisted: &PersistedRuntimeState) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(persisted)?;
        std::fs::write(&tmp_path, content)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

impl StateStore for JsonStateStore {
    fn append(&self, event: RuntimeDomainEvent) -> u64 {
        let mut inner = self.inner.lock();
        inner.next_sequence += 1;
        let sequence = inner.next_sequence;
        reduce_event(&mut inner.snapshot, event, sequence, now_ms());
        let persisted = PersistedRuntimeState {
            version: RUNTIME_STATE_PERSISTENCE_VERSION,
            next_sequence: inner.next_sequence,
            snapshot: inner.snapshot.clone(),
        };
        if let Some(path) = &self.persistence_path {
            let _ = Self::persist_state(path, &persisted);
        }
        sequence
    }

    fn snapshot(&self) -> RuntimeStateSnapshot {
        self.inner.lock().snapshot.clone()
    }

    fn next_sequence(&self) -> u64 {
        self.inner.lock().next_sequence
    }

    fn delete_session(&self, chat_id: &str) -> usize {
        let mut inner = self.inner.lock();
        let (run_ids, _) = purge_session_from_snapshot(&mut inner.snapshot, chat_id);
        let count = run_ids.len();
        if count > 0 {
            if let Some(path) = &self.persistence_path {
                let persisted = PersistedRuntimeState {
                    version: RUNTIME_STATE_PERSISTENCE_VERSION,
                    next_sequence: inner.next_sequence,
                    snapshot: inner.snapshot.clone(),
                };
                let _ = Self::persist_state(path, &persisted);
            }
        }
        count
    }

    fn set_session_title(&self, chat_id: &str, title: String) {
        let mut inner = self.inner.lock();
        inner
            .snapshot
            .session_titles
            .insert(chat_id.to_string(), title);
        if let Some(path) = &self.persistence_path {
            let persisted = PersistedRuntimeState {
                version: RUNTIME_STATE_PERSISTENCE_VERSION,
                next_sequence: inner.next_sequence,
                snapshot: inner.snapshot.clone(),
            };
            let _ = Self::persist_state(path, &persisted);
        }
    }

    fn delete_session_title(&self, chat_id: &str) {
        let mut inner = self.inner.lock();
        if inner.snapshot.session_titles.remove(chat_id).is_some() {
            if let Some(path) = &self.persistence_path {
                let persisted = PersistedRuntimeState {
                    version: RUNTIME_STATE_PERSISTENCE_VERSION,
                    next_sequence: inner.next_sequence,
                    snapshot: inner.snapshot.clone(),
                };
                let _ = Self::persist_state(path, &persisted);
            }
        }
    }
}

const TABLES: &[&str] = &[
    "runs",
    "turns",
    "tasks",
    "suspensions",
    "tool_invocations",
    "requirements",
    "transcript",
    "events",
    "meta",
];

pub struct SqliteStateStore {
    store: SqliteStore,
    inner: Mutex<SqliteStoreInner>,
}

#[derive(Default)]
struct SqliteStoreInner {
    next_sequence: u64,
    snapshot: RuntimeStateSnapshot,
}

impl SqliteStateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let store = SqliteStore::open(path).map_err(|e| e.to_string())?;
        Self::init(store)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let store = SqliteStore::open_in_memory().map_err(|e| e.to_string())?;
        Self::init(store)
    }

    fn init(store: SqliteStore) -> Result<Self, String> {
        for table in TABLES {
            store.ensure_table(table).map_err(|e| e.to_string())?;
        }
        let next_sequence = store
            .get("meta", "next_sequence")
            .map_err(|e| e.to_string())?
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let snapshot = Self::load_snapshot(&store)?;

        Ok(Self {
            store,
            inner: Mutex::new(SqliteStoreInner {
                next_sequence,
                snapshot,
            }),
        })
    }

    fn load_snapshot(store: &SqliteStore) -> Result<RuntimeStateSnapshot, String> {
        let mut snap = RuntimeStateSnapshot::default();

        for (id, data) in store.get_all("runs").map_err(|e| e.to_string())? {
            if let Ok(r) = serde_json::from_str::<RunRecord>(&data) {
                snap.index
                    .run_ids_by_job_id
                    .insert(r.job_id.clone(), id.clone());
                snap.runs.insert(id, r);
            }
        }
        for (id, data) in store.get_all("turns").map_err(|e| e.to_string())? {
            if let Ok(r) = serde_json::from_str::<TurnRecord>(&data) {
                snap.turns.insert(id, r);
            }
        }
        for (id, data) in store.get_all("tasks").map_err(|e| e.to_string())? {
            if let Ok(r) = serde_json::from_str::<TaskRecord>(&data) {
                if let Some(plan_id) = r.plan_id.clone() {
                    snap.index
                        .task_ids_by_plan_id
                        .entry(plan_id)
                        .or_default()
                        .push(id.clone());
                }
                snap.tasks.insert(id, r);
            }
        }
        for (id, data) in store.get_all("suspensions").map_err(|e| e.to_string())? {
            if let Ok(r) = serde_json::from_str::<SuspensionRecord>(&data) {
                if let Some(qid) = r.question_id.clone() {
                    snap.index
                        .suspension_ids_by_question_id
                        .insert(qid.clone(), id.clone());
                    if let Some(inv_id) = r.invocation_id.clone() {
                        snap.index.invocation_ids_by_question_id.insert(qid, inv_id);
                    }
                }
                snap.suspensions.insert(id, r);
            }
        }
        for (id, data) in store
            .get_all("tool_invocations")
            .map_err(|e| e.to_string())?
        {
            if let Ok(r) = serde_json::from_str::<ToolInvocationRuntimeRecord>(&data) {
                snap.tool_invocations.insert(id, r);
            }
        }
        for (id, data) in store.get_all("requirements").map_err(|e| e.to_string())? {
            if let Ok(r) = serde_json::from_str::<RequirementRecord>(&data) {
                snap.requirements.insert(id, r);
            }
        }
        for (_id, data) in store
            .get_all_ordered("transcript")
            .map_err(|e| e.to_string())?
        {
            if let Ok(r) = serde_json::from_str::<MessageRecord>(&data) {
                snap.transcript.push(r);
            }
        }

        for (id, data) in store.get_all("meta").map_err(|e| e.to_string())? {
            if let Some(chat_id) = id.strip_prefix("session_title:") {
                snap.session_titles.insert(chat_id.to_string(), data);
            }
        }

        Ok(snap)
    }

    fn persist_event(&self, event: &RuntimeDomainEvent, sequence: u64) {
        let event_type = match event {
            RuntimeDomainEvent::RunUpserted { run } => {
                let _ = self.store.upsert(
                    "runs",
                    &run.run_id,
                    &serde_json::to_string(run).unwrap_or_default(),
                );
                "run_upserted"
            }
            RuntimeDomainEvent::TurnUpserted { turn } => {
                let _ = self.store.upsert(
                    "turns",
                    &turn.turn_id,
                    &serde_json::to_string(turn).unwrap_or_default(),
                );
                "turn_upserted"
            }
            RuntimeDomainEvent::TaskUpserted { task } => {
                let _ = self.store.upsert(
                    "tasks",
                    &task.task_id,
                    &serde_json::to_string(task).unwrap_or_default(),
                );
                "task_upserted"
            }
            RuntimeDomainEvent::SuspensionUpserted { suspension } => {
                let _ = self.store.upsert(
                    "suspensions",
                    &suspension.suspension_id,
                    &serde_json::to_string(suspension).unwrap_or_default(),
                );
                "suspension_upserted"
            }
            RuntimeDomainEvent::ToolInvocationUpserted { invocation } => {
                let _ = self.store.upsert(
                    "tool_invocations",
                    &invocation.invocation_id,
                    &serde_json::to_string(invocation).unwrap_or_default(),
                );
                "tool_invocation_upserted"
            }
            RuntimeDomainEvent::RequirementUpserted { requirement } => {
                let _ = self.store.upsert(
                    "requirements",
                    &requirement.requirement_id,
                    &serde_json::to_string(requirement).unwrap_or_default(),
                );
                "requirement_upserted"
            }
            RuntimeDomainEvent::MessageAppended { message } => {
                let _ = self.store.append(
                    "transcript",
                    &message.message_id,
                    &serde_json::to_string(message).unwrap_or_default(),
                );
                "message_appended"
            }
            RuntimeDomainEvent::ProjectionHintRecorded {
                run_id,
                scope,
                key,
                value,
            } => {
                let hint_key = format!("{run_id}.{scope}.{key}");
                let _ = self
                    .store
                    .upsert("meta", &format!("hint:{hint_key}"), &value.to_string());
                "projection_hint_recorded"
            }
        };
        let _ = self
            .store
            .upsert("meta", "next_sequence", &sequence.to_string());
        let _ = self
            .store
            .append("events", &sequence.to_string(), event_type);
    }
}

impl StateStore for SqliteStateStore {
    fn append(&self, event: RuntimeDomainEvent) -> u64 {
        let mut inner = self.inner.lock();
        inner.next_sequence += 1;
        let sequence = inner.next_sequence;
        self.persist_event(&event, sequence);
        reduce_event(&mut inner.snapshot, event, sequence, now_ms());
        sequence
    }

    fn snapshot(&self) -> RuntimeStateSnapshot {
        self.inner.lock().snapshot.clone()
    }

    fn next_sequence(&self) -> u64 {
        self.inner.lock().next_sequence
    }

    fn delete_session(&self, chat_id: &str) -> usize {
        let mut inner = self.inner.lock();
        let (run_ids, turn_ids) = purge_session_from_snapshot(&mut inner.snapshot, chat_id);
        let count = run_ids.len();
        for run_id in &run_ids {
            let _ = self.store.delete("runs", run_id);
        }
        for id in &turn_ids {
            let _ = self.store.delete("turns", id);
        }
        // tasks/suspensions/tool_invocations/requirements already removed from snapshot
        // by purge_session_from_snapshot; delete from sqlite by scanning store
        for table in &["tasks", "suspensions", "tool_invocations", "requirements"] {
            if let Ok(rows) = self.store.get_all(table) {
                for (id, data) in rows {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                        if let Some(rid) = val.get("run_id").and_then(|v| v.as_str()) {
                            if run_ids.contains(&rid.to_string()) {
                                let _ = self.store.delete(table, &id);
                            }
                        }
                    }
                }
            }
        }
        if let Ok(rows) = self.store.get_all_ordered("transcript") {
            for (id, data) in rows {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(rid) = val.get("run_id").and_then(|v| v.as_str()) {
                        if run_ids.contains(&rid.to_string()) {
                            let _ = self.store.delete("transcript", &id);
                        }
                    }
                }
            }
        }
        let _ = self
            .store
            .delete("meta", &format!("session_title:{chat_id}"));
        count
    }

    fn set_session_title(&self, chat_id: &str, title: String) {
        let mut inner = self.inner.lock();
        inner
            .snapshot
            .session_titles
            .insert(chat_id.to_string(), title.clone());
        let _ = self
            .store
            .upsert("meta", &format!("session_title:{chat_id}"), &title);
    }

    fn delete_session_title(&self, chat_id: &str) {
        let mut inner = self.inner.lock();
        inner.snapshot.session_titles.remove(chat_id);
        let _ = self
            .store
            .delete("meta", &format!("session_title:{chat_id}"));
    }
}

pub type RuntimeStateStore = JsonStateStore;
pub type SharedRuntimeStateStore = Arc<dyn StateStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::model::{RunRecord, RunStatus};

    fn make_run(run_id: &str, job_id: &str) -> RunRecord {
        RunRecord {
            run_id: run_id.into(),
            trace_id: "t".into(),
            job_id: job_id.into(),
            chat_id: None,
            channel: "test".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: 100,
            updated_at_ms: 100,
            completed_at_ms: None,
        }
    }

    #[test]
    fn json_store_append_and_snapshot() {
        let store = JsonStateStore::new();
        let seq = store.append(RuntimeDomainEvent::RunUpserted {
            run: make_run("r1", "j1"),
        });
        assert_eq!(seq, 1);
        let snap = store.snapshot();
        assert!(snap.runs.contains_key("r1"));
    }

    #[test]
    fn sqlite_store_append_and_snapshot() {
        let store = SqliteStateStore::open_in_memory().unwrap();
        let seq = store.append(RuntimeDomainEvent::RunUpserted {
            run: make_run("r1", "j1"),
        });
        assert_eq!(seq, 1);
        let snap = store.snapshot();
        assert!(snap.runs.contains_key("r1"));
        assert_eq!(
            snap.index.run_ids_by_job_id.get("j1"),
            Some(&"r1".to_string())
        );
    }

    #[test]
    fn sqlite_store_persists_across_reload() {
        let dir = std::env::temp_dir().join("anima_test_sqlite_reload");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");

        {
            let store = SqliteStateStore::open(&db_path).unwrap();
            store.append(RuntimeDomainEvent::RunUpserted {
                run: make_run("r1", "j1"),
            });
            assert_eq!(store.next_sequence(), 1);
        }

        {
            let store = SqliteStateStore::open(&db_path).unwrap();
            assert_eq!(store.next_sequence(), 1);
            let snap = store.snapshot();
            assert!(snap.runs.contains_key("r1"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
