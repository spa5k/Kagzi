use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedWorkflows,
    RetryPolicy, WorkflowCursor, WorkflowExistsResult, WorkflowRun, WorkflowStatus,
};
use crate::repository::WorkflowRepository;
use std::collections::HashMap;

const DEFAULT_QUEUE_CONCURRENCY_LIMIT: i32 = 10_000;

#[derive(sqlx::FromRow)]
struct WorkflowRunRow {
    run_id: Uuid,
    namespace_id: String,
    business_id: String,
    task_queue: String,
    workflow_type: String,
    status: String,
    input: serde_json::Value,
    output: Option<serde_json::Value>,
    context: Option<serde_json::Value>,
    locked_by: Option<String>,
    attempts: i32,
    error: Option<String>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    version: Option<String>,
    parent_step_attempt_id: Option<String>,
    retry_policy: Option<serde_json::Value>,
}

impl WorkflowRunRow {
    fn into_model(self) -> WorkflowRun {
        WorkflowRun {
            run_id: self.run_id,
            namespace_id: self.namespace_id,
            business_id: self.business_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            status: WorkflowStatus::from_db_str(&self.status),
            input: self.input,
            output: self.output,
            context: self.context,
            locked_by: self.locked_by,
            attempts: self.attempts,
            error: self.error,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            wake_up_at: self.wake_up_at,
            deadline_at: self.deadline_at,
            version: self.version,
            parent_step_attempt_id: self.parent_step_attempt_id,
            retry_policy: self
                .retry_policy
                .and_then(|v| serde_json::from_value(v).ok()),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ClaimedRow {
    run_id: Uuid,
    workflow_type: String,
    input: serde_json::Value,
    locked_by: Option<String>,
}

#[derive(Clone)]
pub struct PgWorkflowRepository {
    pool: PgPool,
}

impl PgWorkflowRepository {
    /// Creates a new PostgreSQL-backed workflow repository using the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```
    /// let pool: sqlx::PgPool = /* obtain a PgPool */;
    /// let repo = PgWorkflowRepository::new(pool);
    /// ```
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Resolve the maximum concurrent workflows allowed for a namespace and task queue.
    ///
    /// If a per-queue configuration exists, that value is used; otherwise the global
    /// DEFAULT_QUEUE_CONCURRENCY_LIMIT is used. The returned value is never greater
    /// than DEFAULT_QUEUE_CONCURRENCY_LIMIT.
    ///
    /// # Returns
    ///
    /// The concurrency limit for the specified namespace and task queue, capped at
    /// `DEFAULT_QUEUE_CONCURRENCY_LIMIT`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use kagzi_store::PgWorkflowRepository;
    /// # use sqlx::PgPool;
    /// # async fn doc_example() -> Result<(), Box<dyn std::error::Error>> {
    /// let pool = PgPool::connect("postgres://user:pass@localhost/db").await?;
    /// let repo = PgWorkflowRepository::new(pool);
    /// let limit = repo.queue_limit("my-namespace", "default").await?;
    /// assert!(limit > 0);
    /// # Ok(()) }
    /// ```
    async fn queue_limit(&self, namespace_id: &str, task_queue: &str) -> Result<i32, StoreError> {
        let limit = sqlx::query_scalar!(
            r#"
            SELECT max_concurrent FROM kagzi.queue_configs
            WHERE namespace_id = $1 AND task_queue = $2
            "#,
            namespace_id,
            task_queue
        )
        .fetch_optional(&self.pool)
        .await?;

        let limit = limit.flatten().unwrap_or(DEFAULT_QUEUE_CONCURRENCY_LIMIT);
        Ok(limit.min(DEFAULT_QUEUE_CONCURRENCY_LIMIT))
    }

    /// Fetches configured per-workflow-type concurrency limits for the given namespace and task queue.
    ///
    /// Returns a map where each key is a workflow type and the value is that type's `max_concurrent` limit
    /// as stored in `kagzi.workflow_type_configs`. Entries with a NULL `max_concurrent` are omitted.
    ///
    /// # Parameters
    ///
    /// - `namespace_id`: namespace identifier to scope the lookup.
    /// - `task_queue`: task queue name to scope the lookup.
    ///
    /// # Returns
    ///
    /// A `HashMap<String, i32>` mapping workflow type to its configured maximum concurrent runs.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(repo: &crate::PgWorkflowRepository) -> Result<(), crate::StoreError> {
    /// let limits = repo.workflow_type_limits("my-namespace", "default").await?;
    /// if let Some(&max) = limits.get("email_send") {
    ///     println!("email_send max concurrent: {}", max);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[allow(dead_code)]
    async fn workflow_type_limits(
        &self,
        namespace_id: &str,
        task_queue: &str,
    ) -> Result<HashMap<String, i32>, StoreError> {
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

        let mut map = HashMap::new();
        for r in rows {
            if let Some(limit) = r.max_concurrent {
                map.insert(r.workflow_type, limit);
            }
        }

        Ok(map)
    }
}

#[async_trait]
impl WorkflowRepository for PgWorkflowRepository {
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError> {
        let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                business_id, task_queue, workflow_type, status,
                namespace_id, idempotency_key, deadline_at, version, retry_policy
            )
            VALUES ($1, $2, $3, 'PENDING', $4, $5, $6, $7, $8)
            RETURNING run_id
            "#,
            params.business_id,
            params.task_queue,
            params.workflow_type,
            params.namespace_id,
            params.idempotency_key,
            params.deadline_at,
            params.version,
            retry_policy_json
        )
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO kagzi.workflow_payloads (run_id, input, context)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(row.run_id)
        .bind(&params.input)
        .bind(&params.context)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(row.run_id)
    }

    /// Fetches a workflow run by its ID within the given namespace.
    ///
    /// Returns `Ok(Some(workflow))` when a matching workflow run is found, `Ok(None)` when no matching run exists, or `Err(StoreError)` if the database query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use uuid::Uuid;
    /// # async fn example(repo: &impl crate::store::WorkflowRepository) {
    /// let run_id = Uuid::new_v4();
    /// let namespace = "default";
    /// let result = repo.find_by_id(run_id, namespace).await;
    /// match result {
    ///     Ok(Some(workflow)) => println!("Found workflow: {:?}", workflow.run_id),
    ///     Ok(None) => println!("No workflow found"),
    ///     Err(e) => eprintln!("Query failed: {:?}", e),
    /// }
    /// # }
    /// ```
    async fn find_by_id(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<WorkflowRun>, StoreError> {
        let row = sqlx::query_as::<_, WorkflowRunRow>(
            r#"
            SELECT w.run_id, w.namespace_id, w.business_id, w.task_queue, w.workflow_type, w.status,
                   p.input, p.output, p.context, w.locked_by, w.attempts, w.error,
                   w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at,
                   w.version, w.parent_step_attempt_id, w.retry_policy
            FROM kagzi.workflow_runs w
            JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
            WHERE w.run_id = $1 AND w.namespace_id = $2
            "#,
        )
        .bind(run_id)
        .bind(namespace_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()))
    }

    /// Fetches the retry policy for a workflow run from the database.
    ///
    /// Retrieves the `retry_policy` JSON stored for the given `run_id` and deserializes it into a
    /// `RetryPolicy`. If no row exists, the column is NULL, or deserialization fails, `None` is
    /// returned.
    ///
    /// # Examples
    ///
    /// ```
    /// # use uuid::Uuid;
    /// # async fn doc_example(repo: &crate::pg::PgWorkflowRepository, run_id: Uuid) -> Result<(), Box<dyn std::error::Error>> {
    /// let policy = repo.get_retry_policy(run_id).await?;
    /// // `policy` is `Some(RetryPolicy)` when a valid policy is stored, otherwise `None`.
    /// assert!(policy.is_none() || policy.is_some());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Returns
    ///
    /// `Some(RetryPolicy)` when a valid retry policy JSON is present for the run, `None` otherwise.
    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT retry_policy
            FROM kagzi.workflow_runs
            WHERE run_id = $1
            "#,
            run_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .and_then(|r| r.retry_policy)
            .and_then(|v| serde_json::from_value::<RetryPolicy>(v).ok()))
    }

    /// Finds a workflow run by its idempotency key within the given namespace.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use uuid::Uuid;
    /// # async fn example(repo: &crate::PgWorkflowRepository) -> Result<(), Box<dyn std::error::Error>> {
    /// let result = repo.find_by_idempotency_key("namespace-a", "idem-key-123").await?;
    /// if let Some(run_id) = result {
    ///     println!("Found run: {}", run_id);
    /// } else {
    ///     println!("No run found for that idempotency key");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// @returns `Some(run_id)` if a matching workflow exists, `None` otherwise.
    async fn find_by_idempotency_key(
        &self,
        namespace_id: &str,
        key: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT run_id FROM kagzi.workflow_runs
            WHERE namespace_id = $1 AND idempotency_key = $2
            "#,
            namespace_id,
            key
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.run_id))
    }

    async fn list(&self, params: ListWorkflowsParams) -> Result<PaginatedWorkflows, StoreError> {
        let limit = (params.page_size + 1) as i64;

        let rows: Vec<WorkflowRunRow> = match (&params.cursor, &params.filter_status) {
            (None, None) => {
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT w.run_id, w.namespace_id, w.business_id, w.task_queue, w.workflow_type, w.status,
                           p.input, p.output, p.context, w.locked_by, w.attempts, w.error,
                           w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at,
                           w.version, w.parent_step_attempt_id, w.retry_policy
                    FROM kagzi.workflow_runs w
                    JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
                    WHERE w.namespace_id = $1
                    ORDER BY w.created_at DESC, w.run_id DESC
                    LIMIT $2
                    "#,
                )
                .bind(&params.namespace_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(status)) => {
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT w.run_id, w.namespace_id, w.business_id, w.task_queue, w.workflow_type, w.status,
                           p.input, p.output, p.context, w.locked_by, w.attempts, w.error,
                           w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at,
                           w.version, w.parent_step_attempt_id, w.retry_policy
                    FROM kagzi.workflow_runs w
                    JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
                    WHERE w.namespace_id = $1 AND w.status = $2
                    ORDER BY w.created_at DESC, w.run_id DESC
                    LIMIT $3
                    "#,
                )
                .bind(&params.namespace_id)
                .bind(status)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(cursor), None) => {
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT w.run_id, w.namespace_id, w.business_id, w.task_queue, w.workflow_type, w.status,
                           p.input, p.output, p.context, w.locked_by, w.attempts, w.error,
                           w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at,
                           w.version, w.parent_step_attempt_id, w.retry_policy
                    FROM kagzi.workflow_runs w
                    JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
                    WHERE w.namespace_id = $1 AND (w.created_at, w.run_id) < ($2, $3)
                    ORDER BY w.created_at DESC, w.run_id DESC
                    LIMIT $4
                    "#,
                )
                .bind(&params.namespace_id)
                .bind(cursor.created_at)
                .bind(cursor.run_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(cursor), Some(status)) => {
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT w.run_id, w.namespace_id, w.business_id, w.task_queue, w.workflow_type, w.status,
                           p.input, p.output, p.context, w.locked_by, w.attempts, w.error,
                           w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at,
                           w.version, w.parent_step_attempt_id, w.retry_policy
                    FROM kagzi.workflow_runs w
                    JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
                    WHERE w.namespace_id = $1 AND w.status = $2 AND (w.created_at, w.run_id) < ($3, $4)
                    ORDER BY w.created_at DESC, w.run_id DESC
                    LIMIT $5
                    "#,
                )
                .bind(&params.namespace_id)
                .bind(status)
                .bind(cursor.created_at)
                .bind(cursor.run_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };

        let has_more = rows.len() > params.page_size as usize;
        let workflows: Vec<WorkflowRun> = rows
            .into_iter()
            .take(params.page_size as usize)
            .map(|r| r.into_model())
            .collect();

        let next_cursor = if has_more {
            workflows.last().and_then(|w| {
                w.created_at.map(|created_at| WorkflowCursor {
                    created_at,
                    run_id: w.run_id,
                })
            })
        } else {
            None
        };

        Ok(PaginatedWorkflows {
            workflows,
            next_cursor,
            has_more,
        })
    }

    async fn check_exists(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT status, locked_by FROM kagzi.workflow_runs
            WHERE run_id = $1 AND namespace_id = $2
            "#,
            run_id,
            namespace_id
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(WorkflowExistsResult {
                exists: true,
                status: Some(WorkflowStatus::from_db_str(&r.status)),
                locked_by: r.locked_by,
            }),
            None => Ok(WorkflowExistsResult {
                exists: false,
                status: None,
                locked_by: None,
            }),
        }
    }

    async fn cancel(&self, run_id: Uuid, namespace_id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'CANCELLED',
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1 
              AND namespace_id = $2
              AND status IN ('PENDING', 'RUNNING', 'SLEEPING')
            RETURNING run_id
            "#,
            run_id,
            namespace_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    async fn complete(&self, run_id: Uuid, output: serde_json::Value) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'COMPLETED',
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1
            "#,
            run_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE kagzi.workflow_payloads
            SET output = $2
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .bind(&output)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(())
    }

    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'FAILED',
                error = $2,
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1
            "#,
            run_id,
            error
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError> {
        let duration = duration_secs as f64;
        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'SLEEPING',
                wake_up_at = NOW() + ($2 * INTERVAL '1 second'),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1
            "#,
            run_id,
            duration
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Attempts to claim the next eligible workflow run from the given task queue for the specified worker.
    ///
    /// Respects per-queue and per-workflow-type concurrency limits, sets the run's status to `RUNNING`,
    /// assigns the lock to `worker_id`, extends the lock timeout, and returns the claimed workflow payload when successful.
    ///
    /// # Returns
    ///
    /// `Some(ClaimedWorkflow)` with the run's ID, type, input payload, and lock owner if a run was claimed, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use uuid::Uuid;
    /// # async fn example(repo: &crate::PgWorkflowRepository) -> Result<(), Box<dyn std::error::Error>> {
    /// let claimed = repo.claim_next("default", "my-namespace", "worker-1").await?;
    /// if let Some(c) = claimed {
    ///     println!("claimed run {}", c.run_id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn claim_next(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        let queue_limit = self.queue_limit(namespace_id, task_queue).await?;

        let row = sqlx::query_as::<_, ClaimedRow>(
            r#"
            WITH limits AS (
                SELECT $4::INT AS queue_limit
            ),
            eligible AS (
                SELECT run_id, workflow_type
                FROM kagzi.workflow_runs wr
                WHERE task_queue = $2
                  AND namespace_id = $3
                  AND (
                    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                  )
                  AND (SELECT queue_limit FROM limits) > (
                    SELECT COUNT(*) FROM kagzi.workflow_runs r
                    WHERE r.task_queue = $2 AND r.namespace_id = $3 AND r.status = 'RUNNING'
                  )
                  AND (
                    COALESCE(
                      (SELECT max_concurrent FROM kagzi.workflow_type_configs cfg
                       WHERE cfg.namespace_id = $3 AND cfg.task_queue = $2 AND cfg.workflow_type = wr.workflow_type),
                      (SELECT queue_limit FROM limits)
                    ) > (
                      SELECT COUNT(*) FROM kagzi.workflow_runs r
                      WHERE r.task_queue = $2 AND r.namespace_id = $3
                        AND r.workflow_type = wr.workflow_type
                        AND r.status = 'RUNNING'
                    )
                  )
                ORDER BY COALESCE(wake_up_at, created_at) ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            ),
            claimed AS (
                UPDATE kagzi.workflow_runs
                SET status = 'RUNNING',
                    locked_by = $1,
                    locked_until = NOW() + INTERVAL '30 seconds',
                    started_at = COALESCE(started_at, NOW()),
                    attempts = attempts + 1
                WHERE run_id = (SELECT run_id FROM eligible)
                RETURNING run_id, workflow_type, locked_by
            )
            SELECT c.run_id, c.workflow_type, p.input, c.locked_by
            FROM claimed c
            JOIN kagzi.workflow_payloads p ON c.run_id = p.run_id
            "#,
        )
        .bind(worker_id)
        .bind(task_queue)
        .bind(namespace_id)
        .bind(queue_limit)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ClaimedWorkflow {
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            input: r.input,
            locked_by: r.locked_by,
        }))
    }

    /// Claims the next eligible workflow from a task queue that matches any of the provided workflow types and marks it as `RUNNING`.
    ///
    /// The selected workflow will be locked to `worker_id`, its `locked_until` extended by 30 seconds, `started_at` set if not already, and `attempts` incremented. Eligibility respects per-queue and per-workflow-type concurrency limits and wakes/sleep conditions.
    ///
    /// # Parameters
    ///
    /// * `supported_types` — slice of workflow type names used to filter which workflows are eligible to be claimed.
    ///
    /// # Returns
    ///
    /// `Some(ClaimedWorkflow)` with the claimed run's id, type, input, and lock owner when a workflow was successfully claimed, `None` if no eligible workflow was found.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(repo: &impl kagzi_store::WorkflowRepository) -> Result<(), kagzi_store::StoreError> {
    /// let supported = vec!["email".to_string(), "pdf".to_string()];
    /// let claimed = repo.claim_next_filtered("default", "acct-1", "worker-42", &supported).await?;
    /// if let Some(claim) = claimed {
    ///     println!("Claimed run: {}", claim.run_id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn claim_next_filtered(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        let queue_limit = self.queue_limit(namespace_id, task_queue).await?;

        let row = sqlx::query_as::<_, ClaimedRow>(
            r#"
            WITH limits AS (
                SELECT $5::INT AS queue_limit
            ),
            eligible AS (
                SELECT run_id, workflow_type
                FROM kagzi.workflow_runs wr
                WHERE task_queue = $2
                  AND namespace_id = $3
                  AND workflow_type = ANY($4::TEXT[])
                  AND (
                    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                  )
                  AND (SELECT queue_limit FROM limits) > (
                    SELECT COUNT(*) FROM kagzi.workflow_runs r
                    WHERE r.task_queue = $2 AND r.namespace_id = $3 AND r.status = 'RUNNING'
                  )
                  AND (
                    COALESCE(
                      (SELECT max_concurrent FROM kagzi.workflow_type_configs cfg
                       WHERE cfg.namespace_id = $3 AND cfg.task_queue = $2 AND cfg.workflow_type = wr.workflow_type),
                      (SELECT queue_limit FROM limits)
                    ) > (
                      SELECT COUNT(*) FROM kagzi.workflow_runs r
                      WHERE r.task_queue = $2 AND r.namespace_id = $3
                        AND r.workflow_type = wr.workflow_type
                        AND r.status = 'RUNNING'
                    )
                  )
                ORDER BY COALESCE(wake_up_at, created_at) ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            ),
            claimed AS (
                UPDATE kagzi.workflow_runs
                SET status = 'RUNNING',
                    locked_by = $1,
                    locked_until = NOW() + INTERVAL '30 seconds',
                    started_at = COALESCE(started_at, NOW()),
                    attempts = attempts + 1
                WHERE run_id = (SELECT run_id FROM eligible)
                RETURNING run_id, workflow_type, locked_by
            )
            SELECT c.run_id, c.workflow_type, p.input, c.locked_by
            FROM claimed c
            JOIN kagzi.workflow_payloads p ON c.run_id = p.run_id
            "#,
        )
        .bind(worker_id)
        .bind(task_queue)
        .bind(namespace_id)
        .bind(supported_types)
        .bind(queue_limit)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ClaimedWorkflow {
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            input: r.input,
            locked_by: r.locked_by,
        }))
    }

    /// Attempts to claim up to `limit` workflows from the given task queue for the namespace.
    ///
    /// Repeatedly calls `claim_next` with unique batch worker IDs until `limit` items are claimed or no more eligible workflows are available.
    ///
    /// # Parameters
    ///
    /// - `task_queue`: the task queue to claim workflows from.
    /// - `namespace_id`: namespace to scope the claim to.
    /// - `limit`: maximum number of workflows to claim.
    ///
    /// # Returns
    ///
    /// A `Vec<ClaimedWorkflow>` containing the claimed workflows, with length at most `limit`.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn run_example(repo: &impl crate::store::WorkflowRepository, tq: &str, ns: &str) -> Result<(), crate::store::StoreError> {
    /// let items = repo.claim_batch(tq, ns, 5).await?;
    /// assert!(items.len() <= 5);
    /// # Ok(())
    /// # }
    /// ```
    async fn claim_batch(
        &self,
        task_queue: &str,
        namespace_id: &str,
        limit: i32,
    ) -> Result<Vec<ClaimedWorkflow>, StoreError> {
        let mut claimed = Vec::new();
        for _ in 0..limit {
            if let Some(item) = self
                .claim_next(
                    task_queue,
                    namespace_id,
                    &format!("batch-{}", uuid::Uuid::new_v4()),
                )
                .await?
            {
                claimed.push(item);
            } else {
                break;
            }
        }

        Ok(claimed)
    }

    /// Transfers the lock for a running workflow from one worker to another.
    ///
    /// Attempts to set `locked_by` to `to_worker_id` and extend `locked_until` by 30 seconds
    /// for the given `run_id` when the lock is currently held by `from_worker_id` and the
    /// workflow's status is `RUNNING`.
    ///
    /// # Returns
    ///
    /// `true` if the lock was transferred, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use uuid::Uuid;
    /// # async fn example(repo: &crate::PgWorkflowRepository) -> Result<(), crate::StoreError> {
    /// let run_id = Uuid::new_v4();
    /// let transferred = repo.transfer_lock(run_id, "worker-a", "worker-b").await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn transfer_lock(
        &self,
        run_id: Uuid,
        from_worker_id: &str,
        to_worker_id: &str,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET locked_by = $3,
                locked_until = NOW() + INTERVAL '30 seconds'
            WHERE run_id = $1
              AND locked_by = $2
              AND status = 'RUNNING'
            RETURNING run_id
            "#,
            run_id,
            from_worker_id,
            to_worker_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    async fn extend_lock(&self, run_id: Uuid, worker_id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET locked_until = NOW() + INTERVAL '30 seconds'
            WHERE run_id = $1
              AND locked_by = $2
              AND status = 'RUNNING'
            RETURNING run_id
            "#,
            run_id,
            worker_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    async fn extend_locks_for_worker(
        &self,
        worker_id: &str,
        duration_secs: i64,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET locked_until = NOW() + ($2 * INTERVAL '1 second')
            WHERE locked_by = $1
              AND status = 'RUNNING'
            "#,
            worker_id,
            duration_secs as f64
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn wake_sleeping(&self) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'PENDING',
                wake_up_at = NULL
            WHERE run_id IN (
                SELECT run_id
                FROM kagzi.workflow_runs
                WHERE status = 'SLEEPING'
                  AND wake_up_at <= NOW()
                FOR UPDATE SKIP LOCKED
                LIMIT 100
            )
            "#
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError> {
        let rows = sqlx::query!(
            r#"
            SELECT run_id, locked_by, attempts, retry_policy
            FROM kagzi.workflow_runs
            WHERE status = 'RUNNING'
              AND locked_until IS NOT NULL
              AND locked_until < NOW()
            FOR UPDATE SKIP LOCKED
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| OrphanedWorkflow {
                run_id: r.run_id,
                locked_by: r.locked_by,
                attempts: r.attempts,
                retry_policy: r
                    .retry_policy
                    .and_then(|v| serde_json::from_value::<RetryPolicy>(v).ok()),
            })
            .collect())
    }

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'PENDING',
                locked_by = NULL,
                locked_until = NULL,
                wake_up_at = NOW() + ($2 * INTERVAL '1 millisecond'),
                attempts = attempts + 1
            WHERE run_id = $1
            "#,
            run_id,
            delay_ms as f64
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'FAILED',
                error = $2,
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1
            "#,
            run_id,
            error
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}