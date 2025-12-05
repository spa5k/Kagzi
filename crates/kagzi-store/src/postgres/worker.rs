use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ListWorkersParams, RegisterWorkerParams, Worker, WorkerHeartbeatParams, WorkerStatus,
};
use crate::repository::WorkerRepository;

#[derive(sqlx::FromRow)]
struct WorkerRow {
    worker_id: Uuid,
    namespace_id: String,
    task_queue: String,
    status: String,
    hostname: Option<String>,
    pid: Option<i32>,
    version: Option<String>,
    workflow_types: Vec<String>,
    max_concurrent: i32,
    active_count: i32,
    total_completed: i64,
    total_failed: i64,
    registered_at: chrono::DateTime<chrono::Utc>,
    last_heartbeat_at: chrono::DateTime<chrono::Utc>,
    deregistered_at: Option<chrono::DateTime<chrono::Utc>>,
    labels: serde_json::Value,
}

impl WorkerRow {
    fn into_model(self) -> Worker {
        Worker {
            worker_id: self.worker_id,
            namespace_id: self.namespace_id,
            task_queue: self.task_queue,
            status: WorkerStatus::from_db_str(&self.status),
            hostname: self.hostname,
            pid: self.pid,
            version: self.version,
            workflow_types: self.workflow_types,
            max_concurrent: self.max_concurrent,
            active_count: self.active_count,
            total_completed: self.total_completed,
            total_failed: self.total_failed,
            registered_at: self.registered_at,
            last_heartbeat_at: self.last_heartbeat_at,
            deregistered_at: self.deregistered_at,
            labels: self.labels,
        }
    }
}

#[derive(Clone)]
pub struct PgWorkerRepository {
    pool: PgPool,
}

impl PgWorkerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkerRepository for PgWorkerRepository {
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError> {
        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workers (
                namespace_id, task_queue, hostname, pid, version,
                workflow_types, max_concurrent, labels
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING worker_id
            "#,
            params.namespace_id,
            params.task_queue,
            params.hostname,
            params.pid,
            params.version,
            &params.workflow_types,
            params.max_concurrent,
            params.labels
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.worker_id)
    }

    async fn heartbeat(&self, params: WorkerHeartbeatParams) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET last_heartbeat_at = NOW(),
                active_count = $2,
                total_completed = total_completed + GREATEST($3, 0),
                total_failed = total_failed + GREATEST($4, 0)
            WHERE worker_id = $1
              AND status != 'OFFLINE'
            RETURNING worker_id
            "#,
            params.worker_id,
            params.active_count,
            params.completed_delta,
            params.failed_delta
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    async fn start_drain(&self, worker_id: Uuid) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET status = 'DRAINING'
            WHERE worker_id = $1
            "#,
            worker_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn deregister(&self, worker_id: Uuid) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET status = 'OFFLINE',
                active_count = 0,
                deregistered_at = NOW()
            WHERE worker_id = $1
            "#,
            worker_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn find_by_id(&self, worker_id: Uuid) -> Result<Option<Worker>, StoreError> {
        let row = sqlx::query_as::<_, WorkerRow>(
            r#"
            SELECT worker_id, namespace_id, task_queue, status, hostname, pid, version,
                   workflow_types, max_concurrent, active_count, total_completed, total_failed,
                   registered_at, last_heartbeat_at, deregistered_at, labels
            FROM kagzi.workers
            WHERE worker_id = $1
            "#,
        )
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()))
    }

    async fn validate_online(&self, worker_id: Uuid) -> Result<bool, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT 1 AS one FROM kagzi.workers
            WHERE worker_id = $1 AND status = 'ONLINE'
            "#,
            worker_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.is_some())
    }

    async fn list(&self, params: ListWorkersParams) -> Result<Vec<Worker>, StoreError> {
        let limit = (params.page_size.max(1) + 1) as i64; // fetch one extra to compute next_page_token

        let rows = sqlx::query_as::<_, WorkerRow>(
            r#"
            SELECT worker_id, namespace_id, task_queue, status, hostname, pid, version,
                   workflow_types, max_concurrent, active_count, total_completed, total_failed,
                   registered_at, last_heartbeat_at, deregistered_at, labels
            FROM kagzi.workers
            WHERE namespace_id = $1
              AND ($2::TEXT IS NULL OR task_queue = $2)
              AND ($3::TEXT IS NULL OR status = $3)
              AND ($4::UUID IS NULL OR worker_id < $4)
            ORDER BY worker_id DESC
            LIMIT $5
            "#,
        )
        .bind(&params.namespace_id)
        .bind(params.task_queue.as_deref())
        .bind(params.filter_status.map(|s| s.as_db_str()))
        .bind(params.cursor)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_model()).collect())
    }

    async fn mark_stale_offline(&self, threshold_secs: i64) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET status = 'OFFLINE',
                deregistered_at = NOW()
            WHERE status != 'OFFLINE'
              AND last_heartbeat_at < NOW() - ($1 * INTERVAL '1 second')
            "#,
            threshold_secs as f64
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn find_stale_worker_ids(&self, threshold_secs: i64) -> Result<Vec<Uuid>, StoreError> {
        let rows = sqlx::query!(
            r#"
            SELECT worker_id
            FROM kagzi.workers
            WHERE status != 'OFFLINE'
              AND last_heartbeat_at < NOW() - ($1 * INTERVAL '1 second')
            "#,
            threshold_secs as f64
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.worker_id).collect())
    }

    async fn count_online(&self, namespace_id: &str, task_queue: &str) -> Result<i64, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM kagzi.workers
            WHERE namespace_id = $1
              AND task_queue = $2
              AND status = 'ONLINE'
            "#,
            namespace_id,
            task_queue
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.count.unwrap_or(0))
    }

    async fn update_active_count(&self, worker_id: Uuid, delta: i32) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET active_count = GREATEST(active_count + $2, 0)
            WHERE worker_id = $1
            "#,
            worker_id,
            delta
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn count(
        &self,
        namespace_id: &str,
        task_queue: Option<&str>,
        filter_status: Option<WorkerStatus>,
    ) -> Result<i64, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM kagzi.workers
            WHERE namespace_id = $1
              AND ($2::TEXT IS NULL OR task_queue = $2)
              AND ($3::TEXT IS NULL OR status = $3)
            "#,
            namespace_id,
            task_queue,
            filter_status.map(|s| s.as_db_str())
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.count.unwrap_or(0))
    }
}
