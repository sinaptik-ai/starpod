use rusqlite::Connection;

use orion_core::{Migration, OrionError};

/// Cron migrations.
pub fn migrations() -> &'static [Migration] {
    &[Migration {
        version: 1,
        name: "create_cron_tables",
        sql: "
            CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                prompt TEXT NOT NULL,
                schedule_type TEXT NOT NULL,
                schedule_value TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                delete_after_run INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_run_at TEXT,
                next_run_at TEXT
            );

            CREATE TABLE IF NOT EXISTS cron_runs (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                result_summary TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs(job_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_cron_jobs_next ON cron_jobs(next_run_at);
        ",
    }]
}

/// Run cron database migrations.
pub fn migrate(conn: &Connection) -> Result<(), OrionError> {
    orion_core::run_migrations(conn, "cron", migrations())
}
