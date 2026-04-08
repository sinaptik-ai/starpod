//! SQLite-backed event deduplication for Slack Socket Mode.
//!
//! Slack retries unacked events on the standard Events API schedule
//! (~1s, ~1m, ~5m). Our receive loop acks envelopes before handing them
//! off to the handler, so duplicates are rare — but not impossible:
//!
//! - Reconnect races where an ack is sent but the connection dies before
//!   Slack records it
//! - `disconnect` envelopes mid-processing that cause the whole WS to
//!   close while a handler task is still running
//! - Slack backend quirks we don't control
//!
//! A very small SQLite table keeps the handler idempotent per `event_id`.
//! A background task sweeps rows older than 24h on a 1h cadence so the
//! table stays bounded regardless of traffic.
//!
//! The table itself is created by migration
//! `003_slack_events_seen.sql` inside `starpod-db`, not by this crate.
//! That keeps all SQLite schema ownership in one place.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;
use tracing::{debug, warn};

use crate::error::{Result, SlackError};

/// Handle for the Slack dedup table. Cheap to clone (wraps an
/// `SqlitePool`).
#[derive(Clone)]
pub struct DedupStore {
    pool: SqlitePool,
}

impl DedupStore {
    /// Open a dedup store backed by an existing SQLite pool.
    ///
    /// Assumes migration `003_slack_events_seen.sql` has already been run
    /// by `starpod-db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert `event_id` if it is not already present.
    ///
    /// Returns `Ok(true)` if this is the first time we've seen the event
    /// (caller should process it) and `Ok(false)` if it is a duplicate
    /// (caller should drop silently). Uses `INSERT OR IGNORE` so two
    /// concurrent calls for the same id will deterministically pick one
    /// winner.
    pub async fn insert_if_new(&self, event_id: &str) -> Result<bool> {
        let now = now_secs();
        let result = sqlx::query(
            "INSERT OR IGNORE INTO slack_events_seen (event_id, seen_at) VALUES (?, ?)",
        )
        .bind(event_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| SlackError::Database(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete rows older than `max_age`.
    ///
    /// Returns the number of rows deleted. Safe to call concurrently —
    /// `DELETE ... WHERE seen_at < ?` is atomic in SQLite.
    pub async fn sweep_older_than(&self, max_age: Duration) -> Result<u64> {
        let cutoff = now_secs().saturating_sub(max_age.as_secs() as i64);
        let result = sqlx::query("DELETE FROM slack_events_seen WHERE seen_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(|e| SlackError::Database(e.to_string()))?;
        Ok(result.rows_affected())
    }

    /// Spawn a background sweeper task that deletes rows older than 24h
    /// every hour.
    ///
    /// Returns a `tokio::task::JoinHandle` so the caller can abort it on
    /// shutdown. The loop logs warnings on sweep failures but never
    /// returns — a transient SQLite lock shouldn't kill the sweeper.
    pub fn spawn_sweeper(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let sweep_interval = Duration::from_secs(60 * 60);
            let max_age = Duration::from_secs(24 * 60 * 60);
            // Initial small delay so startup isn't dominated by a sweep.
            tokio::time::sleep(Duration::from_secs(30)).await;
            loop {
                match self.sweep_older_than(max_age).await {
                    Ok(n) if n > 0 => debug!(rows = n, "slack dedup sweeper removed stale rows"),
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "slack dedup sweeper failed"),
                }
                tokio::time::sleep(sweep_interval).await;
            }
        })
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE slack_events_seen (
                event_id TEXT PRIMARY KEY,
                seen_at  INTEGER NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn inserts_new_event_once() {
        let store = DedupStore::new(test_pool().await);
        assert!(store.insert_if_new("Ev_1").await.unwrap());
        assert!(!store.insert_if_new("Ev_1").await.unwrap());
        assert!(store.insert_if_new("Ev_2").await.unwrap());
    }

    #[tokio::test]
    async fn sweep_removes_old_rows_only() {
        let pool = test_pool().await;
        // Insert one row with an old timestamp and one with a fresh one.
        sqlx::query("INSERT INTO slack_events_seen VALUES ('old', 0)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO slack_events_seen VALUES ('new', ?)")
            .bind(now_secs())
            .execute(&pool)
            .await
            .unwrap();

        let store = DedupStore::new(pool.clone());
        let removed = store
            .sweep_older_than(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(removed, 1);

        // The fresh row survives and the old one is gone.
        assert!(!store.insert_if_new("new").await.unwrap());
        assert!(store.insert_if_new("old").await.unwrap());
    }
}
