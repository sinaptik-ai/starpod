use sqlx::SqlitePool;

use starpod_core::StarpodError;

/// Run cron database migrations using sqlx.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), StarpodError> {
    sqlx::migrate!()
        .run(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Cron migration failed: {}", e)))?;
    Ok(())
}
