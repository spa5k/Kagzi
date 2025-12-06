use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ListWorkersParams, RegisterWorkerParams, Worker, WorkerHeartbeatParams, WorkerStatus,
    WorkflowTypeConcurrency,
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
    /// Converts this database row representation into the public `Worker` model,
    /// attaching queue-level and per-workflow-type concurrency limits.
    ///
    /// The returned `Worker` preserves all fields from the row and sets
    /// `queue_concurrency_limit` and `workflow_type_concurrency` to the provided values.
    ///
    /// # Examples
    ///
    /// ```
    /// // given a `WorkerRow` named `row` loaded from the DB:
    /// let worker = row.into_model(Some(10), vec![]);
    /// assert_eq!(worker.queue_concurrency_limit, Some(10));
    /// ```
    fn into_model(
        self,
        queue_concurrency_limit: Option<i32>,
        workflow_type_concurrency: Vec<WorkflowTypeConcurrency>,
    ) -> Worker {
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
    /// Registers a new worker and persists optional concurrency configuration.
    ///
    /// Inserts a row into `kagzi.workers` for the provided `params`. If `params.queue_concurrency_limit` is set,
    /// upserts the queue-level concurrency in `kagzi.queue_configs`. For each entry in
    /// `params.workflow_type_concurrency`, upserts a per-workflow-type concurrency row in
    /// `kagzi.workflow_type_configs`. All operations are executed in a single transaction.
    ///
    /// # Arguments
    ///
    /// * `params` - Registration parameters containing namespace, task queue, host/pid, version, workflow types,
    ///   max concurrent workers, labels, and optional queue/workflow-type concurrency limits.
    ///
    /// # Returns
    ///
    /// The newly created worker's `Uuid`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(repo: &PgWorkerRepository, params: RegisterWorkerParams) -> Result<(), Box<dyn std::error::Error>> {
    /// let worker_id = repo.register(params).await?;
    /// println!("created worker: {}", worker_id);
    /// # Ok(()) }
    /// ```
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

    /// Fetches a worker by its UUID and attaches the queue-level and per-workflow-type concurrency settings.
    ///
    /// The lookup returns the worker row from storage and enriches it with the queue concurrency limit
    /// and the workflow-type concurrency configuration before converting to the public `Worker` model.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use uuid::Uuid;
    /// # async fn example(repo: &crate::PgWorkerRepository, id: Uuid) -> anyhow::Result<()> {
    /// let maybe_worker = repo.find_by_id(id).await?;
    /// if let Some(worker) = maybe_worker {
    ///     println!("Found worker: {}", worker.worker_id);
    /// } else {
    ///     println!("Worker not found");
    /// }
    /// # Ok(()) }
    /// ```
    —
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

        if let Some(r) = row {
            let (queue_limit, type_limits) = self
                .fetch_concurrency(&r.namespace_id, &r.task_queue)
                .await?;
            Ok(Some(r.into_model(queue_limit, type_limits)))
        } else {
            Ok(None)
        }
    }

    /// Check whether a worker is currently ONLINE.
    ///
    /// Returns `true` if a worker with the provided `worker_id` exists and its status is `ONLINE`, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(repo: &crate::postgres::PgWorkerRepository, id: uuid::Uuid) -> Result<(), Box<dyn std::error::Error>> {
    /// let is_online = repo.validate_online(id).await?;
    /// assert!(matches!(is_online, true | false));
    /// # Ok(())
    /// # }
    /// ```
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

    /// Lists workers that match the provided filters and supports cursor-based pagination.
    ///
    /// The `params` filters results by `namespace_id`, optional `task_queue`, optional `filter_status`,
    /// and an optional `cursor` (exclusive upper bound on `worker_id`). The method fetches up to
    /// `page_size + 1` rows so callers can detect whether a next page exists.
    ///
    /// # Returns
    ///
    /// A `Vec<Worker>` containing the matched workers. When `page_size` is set, the result may contain
    /// one additional worker beyond `page_size` to indicate there is a next page.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(repo: &crate::PgWorkerRepository) -> Result<(), crate::StoreError> {
    /// use crate::store::ListWorkersParams;
    ///
    /// let params = ListWorkersParams {
    ///     namespace_id: "default".to_string(),
    ///     task_queue: None,
    ///     filter_status: None,
    ///     cursor: None,
    ///     page_size: 50,
    /// };
    ///
    /// let workers = repo.list(params).await?;
    /// assert!(workers.len() <= 51); // page_size + 1
    /// # Ok(())
    /// # }
    /// ```
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

        let mut workers = Vec::with_capacity(rows.len());
        for r in rows {
            let (queue_limit, type_limits) = self
                .fetch_concurrency(&r.namespace_id, &r.task_queue)
                .await?;
            workers.push(r.into_model(queue_limit, type_limits));
        }

        Ok(workers)
    }

    /// Marks workers whose last heartbeat is older than the given threshold as offline.
    ///
    /// Updates any worker with status not equal to `OFFLINE` and `last_heartbeat_at` earlier than
    /// now minus `threshold_secs` seconds, setting `status` to `OFFLINE` and `deregistered_at` to now.
    ///
    /// # Returns
    ///
    /// The number of rows that were updated.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(repo: &crate::PgWorkerRepository) -> Result<(), crate::StoreError> {
    /// let changed = repo.mark_stale_offline(60).await?; // mark workers stale for > 60 seconds
    /// assert!(changed >= 0);
    /// # Ok(())
    /// # }
    /// ```
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

    /// Counts workers for a namespace, optionally restricted to a task queue and/or status.
    ///
    /// If `task_queue` is `Some`, only workers assigned to that queue are counted.
    /// If `filter_status` is `Some`, only workers with that status are counted.
    ///
    /// # Returns
    ///
    /// The number of workers that match the provided criteria (0 if none).
    ///
    /// # Examples
    ///
    /// ```
    /// # tokio_test::block_on(async {
    /// // `repo` is a `PgWorkerRepository`.
    /// let n = repo.count("namespace-a", Some("default"), None).await.unwrap();
    /// assert!(n >= 0);
    /// # });
    /// ```
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

impl PgWorkerRepository {
    /// Fetches configured concurrency limits for a namespace and task queue.
    ///
    /// Returns a tuple with the optional per-queue `max_concurrent` (if configured) as the first element,
    /// and a `Vec<WorkflowTypeConcurrency>` listing per-workflow-type `max_concurrent` values for that queue.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(repo: &PgWorkerRepository) -> Result<(), StoreError> {
    /// let (queue_limit, type_limits) = repo.fetch_concurrency("namespace", "tasks").await?;
    /// if let Some(limit) = queue_limit {
    ///     println!("queue limit: {}", limit);
    /// }
    /// for tl in type_limits {
    ///     println!("workflow_type={} max_concurrent={}", tl.workflow_type, tl.max_concurrent);
    /// }
    /// # Ok(()) }
    /// ```
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
                max_concurrent: r.max_concurrent.unwrap_or(0),
            })
            .collect();

        Ok((queue_limit.flatten(), type_limits))
    }
}