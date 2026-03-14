use sqlx::SqlitePool;

use starpod_core::StarpodError;

/// Run vault database migrations using sqlx.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), StarpodError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Vault migration failed: {}", e)))?;
    Ok(())
}
