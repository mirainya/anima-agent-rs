//! 熔断器模块
//!
//! 实现经典的三态熔断器模式（Closed → Open → HalfOpen → Closed），
//! 用于保护下游服务免受级联故障影响。
//!
//! 状态转换规则：
//! - Closed（正常）：连续失败达到阈值 → Open
//! - Open（熔断）：超时后 → HalfOpen
//! - HalfOpen（试探）：连续成功达到阈值 → Closed，任意失败 → Open

use crate::support::now_ms;
use parking_lot::Mutex;
use uuid::Uuid;

// ── Circuit Breaker States ──────────────────────────────────────────

/// 熔断器状态
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    /// 正常放行请求
    Closed,
    /// 熔断中，拒绝所有请求
    Open,
    /// 半开状态，允许有限请求通过以试探恢复
    HalfOpen,
}

impl CircuitState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half-open",
        }
    }
}

// ── Configuration ───────────────────────────────────────────────────

/// 熔断器配置参数
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// 触发熔断的连续失败次数
    pub failure_threshold: u64,
    /// 从 HalfOpen 恢复到 Closed 所需的连续成功次数
    pub success_threshold: u64,
    /// Open 状态持续时间（毫秒），超时后进入 HalfOpen
    pub timeout_ms: u64,
    /// HalfOpen 状态下允许通过的最大请求数
    pub half_open_max_calls: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            timeout_ms: 60_000,
            half_open_max_calls: 5,
        }
    }
}

// ── Stats ───────────────────────────────────────────────────────────

/// 熔断器运行统计
#[derive(Debug, Clone, Default)]
pub struct CircuitBreakerStats {
    pub total_failures: u64,
    pub total_successes: u64,
    pub total_rejections: u64,
    pub last_failure_ms: Option<u64>,
    pub last_success_ms: Option<u64>,
    pub tripped_at_ms: Option<u64>,
}

// ── Result type ─────────────────────────────────────────────────────

/// 熔断器执行结果
#[derive(Debug)]
pub enum CircuitBreakerResult<T> {
    /// 执行成功
    Ok(T),
    /// 请求被熔断器拒绝（附带建议的重试等待时间）
    Rejected { retry_after_ms: Option<u64> },
    /// 执行失败
    Failed(String),
}

// ── Circuit Breaker ─────────────────────────────────────────────────

/// 熔断器核心实现
///
/// 线程安全（内部使用 parking_lot::Mutex），可在多线程环境下共享使用。
pub struct CircuitBreaker {
    pub id: String,
    state: Mutex<CircuitState>,
    failure_count: Mutex<u64>,
    success_count: Mutex<u64>,
    config: CircuitBreakerConfig,
    stats: Mutex<CircuitBreakerStats>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            state: Mutex::new(CircuitState::Closed),
            failure_count: Mutex::new(0),
            success_count: Mutex::new(0),
            config,
            stats: Mutex::new(CircuitBreakerStats::default()),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    // ── State queries ───────────────────────────────────────────

    pub fn state(&self) -> CircuitState {
        self.state.lock().clone()
    }

    pub fn is_closed(&self) -> bool {
        *self.state.lock() == CircuitState::Closed
    }

    pub fn is_open(&self) -> bool {
        *self.state.lock() == CircuitState::Open
    }

    pub fn is_half_open(&self) -> bool {
        *self.state.lock() == CircuitState::HalfOpen
    }

    /// 判断当前是否允许请求通过
    /// HalfOpen 状态下受 half_open_max_calls 限制
    pub fn allows_request(&self) -> bool {
        let state = self.state.lock();
        match *state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => *self.success_count.lock() < self.config.half_open_max_calls,
            CircuitState::Open => false,
        }
    }

    // ── State transitions ───────────────────────────────────────

    /// 校验状态转换是否合法，防止非法跳转
    fn valid_transition(from: &CircuitState, to: &CircuitState) -> bool {
        matches!(
            (from, to),
            (CircuitState::Closed, CircuitState::Open)
                | (CircuitState::Open, CircuitState::HalfOpen)
                | (CircuitState::HalfOpen, CircuitState::Closed)
                | (CircuitState::HalfOpen, CircuitState::Open)
        )
    }

    fn transition_to(&self, new_state: CircuitState) -> bool {
        let mut state = self.state.lock();
        if Self::valid_transition(&state, &new_state) {
            *state = new_state;
            true
        } else {
            false
        }
    }

    /// 检查 Open 状态是否已超时，超时则转入 HalfOpen 进行试探
    fn check_timeout(&self) {
        let state = self.state.lock().clone();
        if state != CircuitState::Open {
            return;
        }
        let stats = self.stats.lock();
        if let Some(tripped_at) = stats.tripped_at_ms {
            if now_ms().saturating_sub(tripped_at) > self.config.timeout_ms {
                drop(stats);
                if self.transition_to(CircuitState::HalfOpen) {
                    *self.success_count.lock() = 0;
                }
            }
        }
    }

    /// 记录一次失败
    /// Closed 状态下累计失败达阈值则熔断；HalfOpen 状态下任何失败立即重新熔断
    fn record_failure(&self, _error: &str) {
        let state = self.state.lock().clone();
        {
            let mut fc = self.failure_count.lock();
            *fc += 1;
        }
        {
            let mut stats = self.stats.lock();
            stats.total_failures += 1;
            stats.last_failure_ms = Some(now_ms());
        }

        match state {
            CircuitState::Closed => {
                let fc = *self.failure_count.lock();
                if fc >= self.config.failure_threshold && self.transition_to(CircuitState::Open) {
                    self.stats.lock().tripped_at_ms = Some(now_ms());
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open immediately re-opens
                self.transition_to(CircuitState::Open);
                *self.success_count.lock() = 0;
                self.stats.lock().tripped_at_ms = Some(now_ms());
            }
            CircuitState::Open => {}
        }
    }

    /// 记录一次成功
    /// Closed 状态下重置失败计数；HalfOpen 状态下累计成功达阈值则恢复
    fn record_success(&self) {
        let state = self.state.lock().clone();
        {
            let mut stats = self.stats.lock();
            stats.total_successes += 1;
            stats.last_success_ms = Some(now_ms());
        }

        match state {
            CircuitState::Closed => {
                // Reset failure count on success
                *self.failure_count.lock() = 0;
            }
            CircuitState::HalfOpen => {
                let mut sc = self.success_count.lock();
                *sc += 1;
                if *sc >= self.config.success_threshold {
                    drop(sc);
                    if self.transition_to(CircuitState::Closed) {
                        *self.failure_count.lock() = 0;
                        *self.success_count.lock() = 0;
                    }
                }
            }
            CircuitState::Open => {}
        }
    }

    // ── Main API ────────────────────────────────────────────────

    /// 在熔断器保护下执行闭包
    /// 自动处理超时检查、请求放行判断、成功/失败记录
    /// Execute a function with circuit breaker protection.
    pub fn execute<T, F>(&self, f: F) -> CircuitBreakerResult<T>
    where
        F: FnOnce() -> Result<T, String>,
    {
        self.check_timeout();

        if !self.allows_request() {
            self.stats.lock().total_rejections += 1;
            let retry_after = self.stats.lock().tripped_at_ms.map(|t| {
                self.config
                    .timeout_ms
                    .saturating_sub(now_ms().saturating_sub(t))
            });
            return CircuitBreakerResult::Rejected {
                retry_after_ms: retry_after,
            };
        }

        match f() {
            Ok(value) => {
                self.record_success();
                CircuitBreakerResult::Ok(value)
            }
            Err(e) => {
                self.record_failure(&e);
                CircuitBreakerResult::Failed(e)
            }
        }
    }

    /// Manually trip (open) the circuit breaker.
    pub fn trip(&self) {
        if self.is_closed() {
            self.transition_to(CircuitState::Open);
            self.stats.lock().tripped_at_ms = Some(now_ms());
        }
    }

    /// Manually reset the circuit breaker to closed.
    pub fn reset(&self) {
        *self.state.lock() = CircuitState::Closed;
        *self.failure_count.lock() = 0;
        *self.success_count.lock() = 0;
    }

    pub fn stats(&self) -> CircuitBreakerStats {
        self.stats.lock().clone()
    }

    pub fn status(&self) -> CircuitBreakerStatus {
        CircuitBreakerStatus {
            id: self.id.clone(),
            state: self.state(),
            failure_count: *self.failure_count.lock(),
            success_count: *self.success_count.lock(),
            stats: self.stats(),
            config: self.config.clone(),
        }
    }
}

/// 熔断器状态快照，用于外部监控和诊断
#[derive(Debug, Clone)]
pub struct CircuitBreakerStatus {
    pub id: String,
    pub state: CircuitState,
    pub failure_count: u64,
    pub success_count: u64,
    pub stats: CircuitBreakerStats,
    pub config: CircuitBreakerConfig,
}
