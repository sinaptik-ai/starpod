//! In-memory sliding-window rate limiter.
//!
//! Each user's request timestamps are stored in a `Vec<Instant>` behind a
//! `Mutex`. On every `check()`, expired entries are pruned before comparing
//! against the limit. This keeps memory proportional to active users × window size.
//!
//! ## Thread safety
//!
//! The `Mutex<HashMap>` is safe to share across Tokio tasks via `Arc<RateLimiter>`.
//! Lock contention is minimal because the critical section only does Vec filtering.
//!
//! ## Limitations
//!
//! - State is in-memory only — restarting the process resets all counters.
//! - No per-user cleanup of idle entries; they stay until the next `check()`.
//! - Setting `max_requests = 0` disables rate limiting entirely.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// In-memory sliding-window per-user rate limiter.
///
/// # Example
///
/// ```
/// use std::time::Duration;
/// use starpod_auth::RateLimiter;
///
/// let limiter = RateLimiter::new(10, Duration::from_secs(60));
/// assert!(limiter.check("user-1")); // first request — allowed
/// ```
pub struct RateLimiter {
    max_requests: u32,
    window: Duration,
    state: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// - `max_requests`: maximum allowed requests per `window`. Use `0` to disable.
    /// - `window`: sliding time window for counting requests.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Check if a request from `user_id` is allowed.
    ///
    /// Returns `true` and records the timestamp if the request is within limits.
    /// Returns `false` if the user has exceeded `max_requests` in the current window.
    ///
    /// Expired entries are cleaned up on every call, so there is no need for a
    /// separate background cleanup task.
    pub fn check(&self, user_id: &str) -> bool {
        if self.max_requests == 0 {
            return true; // disabled
        }

        let now = Instant::now();
        let cutoff = now - self.window;

        let mut state = self.state.lock().unwrap();
        let timestamps = state.entry(user_id.to_string()).or_default();

        // Remove expired entries
        timestamps.retain(|&t| t > cutoff);

        if timestamps.len() >= self.max_requests as usize {
            return false;
        }

        timestamps.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_limit() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
    }

    #[test]
    fn blocks_over_limit() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1"));
    }

    #[test]
    fn exact_boundary_allowed_then_blocked() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("user1"), "exactly at limit should pass");
        assert!(!limiter.check("user1"), "one over limit should fail");
    }

    #[test]
    fn separate_users_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user2"));
        assert!(!limiter.check("user1"));
        assert!(!limiter.check("user2"));
    }

    #[test]
    fn zero_max_requests_disables() {
        let limiter = RateLimiter::new(0, Duration::from_secs(60));
        for _ in 0..100 {
            assert!(limiter.check("user1"));
        }
    }

    #[test]
    fn expired_entries_are_cleaned() {
        let limiter = RateLimiter::new(1, Duration::from_millis(1));
        assert!(limiter.check("user1"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(limiter.check("user1")); // old entry expired
    }

    #[test]
    fn many_users_stay_independent() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        for i in 0..50 {
            let uid = format!("user{}", i);
            assert!(limiter.check(&uid));
            assert!(limiter.check(&uid));
            assert!(!limiter.check(&uid));
        }
    }

    #[test]
    fn blocked_user_unblocked_after_expiry() {
        let limiter = RateLimiter::new(1, Duration::from_millis(10));
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1"));
        std::thread::sleep(Duration::from_millis(20));
        assert!(limiter.check("user1"), "should be allowed after window expires");
    }
}
