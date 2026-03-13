use sqlx::SqlitePool;

use orion_core::OrionError;

/// Run memory database migrations using sqlx.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), OrionError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| OrionError::Database(format!("Memory migration failed: {}", e)))?;
    Ok(())
}
