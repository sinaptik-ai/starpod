use sqlx::SqlitePool;

use starpod_core::StarpodError;

/// Run memory database migrations using sqlx.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), StarpodError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Memory migration failed: {}", e)))?;
    Ok(())
}
