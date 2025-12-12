use async_trait::async_trait;
use sqlx::{PgPool, QueryBuilder};
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ListWorkersParams, PaginatedResult, RegisterWorkerParams, Worker, WorkerCursor,
    WorkerHeartbeatParams, WorkerStatus, WorkflowTypeConcurrency,
};
use crate::repository::WorkerRepository;

const WORKER_COLUMNS: &str = "\
    worker_id, namespace_id, task_queue, status, hostname, pid, version, \
    workflow_types, max_concurrent, active_count, total_completed, total_failed, \
    registered_at, last_heartbeat_at, deregistered_at, labels";

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
    fn into_model(
        self,
        queue_concurrency_limit: Option<i32>,
        workflow_type_concurrency: Vec<WorkflowTypeConcurrency>,
    ) -> Result<Worker, StoreError> {
        let status = self.status.parse().map_err(|_| {
            StoreError::invalid_state(format!("invalid worker status: {}", self.status))
        })?;

        Ok(Worker {
            worker_id: self.worker_id,
            namespace_id: self.namespace_id,
            task_queue: self.task_queue,
            status,
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
            queue_concurrency_limit,
            workflow_type_concurrency,
        })
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
    #[instrument(skip(self, params))]
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError> {
        let worker_id: Uuid = sqlx::query_scalar!(
            r#"
            INSERT INTO kagzi.workers (
                namespace_id, task_queue, hostname, pid, version,
                workflow_types, max_concurrent, labels, status, active_count,
                last_heartbeat_at, deregistered_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'ONLINE', 0, NOW(), NULL)
            ON CONFLICT (namespace_id, task_queue, hostname, pid) WHERE status != 'OFFLINE'
            DO UPDATE SET
                version = EXCLUDED.version,
                workflow_types = EXCLUDED.workflow_types,
                max_concurrent = EXCLUDED.max_concurrent,
                labels = EXCLUDED.labels,
                status = 'ONLINE',
                active_count = 0,
                deregistered_at = NULL,
                last_heartbeat_at = NOW()
            RETURNING worker_id
            "#,
            &params.namespace_id,
            &params.task_queue,
            params.hostname.as_deref(),
            params.pid,
            params.version.as_deref(),
            &params.workflow_types,
            params.max_concurrent,
            &params.labels
        )
        .fetch_one(&self.pool)
        .await?;

        // Note: queue_concurrency_limit and workflow_type_concurrency are now
        // handled by worker-side semaphores, not server-side config tables

        Ok(worker_id)
    }

    #[instrument(skip(self, params))]
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

    #[instrument(skip(self))]
    async fn start_drain(&self, worker_id: Uuid, namespace_id: &str) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET status = 'DRAINING'
            WHERE worker_id = $1 AND namespace_id = $2
            "#,
            worker_id,
            namespace_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn deregister(&self, worker_id: Uuid, namespace_id: &str) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET status = 'OFFLINE',
                active_count = 0,
                deregistered_at = NOW()
            WHERE worker_id = $1 AND namespace_id = $2
            "#,
            worker_id,
            namespace_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn find_by_id(&self, worker_id: Uuid) -> Result<Option<Worker>, StoreError> {
        let row = sqlx::query_as!(
            WorkerRow,
            r#"
            SELECT 
                worker_id,
                namespace_id,
                task_queue,
                status,
                hostname,
                pid,
                version,
                workflow_types,
                max_concurrent,
                active_count,
                total_completed,
                total_failed,
                registered_at,
                last_heartbeat_at,
                deregistered_at,
                labels
            FROM kagzi.workers
            WHERE worker_id = $1
            "#,
            worker_id
        )
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = row {
            // Concurrency limits are now managed by worker-side semaphores
            Ok(Some(r.into_model(None, Vec::new())?))
        } else {
            Ok(None)
        }
    }

    #[instrument(skip(self, params))]
    async fn list(
        &self,
        params: ListWorkersParams,
    ) -> Result<PaginatedResult<Worker, WorkerCursor>, StoreError> {
        let limit = (params.page_size.max(1) + 1) as i64;

        let mut builder = QueryBuilder::new("SELECT ");
        builder.push(WORKER_COLUMNS);
        builder.push(" FROM kagzi.workers WHERE namespace_id = ");
        builder.push_bind(&params.namespace_id);
        if let Some(task_queue) = &params.task_queue {
            builder.push(" AND task_queue = ").push_bind(task_queue);
        }
        if let Some(status) = params.filter_status {
            let status_str = status.as_ref().to_string();
            builder.push(" AND status = ").push_bind(status_str);
        }
        if let Some(ref cursor) = params.cursor {
            builder
                .push(" AND worker_id < ")
                .push_bind(cursor.worker_id);
        }
        builder.push(" ORDER BY worker_id DESC LIMIT ");
        builder.push_bind(limit);

        let rows: Vec<WorkerRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        if rows.is_empty() {
            return Ok(PaginatedResult::empty());
        }

        let has_more = rows.len() > params.page_size as usize;
        let mut items = Vec::with_capacity(rows.len().min(params.page_size as usize));
        for r in rows.into_iter().take(params.page_size as usize) {
            // Concurrency limits are now managed by worker-side semaphores
            items.push(r.into_model(None, Vec::new())?);
        }

        let next_cursor = if has_more {
            items.last().map(|w| WorkerCursor {
                worker_id: w.worker_id,
            })
        } else {
            None
        };

        Ok(PaginatedResult {
            items,
            next_cursor,
            has_more,
        })
    }

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
    async fn update_active_count(
        &self,
        worker_id: Uuid,
        namespace_id: &str,
        delta: i32,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workers
            SET active_count = GREATEST(active_count + $2, 0)
            WHERE worker_id = $1 AND namespace_id = $3
            "#,
            worker_id,
            delta,
            namespace_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn count(
        &self,
        namespace_id: &str,
        task_queue: Option<&str>,
        filter_status: Option<WorkerStatus>,
    ) -> Result<i64, StoreError> {
        let status_filter = filter_status.map(|status| status.as_ref().to_owned());

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
            status_filter
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.count.unwrap_or(0))
    }
}
