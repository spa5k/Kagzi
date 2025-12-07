use async_trait::async_trait;
use sqlx::{PgPool, QueryBuilder};
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedWorkflows,
    RetryPolicy, WorkCandidate, WorkflowCursor, WorkflowExistsResult, WorkflowRun, WorkflowStatus,
};
use crate::postgres::columns;
use crate::postgres::query::{FilterBuilder, push_limit, push_tuple_cursor};
use crate::repository::WorkflowRepository;

const DEFAULT_QUEUE_CONCURRENCY_LIMIT: i32 = 10_000;

#[derive(sqlx::FromRow)]
struct WorkflowRunRow {
    run_id: Uuid,
    namespace_id: String,
    external_id: String,
    task_queue: String,
    workflow_type: String,
    status: WorkflowStatus,
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
            external_id: self.external_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            status: self.status,
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

#[derive(sqlx::FromRow)]
struct CandidateRow {
    run_id: Uuid,
    workflow_type: String,
    wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone)]
pub struct PgWorkflowRepository {
    pool: PgPool,
}

impl PgWorkflowRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

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

    async fn set_failed(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
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

#[async_trait]
impl WorkflowRepository for PgWorkflowRepository {
    #[instrument(skip(self, params))]
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError> {
        let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                external_id, task_queue, workflow_type, status,
                namespace_id, idempotency_suffix, deadline_at, version, retry_policy
            )
            VALUES ($1, $2, $3, 'PENDING', $4, $5, $6, $7, $8)
            RETURNING run_id
            "#,
            params.external_id,
            params.task_queue,
            params.workflow_type,
            params.namespace_id,
            params.idempotency_suffix,
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

    #[instrument(skip(self))]
    async fn find_by_id(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<WorkflowRun>, StoreError> {
        let query = format!(
            "SELECT {} FROM kagzi.workflow_runs w \
             JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id \
             WHERE w.run_id = $1 AND w.namespace_id = $2",
            columns::workflow::WITH_PAYLOAD
        );
        let row = sqlx::query_as::<_, WorkflowRunRow>(&query)
            .bind(run_id)
            .bind(namespace_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| r.into_model()))
    }

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
    async fn find_active_by_external_id(
        &self,
        namespace_id: &str,
        external_id: &str,
        idempotency_suffix: Option<&str>,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT run_id FROM kagzi.workflow_runs
            WHERE namespace_id = $1
              AND external_id = $2
              AND COALESCE(idempotency_suffix, '') = COALESCE($3, '')
              AND status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED')
            "#,
            namespace_id,
            external_id,
            idempotency_suffix
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.run_id))
    }

    #[instrument(skip(self, params))]
    async fn list(&self, params: ListWorkflowsParams) -> Result<PaginatedWorkflows, StoreError> {
        let limit = (params.page_size + 1) as i64;

        let mut filters = FilterBuilder::select(
            columns::workflow::WITH_PAYLOAD,
            "kagzi.workflow_runs w JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id",
        );
        filters.and_eq("w.namespace_id", &params.namespace_id);

        if let Some(ref status) = params.filter_status {
            filters.and_eq("w.status", status);
        }

        if let Some(ref cursor) = params.cursor {
            push_tuple_cursor(
                filters.builder(),
                &["w.created_at", "w.run_id"],
                "<",
                |b: &mut QueryBuilder<'_, _>| {
                    b.push_bind(cursor.created_at)
                        .push(", ")
                        .push_bind(cursor.run_id);
                },
            );
        }

        let mut builder = filters.finalize();
        builder.push(" ORDER BY w.created_at DESC, w.run_id DESC");
        push_limit(&mut builder, limit);

        let rows: Vec<WorkflowRunRow> = builder.build_query_as().fetch_all(&self.pool).await?;

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

    #[instrument(skip(self))]
    async fn check_exists(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT status as "status: WorkflowStatus", locked_by FROM kagzi.workflow_runs
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
                status: Some(r.status),
                locked_by: r.locked_by,
            }),
            None => Ok(WorkflowExistsResult {
                exists: false,
                status: None,
                locked_by: None,
            }),
        }
    }

    #[instrument(skip(self))]
    async fn check_status(&self, run_id: Uuid) -> Result<WorkflowExistsResult, StoreError> {
        let row = sqlx::query!(
            r#"
            SELECT status as "status: WorkflowStatus", locked_by FROM kagzi.workflow_runs
            WHERE run_id = $1
            "#,
            run_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(WorkflowExistsResult {
                exists: true,
                status: Some(r.status),
                locked_by: r.locked_by,
            }),
            None => Ok(WorkflowExistsResult {
                exists: false,
                status: None,
                locked_by: None,
            }),
        }
    }

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        self.set_failed(run_id, error).await
    }

    #[instrument(skip(self))]
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

    #[instrument(skip(self, supported_types))]
    async fn claim_next_filtered(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        let queue_limit = self.queue_limit(namespace_id, task_queue).await?;

        // Count predicates align with partial index `idx_workflow_runs_running` on
        // (task_queue, namespace_id, workflow_type) WHERE status = 'RUNNING'.
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
                  AND (
                    array_length($4::TEXT[], 1) IS NULL
                    OR array_length($4::TEXT[], 1) = 0
                    OR workflow_type = ANY($4::TEXT[])
                  )
                  AND (
                    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                  )
                  AND (SELECT queue_limit FROM limits) > COALESCE(
                    (SELECT SUM(active_count) FROM kagzi.workers w
                     WHERE w.task_queue = $2 AND w.namespace_id = $3 AND w.status = 'ONLINE'),
                    0
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

    #[instrument(skip(self, supported_types))]
    async fn scan_available(
        &self,
        task_queue: &str,
        namespace_id: &str,
        supported_types: &[String],
        limit: i32,
    ) -> Result<Vec<WorkCandidate>, StoreError> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"
            SELECT run_id, workflow_type, wake_up_at
            FROM kagzi.workflow_runs
            WHERE task_queue = $1
              AND namespace_id = $2
              AND (
                array_length($3::TEXT[], 1) IS NULL
                OR array_length($3::TEXT[], 1) = 0
                OR workflow_type = ANY($3::TEXT[])
              )
              AND (
                (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                OR (status = 'SLEEPING' AND wake_up_at <= NOW())
              )
            ORDER BY COALESCE(wake_up_at, created_at) ASC
            LIMIT $4
            "#,
        )
        .bind(task_queue)
        .bind(namespace_id)
        .bind(supported_types)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| WorkCandidate {
                run_id: r.run_id,
                workflow_type: r.workflow_type,
                wake_up_at: r.wake_up_at,
            })
            .collect())
    }

    #[instrument(skip(self))]
    async fn claim_by_id(
        &self,
        run_id: Uuid,
        worker_id: &str,
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        let row = sqlx::query_as::<_, ClaimedRow>(
            r#"
            WITH claimed AS (
                UPDATE kagzi.workflow_runs
                SET status = 'RUNNING',
                    locked_by = $2,
                    locked_until = NOW() + INTERVAL '30 seconds',
                    started_at = COALESCE(started_at, NOW()),
                    attempts = attempts + 1
                WHERE run_id = $1
                  AND (
                    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                  )
                  AND EXISTS (
                    SELECT 1
                    FROM kagzi.workflow_runs wr
                    WHERE wr.run_id = $1
                      AND COALESCE(
                            (
                              SELECT max_concurrent
                              FROM kagzi.queue_configs qc
                              WHERE qc.namespace_id = wr.namespace_id AND qc.task_queue = wr.task_queue
                            ),
                            $3::INT
                          ) > COALESCE(
                            (
                              SELECT SUM(active_count)
                              FROM kagzi.workers w
                              WHERE w.task_queue = wr.task_queue
                                AND w.namespace_id = wr.namespace_id
                                AND w.status = 'ONLINE'
                            ),
                            0
                          )
                  )
                  AND EXISTS (
                    SELECT 1
                    FROM kagzi.workflow_runs wr
                    WHERE wr.run_id = $1
                      AND COALESCE(
                            (
                              SELECT max_concurrent
                              FROM kagzi.workflow_type_configs cfg
                              WHERE cfg.namespace_id = wr.namespace_id
                                AND cfg.task_queue = wr.task_queue
                                AND cfg.workflow_type = wr.workflow_type
                            ),
                            COALESCE(
                              (
                                SELECT max_concurrent
                                FROM kagzi.queue_configs qc
                                WHERE qc.namespace_id = wr.namespace_id AND qc.task_queue = wr.task_queue
                              ),
                              $3::INT
                            )
                          ) > (
                            SELECT COUNT(*)
                            FROM kagzi.workflow_runs r
                            WHERE r.task_queue = wr.task_queue
                              AND r.namespace_id = wr.namespace_id
                              AND r.workflow_type = wr.workflow_type
                              AND r.status = 'RUNNING'
                          )
                  )
                RETURNING run_id, workflow_type, locked_by
            )
            SELECT c.run_id, c.workflow_type, p.input, c.locked_by
            FROM claimed c
            JOIN kagzi.workflow_payloads p ON c.run_id = p.run_id
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .bind(DEFAULT_QUEUE_CONCURRENCY_LIMIT)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ClaimedWorkflow {
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            input: r.input,
            locked_by: r.locked_by,
        }))
    }

    #[instrument(skip(self))]
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

    #[instrument(skip(self, run_ids))]
    async fn extend_locks_batch(
        &self,
        run_ids: &[Uuid],
        duration_secs: i64,
    ) -> Result<u64, StoreError> {
        if run_ids.is_empty() {
            return Ok(0);
        }

        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET locked_until = NOW() + ($2 * INTERVAL '1 second')
            WHERE run_id = ANY($1)
            "#,
            run_ids,
            duration_secs as f64
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    #[instrument(skip(self, params))]
    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError> {
        if params.is_empty() {
            return Ok(Vec::new());
        }

        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(params.len());

        for p in params {
            let retry_policy_json = p.retry_policy.map(serde_json::to_value).transpose()?;
            let row = sqlx::query!(
                r#"
                INSERT INTO kagzi.workflow_runs (
                    external_id, task_queue, workflow_type, status,
                    namespace_id, idempotency_suffix, deadline_at, version, retry_policy
                )
                VALUES ($1, $2, $3, 'PENDING', $4, $5, $6, $7, $8)
                RETURNING run_id
                "#,
                p.external_id,
                p.task_queue,
                p.workflow_type,
                p.namespace_id,
                p.idempotency_suffix,
                p.deadline_at,
                p.version,
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
            .bind(&p.input)
            .bind(&p.context)
            .execute(&mut *tx)
            .await?;

            ids.push(row.run_id);
        }

        tx.commit().await?;

        Ok(ids)
    }

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
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

    #[instrument(skip(self))]
    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        self.set_failed(run_id, error).await
    }
}
