use crate::support::now_ms;
use parking_lot::Mutex;
use uuid::Uuid;

// ── Circuit Breaker States ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,
    Open,
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

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u64,
    pub success_threshold: u64,
    pub timeout_ms: u64,
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

#[derive(Debug)]
pub enum CircuitBreakerResult<T> {
    Ok(T),
    Rejected { retry_after_ms: Option<u64> },
    Failed(String),
}

// ── Circuit Breaker ─────────────────────────────────────────────────

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

    pub fn allows_request(&self) -> bool {
        let state = self.state.lock();
        match *state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => {
                *self.success_count.lock() < self.config.half_open_max_calls
            }
            CircuitState::Open => false,
        }
    }

    // ── State transitions ───────────────────────────────────────

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
                if fc >= self.config.failure_threshold
                    && self.transition_to(CircuitState::Open) {
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

    /// Execute a function with circuit breaker protection.
    pub fn execute<T, F>(&self, f: F) -> CircuitBreakerResult<T>
    where
        F: FnOnce() -> Result<T, String>,
    {
        self.check_timeout();

        if !self.allows_request() {
            self.stats.lock().total_rejections += 1;
            let retry_after = self.stats.lock().tripped_at_ms.map(|t| {
                self.config.timeout_ms.saturating_sub(now_ms().saturating_sub(t))
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

#[derive(Debug, Clone)]
pub struct CircuitBreakerStatus {
    pub id: String,
    pub state: CircuitState,
    pub failure_count: u64,
    pub success_count: u64,
    pub stats: CircuitBreakerStats,
    pub config: CircuitBreakerConfig,
}

