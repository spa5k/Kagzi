use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use sqlx::{PgPool, Postgres, QueryBuilder};
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ListWorkersParams, PaginatedResult, RegisterWorkerParams, Worker, WorkerCursor,
    WorkerHeartbeatParams, WorkerStatus, WorkflowTypeConcurrency,
};
use crate::postgres::columns;
use crate::postgres::query::{FilterBuilder, push_limit};
use crate::repository::WorkerRepository;

#[derive(sqlx::FromRow)]
struct WorkerRow {
    worker_id: Uuid,
    namespace_id: String,
    task_queue: String,
    status: WorkerStatus,
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
    ) -> Worker {
        Worker {
            worker_id: self.worker_id,
            namespace_id: self.namespace_id,
            task_queue: self.task_queue,
            status: self.status,
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
    #[instrument(skip(self, params))]
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workers (
                namespace_id, task_queue, hostname, pid, version,
                workflow_types, max_concurrent, labels
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING worker_id
            "#,
            &params.namespace_id,
            &params.task_queue,
            params.hostname,
            params.pid,
            params.version,
            &params.workflow_types,
            params.max_concurrent,
            params.labels
        )
        .fetch_one(&mut *tx)
        .await?;

        // Persist queue-level concurrency if provided.
        if let Some(limit) = params.queue_concurrency_limit {
            sqlx::query!(
                r#"
                INSERT INTO kagzi.queue_configs (namespace_id, task_queue, max_concurrent)
                VALUES ($1, $2, $3)
                ON CONFLICT (namespace_id, task_queue)
                DO UPDATE SET max_concurrent = EXCLUDED.max_concurrent,
                              updated_at = NOW()
                "#,
                &params.namespace_id,
                &params.task_queue,
                limit
            )
            .execute(&mut *tx)
            .await?;
        }

        // Persist workflow-type concurrency limits.
        for entry in params.workflow_type_concurrency {
            sqlx::query!(
                r#"
                INSERT INTO kagzi.workflow_type_configs (namespace_id, task_queue, workflow_type, max_concurrent)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (namespace_id, task_queue, workflow_type)
                DO UPDATE SET max_concurrent = EXCLUDED.max_concurrent,
                              updated_at = NOW()
                "#,
                &params.namespace_id,
                &params.task_queue,
                entry.workflow_type,
                entry.max_concurrent
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(row.worker_id)
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
        let query = format!(
            "SELECT {} FROM kagzi.workers WHERE worker_id = $1",
            columns::worker::BASE
        );
        let row = sqlx::query_as::<_, WorkerRow>(&query)
            .bind(worker_id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(r) = row {
            let (queue_limit, type_limits) = self
                .fetch_concurrency(&r.namespace_id, &r.task_queue)
                .await?;
            Ok(Some(r.into_model(queue_limit, type_limits)))
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

        let status_filter = params
            .filter_status
            .map(|status| status.as_ref().to_owned());

        let mut filters = FilterBuilder::select(columns::worker::BASE, "kagzi.workers");
        filters.and_eq("namespace_id", &params.namespace_id);
        filters.and_optional_eq("task_queue", params.task_queue.as_deref());
        filters.and_optional_eq("status", status_filter);
        if let Some(ref cursor) = params.cursor {
            filters
                .builder()
                .push(" AND worker_id < ")
                .push_bind(cursor.worker_id);
        }

        let mut builder = filters.finalize();
        builder.push(" ORDER BY worker_id DESC");
        push_limit(&mut builder, limit);

        let rows: Vec<WorkerRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        if rows.is_empty() {
            return Ok(PaginatedResult::empty());
        }

        // Collect unique (namespace_id, task_queue) pairs to batch concurrency lookups.
        let mut seen = HashSet::new();
        let mut pairs = Vec::new();
        for r in &rows {
            let key = (r.namespace_id.clone(), r.task_queue.clone());
            if seen.insert(key.clone()) {
                pairs.push(key);
            }
        }

        let queue_limits = self.fetch_queue_limits_for_pairs(&pairs).await?;
        let workflow_limits = self.fetch_workflow_limits_for_pairs(&pairs).await?;

        let has_more = rows.len() > params.page_size as usize;
        let mut items = Vec::with_capacity(rows.len().min(params.page_size as usize));
        for r in rows.into_iter().take(params.page_size as usize) {
            let key = (r.namespace_id.clone(), r.task_queue.clone());
            let queue_limit = queue_limits.get(&key).cloned().flatten();
            let type_limits = workflow_limits.get(&key).cloned().unwrap_or_default();
            items.push(r.into_model(queue_limit, type_limits));
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

impl PgWorkerRepository {
    async fn fetch_queue_limits_for_pairs(
        &self,
        pairs: &[(String, String)],
    ) -> Result<HashMap<(String, String), Option<i32>>, StoreError> {
        if pairs.is_empty() {
            return Ok(HashMap::new());
        }

        #[derive(sqlx::FromRow)]
        struct QueueLimitRow {
            namespace_id: String,
            task_queue: String,
            max_concurrent: Option<i32>,
        }

        let mut builder = QueryBuilder::<Postgres>::new(
            "SELECT namespace_id, task_queue, max_concurrent \
             FROM kagzi.queue_configs \
             WHERE (namespace_id, task_queue) IN (",
        );
        builder.push_tuples(pairs, |mut b, (ns, tq)| {
            b.push_bind(ns);
            b.push_bind(tq);
        });
        builder.push(")");

        let rows: Vec<QueueLimitRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            map.insert(
                (row.namespace_id, row.task_queue),
                row.max_concurrent, // keep Option<i32>, flatten later
            );
        }

        Ok(map)
    }

    async fn fetch_workflow_limits_for_pairs(
        &self,
        pairs: &[(String, String)],
    ) -> Result<HashMap<(String, String), Vec<WorkflowTypeConcurrency>>, StoreError> {
        if pairs.is_empty() {
            return Ok(HashMap::new());
        }

        #[derive(sqlx::FromRow)]
        struct WorkflowLimitRow {
            namespace_id: String,
            task_queue: String,
            workflow_type: String,
            max_concurrent: Option<i32>,
        }

        let mut builder = QueryBuilder::<Postgres>::new(
            "SELECT namespace_id, task_queue, workflow_type, max_concurrent \
             FROM kagzi.workflow_type_configs \
             WHERE (namespace_id, task_queue) IN (",
        );
        builder.push_tuples(pairs, |mut b, (ns, tq)| {
            b.push_bind(ns);
            b.push_bind(tq);
        });
        builder.push(")");

        let rows: Vec<WorkflowLimitRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        let mut map: HashMap<(String, String), Vec<WorkflowTypeConcurrency>> = HashMap::new();
        for row in rows {
            map.entry((row.namespace_id, row.task_queue))
                .or_default()
                .push(WorkflowTypeConcurrency {
                    workflow_type: row.workflow_type,
                    // NULL in DB means "no explicit limit configured".
                    // Represented as 0 and later filtered by normalize_limit() in service layer.
                    max_concurrent: row.max_concurrent.unwrap_or(0),
                });
        }

        Ok(map)
    }

    async fn fetch_concurrency(
        &self,
        namespace_id: &str,
        task_queue: &str,
    ) -> Result<(Option<i32>, Vec<WorkflowTypeConcurrency>), StoreError> {
        let queue_limit = sqlx::query_scalar!(
            r#"
            SELECT max_concurrent
            FROM kagzi.queue_configs
            WHERE namespace_id = $1 AND task_queue = $2
            "#,
            namespace_id,
            task_queue
        )
        .fetch_optional(&self.pool)
        .await?;

        let rows = sqlx::query!(
            r#"
            SELECT workflow_type, max_concurrent
            FROM kagzi.workflow_type_configs
            WHERE namespace_id = $1 AND task_queue = $2
            "#,
            namespace_id,
            task_queue
        )
        .fetch_all(&self.pool)
        .await?;

        let type_limits = rows
            .into_iter()
            .map(|r| WorkflowTypeConcurrency {
                workflow_type: r.workflow_type,
                // NULL in DB means "no explicit limit configured".
                // Represented as 0 and later filtered by normalize_limit() in service layer.
                max_concurrent: r.max_concurrent.unwrap_or(0),
            })
            .collect();

        Ok((queue_limit.flatten(), type_limits))
    }
}
