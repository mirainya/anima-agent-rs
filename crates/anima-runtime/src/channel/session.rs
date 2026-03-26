use crate::channel::message::{extract_session_id, make_session_routing_key};
use crate::support::now_ms;
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub channel: String,
    pub routing_key: String,
    pub context: Value,
    pub metadata: Value,
    pub created_at: u64,
    pub last_active: u64,
}

#[derive(Debug, Default)]
pub struct SessionCreateOptions {
    pub id: Option<String>,
    pub routing_key: Option<String>,
    pub context: Option<Value>,
    pub metadata: Option<Value>,
    pub account_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct FindSessionOptions {
    pub session_id: Option<String>,
    pub routing_key: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub total: usize,
    pub by_channel: IndexMap<String, usize>,
    pub active_last_hour: usize,
}

#[derive(Debug, Default)]
struct SessionStoreState {
    sessions: IndexMap<String, Session>,
    by_routing_key: IndexMap<String, String>,
    by_channel: IndexMap<String, IndexMap<String, Vec<String>>>,
}

#[derive(Debug, Default)]
pub struct SessionStore {
    state: Mutex<SessionStoreState>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_session(&self, channel: &str, opts: SessionCreateOptions) -> Session {
        let session_id = opts.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let routing_key = opts
            .routing_key
            .unwrap_or_else(|| make_session_routing_key(&session_id));
        let account_id = opts.account_id.unwrap_or_else(|| "default".into());
        let now = now_ms();
        let session = Session {
            id: session_id.clone(),
            channel: channel.to_string(),
            routing_key: routing_key.clone(),
            context: opts.context.unwrap_or_else(|| json!({})),
            metadata: opts.metadata.unwrap_or_else(|| json!({})),
            created_at: now,
            last_active: now,
        };

        let mut state = self.state.lock().unwrap();
        state.sessions.insert(session_id.clone(), session.clone());
        state.by_routing_key.insert(routing_key, session_id.clone());
        state
            .by_channel
            .entry(channel.to_string())
            .or_default()
            .entry(account_id)
            .or_default()
            .push(session_id);
        session
    }

    pub fn get_session(&self, session_id: &str) -> Option<Session> {
        self.state.lock().unwrap().sessions.get(session_id).cloned()
    }

    pub fn get_session_by_routing_key(&self, routing_key: &str) -> Option<Session> {
        let state = self.state.lock().unwrap();
        let session_id = state.by_routing_key.get(routing_key)?.clone();
        state.sessions.get(&session_id).cloned()
    }

    pub fn get_sessions_by_channel(&self, channel: &str, account_id: Option<&str>) -> Vec<Session> {
        let state = self.state.lock().unwrap();
        let Some(accounts) = state.by_channel.get(channel) else {
            return Vec::new();
        };
        let account = account_id.unwrap_or("default");
        accounts
            .get(account)
            .into_iter()
            .flatten()
            .filter_map(|id| state.sessions.get(id).cloned())
            .collect()
    }

    pub fn get_all_sessions(&self) -> Vec<Session> {
        self.state
            .lock()
            .unwrap()
            .sessions
            .values()
            .cloned()
            .collect()
    }

    pub fn session_exists(&self, session_id: &str) -> bool {
        self.state.lock().unwrap().sessions.contains_key(session_id)
    }

    pub fn session_count(&self) -> usize {
        self.state.lock().unwrap().sessions.len()
    }

    pub fn update_session_context(
        &self,
        session_id: &str,
        context_update: Value,
    ) -> Option<Session> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.get_mut(session_id)?;
        merge_json_object(&mut session.context, &context_update);
        session.last_active = now_ms();
        Some(session.clone())
    }

    pub fn set_session_context(&self, session_id: &str, new_context: Value) -> Option<Session> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.get_mut(session_id)?;
        session.context = new_context;
        session.last_active = now_ms();
        Some(session.clone())
    }

    pub fn add_to_history(&self, session_id: &str, message: Value) -> Option<Session> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.get_mut(session_id)?;
        if !session.context.is_object() {
            session.context = json!({});
        }
        let obj = session.context.as_object_mut().unwrap();
        let history = obj.entry("history").or_insert_with(|| json!([]));
        if !history.is_array() {
            *history = json!([]);
        }
        history.as_array_mut().unwrap().push(message);
        session.last_active = now_ms();
        Some(session.clone())
    }

    pub fn get_history(&self, session_id: &str) -> Vec<Value> {
        self.get_session(session_id)
            .and_then(|session| session.context.get("history").cloned())
            .and_then(|history| history.as_array().cloned())
            .unwrap_or_default()
    }

    pub fn touch_session(&self, session_id: &str) -> Option<Session> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.get_mut(session_id)?;
        session.last_active = now_ms();
        Some(session.clone())
    }

    pub fn close_session(&self, session_id: &str) -> Option<Session> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.shift_remove(session_id)?;
        state.by_routing_key.shift_remove(&session.routing_key);
        if let Some(accounts) = state.by_channel.get_mut(&session.channel) {
            let empty_accounts: Vec<String> = accounts
                .iter_mut()
                .filter_map(|(account_id, ids)| {
                    ids.retain(|id| id != session_id);
                    if ids.is_empty() {
                        Some(account_id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            for account_id in empty_accounts {
                accounts.shift_remove(&account_id);
            }
        }
        Some(session)
    }

    pub fn close_all_sessions(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        let count = state.sessions.len();
        *state = SessionStoreState::default();
        count
    }

    pub fn session_count_by_channel(&self, channel: &str, account_id: Option<&str>) -> usize {
        let state = self.state.lock().unwrap();
        let Some(accounts) = state.by_channel.get(channel) else {
            return 0;
        };
        if let Some(account_id) = account_id {
            accounts.get(account_id).map(|ids| ids.len()).unwrap_or(0)
        } else {
            accounts.values().map(Vec::len).sum()
        }
    }

    pub fn get_stats(&self) -> SessionStats {
        let state = self.state.lock().unwrap();
        let one_hour_ago = now_ms().saturating_sub(3_600_000);
        let mut by_channel = IndexMap::new();
        for session in state.sessions.values() {
            *by_channel.entry(session.channel.clone()).or_insert(0) += 1;
        }
        let active_last_hour = state
            .sessions
            .values()
            .filter(|session| session.last_active > one_hour_ago)
            .count();
        SessionStats {
            total: state.sessions.len(),
            by_channel,
            active_last_hour,
        }
    }

    pub fn find_or_create_session(&self, channel_name: &str, opts: FindSessionOptions) -> Session {
        if let Some(session_id) = opts.session_id.as_deref() {
            if let Some(session) = self.get_session(session_id) {
                return session;
            }
        }

        if let Some(routing_key) = opts.routing_key.as_deref() {
            if let Some(session) = self.get_session_by_routing_key(routing_key) {
                return session;
            }
        }

        if let Some(routing_key) = opts.routing_key.as_deref() {
            if let Some(session_id) = extract_session_id(routing_key) {
                if let Some(session) = self.get_session(&session_id) {
                    return session;
                }
            }
        }

        self.create_session(
            channel_name,
            SessionCreateOptions {
                routing_key: opts.routing_key,
                account_id: opts.account_id,
                ..Default::default()
            },
        )
    }
}

fn merge_json_object(target: &mut Value, update: &Value) {
    if !target.is_object() || !update.is_object() {
        *target = update.clone();
        return;
    }

    let target_obj = target.as_object_mut().unwrap();
    let update_obj = update.as_object().unwrap();
    for (key, value) in update_obj {
        target_obj.insert(key.clone(), value.clone());
    }
}
