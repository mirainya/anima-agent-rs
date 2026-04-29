//! 会话管理模块
//!
//! 管理通道中的用户会话生命周期。每个会话绑定到一个通道和账户，
//! 携带独立的上下文（context）和消息历史（history）。
//! `SessionStore` 通过多级索引（ID、路由键、通道+账户）实现高效查找，
//! 并支持会话的创建、更新、关闭和统计。

use crate::channel::message::{extract_session_id, make_session_routing_key};
use crate::support::now_ms;
use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};
use uuid::Uuid;

/// 会话实体，表示一个用户与通道之间的交互上下文
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

/// 创建会话时的可选参数
#[derive(Debug, Default)]
pub struct SessionCreateOptions {
    pub id: Option<String>,
    pub routing_key: Option<String>,
    pub context: Option<Value>,
    pub metadata: Option<Value>,
    pub account_id: Option<String>,
}

/// 查找会话时的匹配条件，按优先级依次尝试：session_id → routing_key → account_id
#[derive(Debug, Default)]
pub struct FindSessionOptions {
    pub session_id: Option<String>,
    pub routing_key: Option<String>,
    pub account_id: Option<String>,
}

/// 会话统计信息
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub total: usize,
    pub by_channel: IndexMap<String, usize>,
    pub active_last_hour: usize,
}

/// 会话存储的内部状态，维护三个索引以支持不同维度的查找
#[derive(Debug, Default)]
struct SessionStoreState {
    /// 主索引：session_id → Session
    sessions: IndexMap<String, Session>,
    /// 路由键索引：routing_key → session_id
    by_routing_key: IndexMap<String, String>,
    /// 通道+账户索引：channel → account_id → [session_id]
    by_channel: IndexMap<String, IndexMap<String, Vec<String>>>,
}

/// 会话存储，线程安全的会话管理器
///
/// 通过 Mutex 保护内部状态，支持并发访问。
/// 提供会话的 CRUD、历史记录管理和统计查询。
#[derive(Debug, Default)]
pub struct SessionStore {
    state: Mutex<SessionStoreState>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建新会话并建立所有索引映射
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

        let mut state = self.state.lock();
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
        self.state.lock().sessions.get(session_id).cloned()
    }

    pub fn get_session_by_routing_key(&self, routing_key: &str) -> Option<Session> {
        let state = self.state.lock();
        let session_id = state.by_routing_key.get(routing_key)?.clone();
        state.sessions.get(&session_id).cloned()
    }

    pub fn get_sessions_by_channel(&self, channel: &str, account_id: Option<&str>) -> Vec<Session> {
        let state = self.state.lock();
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
        self.state.lock().sessions.values().cloned().collect()
    }

    pub fn session_exists(&self, session_id: &str) -> bool {
        self.state.lock().sessions.contains_key(session_id)
    }

    pub fn session_count(&self) -> usize {
        self.state.lock().sessions.len()
    }

    /// 增量合并更新会话上下文（浅合并 JSON 对象的顶层字段）
    pub fn update_session_context(
        &self,
        session_id: &str,
        context_update: Value,
    ) -> Option<Session> {
        let mut state = self.state.lock();
        let session = state.sessions.get_mut(session_id)?;
        merge_json_object(&mut session.context, &context_update);
        session.last_active = now_ms();
        Some(session.clone())
    }

    pub fn set_session_context(&self, session_id: &str, new_context: Value) -> Option<Session> {
        let mut state = self.state.lock();
        let session = state.sessions.get_mut(session_id)?;
        session.context = new_context;
        session.last_active = now_ms();
        Some(session.clone())
    }

    /// 向会话的 context.history 数组追加一条消息记录
    pub fn add_to_history(&self, session_id: &str, message: Value) -> Option<Session> {
        let mut state = self.state.lock();
        let session = state.sessions.get_mut(session_id)?;
        if !session.context.is_object() {
            session.context = json!({});
        }
        if let Some(obj) = session.context.as_object_mut() {
            let history = obj.entry("history").or_insert_with(|| json!([]));
            if !history.is_array() {
                *history = json!([]);
            }
            if let Some(arr) = history.as_array_mut() {
                arr.push(message);
            }
        }
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
        let mut state = self.state.lock();
        let session = state.sessions.get_mut(session_id)?;
        session.last_active = now_ms();
        Some(session.clone())
    }

    /// 关闭会话并清理所有索引（路由键索引、通道索引）
    pub fn close_session(&self, session_id: &str) -> Option<Session> {
        let mut state = self.state.lock();
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
        let mut state = self.state.lock();
        let count = state.sessions.len();
        *state = SessionStoreState::default();
        count
    }

    pub fn session_count_by_channel(&self, channel: &str, account_id: Option<&str>) -> usize {
        let state = self.state.lock();
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
        let state = self.state.lock();
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

    /// 查找或创建会话，按优先级依次尝试：session_id → routing_key → 从 routing_key 提取 session_id → 新建
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

/// 浅合并两个 JSON 对象（仅合并顶层字段，非对象类型直接覆盖）
fn merge_json_object(target: &mut Value, update: &Value) {
    if !target.is_object() || !update.is_object() {
        *target = update.clone();
        return;
    }

    if let (Some(target_obj), Some(update_obj)) = (target.as_object_mut(), update.as_object()) {
        for (key, value) in update_obj {
            target_obj.insert(key.clone(), value.clone());
        }
    }
}
