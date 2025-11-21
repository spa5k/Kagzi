//! Database connection and migration management

use crate::error::{Error, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;
use tracing::{debug, info};

/// Database connection manager
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connect to the database using the provided URL
    pub async fn connect(database_url: &str) -> Result<Self> {
        info!("Connecting to database...");

        let pool = PgPoolOptions::new()
            .max_connections(50)
            .acquire_timeout(Duration::from_secs(30))
            .connect(database_url)
            .await?;

        info!("Database connection established");

        Ok(Self { pool })
    }

    /// Run database migrations
    pub async fn migrate(&self) -> Result<()> {
        info!("Running database migrations...");

        // Read and execute migration files
        let migration_sql = include_str!("../../../migrations/001_initial_schema.sql");

        // Check if migrations have already been run by checking if tables exist
        let table_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT FROM information_schema.tables
                WHERE table_schema = 'public'
                AND table_name = 'workflow_runs'
            )",
        )
        .fetch_one(&self.pool)
        .await?;

        if table_exists {
            debug!("Database schema already exists, skipping migration");
            return Ok(());
        }

        // Execute the migration
        sqlx::raw_sql(migration_sql)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Migration(e.to_string()))?;

        info!("Database migrations completed successfully");
        Ok(())
    }

    /// Get a reference to the underlying connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Health check - verify database connectivity
    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}
