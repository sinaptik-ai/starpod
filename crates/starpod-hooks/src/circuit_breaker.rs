//! Circuit breaker for hooks — auto-disables after consecutive failures.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Configuration for the circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before the breaker opens.
    pub max_consecutive_failures: u32,
    /// How long the breaker stays open before allowing retries.
    pub cooldown: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            max_consecutive_failures: 5,
            cooldown: Duration::from_secs(60),
        }
    }
}

/// Current status of a circuit breaker for a named hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakerStatus {
    /// Hook is healthy, executing normally.
    Closed,
    /// Hook is disabled due to repeated failures.
    Open {
        /// When the cooldown expires and the hook can retry.
        until: Instant,
        /// Number of consecutive failures that triggered the open state.
        failures: u32,
    },
}

/// Internal state for a single hook's breaker.
#[derive(Debug, Default)]
struct BreakerState {
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

/// Circuit breaker that tracks consecutive failures per named hook.
///
/// When a hook fails `max_consecutive_failures` times in a row, it is
/// "tripped" (opened) and skipped for the `cooldown` duration. After
/// cooldown, the next execution attempt is allowed through; a success
/// resets the breaker, a failure re-opens it.
///
/// # Example
///
/// ```
/// use starpod_hooks::CircuitBreaker;
/// use starpod_hooks::CircuitBreakerConfig;
/// use std::time::Duration;
///
/// let cb = CircuitBreaker::new(CircuitBreakerConfig {
///     max_consecutive_failures: 3,
///     cooldown: Duration::from_secs(60),
/// });
///
/// // Hook starts healthy
/// assert!(!cb.is_tripped("my-hook"));
///
/// // Record failures
/// cb.record_failure("my-hook");
/// cb.record_failure("my-hook");
/// cb.record_failure("my-hook");
///
/// // Now tripped
/// assert!(cb.is_tripped("my-hook"));
/// ```
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    states: Mutex<HashMap<String, BreakerState>>,
}

impl CircuitBreaker {
    /// Create a circuit breaker with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            states: Mutex::new(HashMap::new()),
        }
    }

    /// Check whether the breaker is currently tripped (open) for the given hook name.
    ///
    /// Returns `true` if the hook should be skipped.
    pub fn is_tripped(&self, name: &str) -> bool {
        let mut states = self.states.lock().unwrap();
        let state = match states.get_mut(name) {
            Some(s) => s,
            None => return false,
        };

        if let Some(opened_at) = state.opened_at {
            if opened_at.elapsed() >= self.config.cooldown {
                // Cooldown expired — allow one attempt (half-open).
                // We clear opened_at but keep the failure count so that
                // if it fails again it re-opens immediately.
                state.opened_at = None;
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Record a successful execution for the named hook, resetting its breaker.
    pub fn record_success(&self, name: &str) {
        let mut states = self.states.lock().unwrap();
        if let Some(state) = states.get_mut(name) {
            state.consecutive_failures = 0;
            state.opened_at = None;
        }
    }

    /// Record a failed execution for the named hook.
    ///
    /// If consecutive failures reach the threshold, the breaker opens.
    pub fn record_failure(&self, name: &str) {
        let mut states = self.states.lock().unwrap();
        let state = states.entry(name.to_string()).or_default();
        state.consecutive_failures += 1;

        if state.consecutive_failures >= self.config.max_consecutive_failures {
            state.opened_at = Some(Instant::now());
        }
    }

    /// Get the current status of the breaker for a named hook.
    pub fn status(&self, name: &str) -> BreakerStatus {
        let states = self.states.lock().unwrap();
        match states.get(name) {
            None => BreakerStatus::Closed,
            Some(state) => match state.opened_at {
                Some(opened_at) => {
                    let remaining = self.config.cooldown.saturating_sub(opened_at.elapsed());
                    if remaining.is_zero() {
                        BreakerStatus::Closed
                    } else {
                        BreakerStatus::Open {
                            until: opened_at + self.config.cooldown,
                            failures: state.consecutive_failures,
                        }
                    }
                }
                None => BreakerStatus::Closed,
            },
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.max_consecutive_failures, 5);
        assert_eq!(config.cooldown, Duration::from_secs(60));
    }

    #[test]
    fn not_tripped_initially() {
        let cb = CircuitBreaker::default();
        assert!(!cb.is_tripped("my-hook"));
        assert_eq!(cb.status("my-hook"), BreakerStatus::Closed);
    }

    #[test]
    fn opens_after_max_failures() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            max_consecutive_failures: 3,
            cooldown: Duration::from_secs(60),
        });

        cb.record_failure("h");
        assert!(!cb.is_tripped("h"));
        cb.record_failure("h");
        assert!(!cb.is_tripped("h"));
        cb.record_failure("h");
        // Now at 3 failures — should be tripped
        assert!(cb.is_tripped("h"));
        assert!(matches!(cb.status("h"), BreakerStatus::Open { failures: 3, .. }));
    }

    #[test]
    fn success_resets_breaker() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            max_consecutive_failures: 2,
            cooldown: Duration::from_secs(60),
        });

        cb.record_failure("h");
        cb.record_success("h");
        cb.record_failure("h");
        // Only 1 failure since last success — not tripped
        assert!(!cb.is_tripped("h"));
        assert_eq!(cb.status("h"), BreakerStatus::Closed);
    }

    #[test]
    fn cooldown_expires() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            max_consecutive_failures: 1,
            cooldown: Duration::from_millis(0), // instant cooldown
        });

        cb.record_failure("h");
        // Cooldown is 0ms so should already have expired
        assert!(!cb.is_tripped("h"));
    }

    #[test]
    fn unnamed_hooks_bypass() {
        let cb = CircuitBreaker::default();
        // A name that was never registered is never tripped
        assert!(!cb.is_tripped("never-seen"));
        assert_eq!(cb.status("never-seen"), BreakerStatus::Closed);
    }

    #[test]
    fn independent_hooks_tracked_separately() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            max_consecutive_failures: 2,
            cooldown: Duration::from_secs(60),
        });

        cb.record_failure("a");
        cb.record_failure("a");
        assert!(cb.is_tripped("a"));
        assert!(!cb.is_tripped("b"));
    }

    #[test]
    fn re_opens_on_failure_after_half_open() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            max_consecutive_failures: 2,
            cooldown: Duration::from_secs(60),
        });

        cb.record_failure("h");
        cb.record_failure("h");
        // Breaker is now open with 60s cooldown
        assert!(cb.is_tripped("h"));
        // A success resets it
        cb.record_success("h");
        assert!(!cb.is_tripped("h"));
        // Fail again — needs 2 consecutive failures to re-open
        cb.record_failure("h");
        assert!(!cb.is_tripped("h"));
        cb.record_failure("h");
        assert!(cb.is_tripped("h"));
    }
}
