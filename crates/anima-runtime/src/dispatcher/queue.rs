use crate::dispatcher::diagnostics::DispatchFailureStage;
use crate::dispatcher::message::DispatchMessage;
use crate::support::now_ms;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchEnvelope {
    pub message: DispatchMessage,
    pub priority: u8,
    pub routing_key: Option<String>,
    pub session_key: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub backoff_ms: u64,
    pub next_attempt_at_ms: u64,
    pub retryable_stages: Vec<DispatchFailureStage>,
    pub enqueue_timestamp_ms: u64,
}

impl DispatchEnvelope {
    pub fn from_message(message: DispatchMessage) -> Self {
        let now = now_ms();
        Self {
            priority: message.priority,
            routing_key: message.routing_key.clone(),
            session_key: message.session_key.clone(),
            message,
            retry_count: 0,
            max_retries: 0,
            backoff_ms: 0,
            next_attempt_at_ms: now,
            retryable_stages: vec![DispatchFailureStage::Send],
            enqueue_timestamp_ms: now,
        }
    }

    pub fn with_retry_policy(mut self, max_retries: u32, backoff_ms: u64) -> Self {
        self.max_retries = max_retries;
        self.backoff_ms = backoff_ms;
        self
    }

    pub fn with_retryable_stages(mut self, stages: Vec<DispatchFailureStage>) -> Self {
        self.retryable_stages = stages;
        self
    }

    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    pub fn should_retry_for_stage(&self, stage: Option<DispatchFailureStage>) -> bool {
        match stage {
            Some(stage) => self.retryable_stages.contains(&stage) && self.can_retry(),
            None => false,
        }
    }

    pub fn next_retry(&self) -> Option<Self> {
        if !self.can_retry() {
            return None;
        }
        let mut next = self.clone();
        next.retry_count += 1;
        next.next_attempt_at_ms = now_ms().saturating_add(next.backoff_ms);
        Some(next)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DispatchQueueSnapshot {
    pub total_len: usize,
    pub by_priority: IndexMap<u8, usize>,
    pub by_route: IndexMap<String, usize>,
}

#[derive(Debug, Default)]
struct DispatchQueueState {
    routes: IndexMap<String, IndexMap<u8, VecDeque<DispatchEnvelope>>>,
    last_route_key: Option<String>,
}

#[derive(Debug, Default)]
pub struct DispatchQueue {
    state: Mutex<DispatchQueueState>,
}

impl DispatchQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&self, envelope: DispatchEnvelope) {
        let mut state = self.state.lock().unwrap();
        state
            .routes
            .entry(envelope.message.channel.clone())
            .or_default()
            .entry(envelope.priority)
            .or_default()
            .push_back(envelope);
    }

    pub fn dequeue(&self) -> Option<DispatchEnvelope> {
        let mut state = self.state.lock().unwrap();
        let now = now_ms();

        let route_min_priorities: Vec<(String, u8)> = state
            .routes
            .iter()
            .filter_map(|(route, priorities)| {
                priorities
                    .iter()
                    .filter_map(|(priority, queue)| {
                        queue.front().and_then(|envelope| {
                            (envelope.next_attempt_at_ms <= now).then_some(*priority)
                        })
                    })
                    .min()
                    .map(|priority| (route.clone(), priority))
            })
            .collect();

        let global_priority = route_min_priorities.iter().map(|(_, priority)| *priority).min()?;
        let eligible_routes: Vec<String> = route_min_priorities
            .into_iter()
            .filter_map(|(route, priority)| (priority == global_priority).then_some(route))
            .collect();

        let chosen_route = select_next_route(&eligible_routes, state.last_route_key.as_deref())?;
        state.last_route_key = Some(chosen_route.clone());

        let route_queues = state.routes.get_mut(&chosen_route)?;
        let queue = route_queues.get_mut(&global_priority)?;
        let item = queue.pop_front();

        if queue.is_empty() {
            route_queues.shift_remove(&global_priority);
        }
        if route_queues.is_empty() {
            state.routes.shift_remove(&chosen_route);
            if state.last_route_key.as_deref() == Some(chosen_route.as_str()) {
                state.last_route_key = None;
            }
        }

        item
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .routes
            .values()
            .flat_map(|priorities| priorities.values())
            .map(VecDeque::len)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn snapshot(&self) -> DispatchQueueSnapshot {
        let state = self.state.lock().unwrap();
        let mut by_priority = IndexMap::new();
        let mut by_route = IndexMap::new();

        for (route, priorities) in &state.routes {
            let mut route_total = 0;
            for (priority, queue) in priorities {
                *by_priority.entry(*priority).or_insert(0) += queue.len();
                route_total += queue.len();
            }
            by_route.insert(route.clone(), route_total);
        }

        DispatchQueueSnapshot {
            total_len: by_route.values().sum(),
            by_priority,
            by_route,
        }
    }
}

fn select_next_route(eligible_routes: &[String], last_route_key: Option<&str>) -> Option<String> {
    if eligible_routes.is_empty() {
        return None;
    }
    let next_index = match last_route_key {
        Some(last) => eligible_routes
            .iter()
            .position(|route| route == last)
            .map(|idx| (idx + 1) % eligible_routes.len())
            .unwrap_or(0),
        None => 0,
    };
    eligible_routes.get(next_index).cloned()
}

