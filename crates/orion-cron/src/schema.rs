use sqlx::SqlitePool;

use orion_core::OrionError;

/// Run cron database migrations using sqlx.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), OrionError> {
    sqlx::migrate!()
        .run(pool)
        .await
        .map_err(|e| OrionError::Database(format!("Cron migration failed: {}", e)))?;
    Ok(())
}
