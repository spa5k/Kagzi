use async_trait::async_trait;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedResult,
    RetryPolicy, WorkCandidate, WorkflowCursor, WorkflowExistsResult, WorkflowRun,
};
use crate::repository::WorkflowRepository;

const DEFAULT_QUEUE_CONCURRENCY_LIMIT: i32 = 10_000;
const WORKFLOW_COLUMNS_WITH_PAYLOAD: &str = "\
    w.run_id, w.namespace_id, w.external_id, w.task_queue, w.workflow_type, \
    w.status, p.input, p.output, p.context, w.locked_by, w.attempts, w.error, \
    w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at, \
    w.version, w.parent_step_attempt_id, w.retry_policy";

#[derive(sqlx::FromRow)]
struct WorkflowRunRow {
    run_id: Uuid,
    namespace_id: String,
    external_id: String,
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
            external_id: self.external_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            status: self
                .status
                .parse()
                .expect("status should be a valid WorkflowStatus"),
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

    async fn increment_counter_row(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
        max: i32,
    ) -> Result<bool, StoreError> {
        let max = max.max(1);
        let result = sqlx::query_scalar!(
            r#"
            INSERT INTO kagzi.queue_counters (namespace_id, task_queue, workflow_type, active_count)
            VALUES ($1, $2, $3, 1)
            ON CONFLICT (namespace_id, task_queue, workflow_type)
            DO UPDATE SET active_count = queue_counters.active_count + 1
            WHERE queue_counters.active_count < $4
            RETURNING active_count
            "#,
            namespace_id,
            task_queue,
            workflow_type,
            max
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    async fn decrement_counter_row(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.queue_counters
            SET active_count = GREATEST(active_count - 1, 0)
            WHERE namespace_id = $1 AND task_queue = $2 AND workflow_type = $3
            "#,
            namespace_id,
            task_queue,
            workflow_type
        )
        .execute(tx.as_mut())
        .await?;

        Ok(())
    }

    async fn decrement_counters_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
    ) -> Result<(), StoreError> {
        // Queue-level counter uses empty workflow_type
        self.decrement_counter_row(tx, namespace_id, task_queue, "")
            .await?;
        self.decrement_counter_row(tx, namespace_id, task_queue, workflow_type)
            .await
    }

    async fn set_failed_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run_id: Uuid,
        error: &str,
    ) -> Result<Option<(String, String, String)>, StoreError> {
        let row = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'FAILED',
                error = $2,
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1 AND status = 'RUNNING'
            RETURNING namespace_id, task_queue, workflow_type
            "#,
            run_id,
            error
        )
        .fetch_optional(tx.as_mut())
        .await?;

        Ok(row.map(|r| (r.namespace_id, r.task_queue, r.workflow_type)))
    }
}

#[async_trait]
impl WorkflowRepository for PgWorkflowRepository {
    #[instrument(skip(self, params))]
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError> {
        let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;
        let mut tx = self.pool.begin().await?;
        let run_id = Uuid::now_v7();

        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                run_id,
                external_id, task_queue, workflow_type, status,
                namespace_id, idempotency_suffix, deadline_at, version, retry_policy
            )
            VALUES ($1, $2, $3, $4, 'PENDING', $5, $6, $7, $8, $9)
            RETURNING run_id
            "#,
            run_id,
            params.external_id,
            params.task_queue,
            params.workflow_type,
            params.namespace_id,
            params.idempotency_suffix,
            params.deadline_at,
            params.version,
            retry_policy_json
        )
        .fetch_one(tx.as_mut())
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_payloads (run_id, input, context)
            VALUES ($1, $2, $3)
            "#,
            row.run_id,
            params.input,
            params.context
        )
        .execute(tx.as_mut())
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
        let row = sqlx::query_as!(
            WorkflowRunRow,
            r#"
            SELECT 
                w.run_id,
                w.namespace_id,
                w.external_id,
                w.task_queue,
                w.workflow_type,
                w.status,
                p.input,
                p.output,
                p.context,
                w.locked_by,
                w.attempts,
                w.error,
                w.created_at,
                w.started_at,
                w.finished_at,
                w.wake_up_at,
                w.deadline_at,
                w.version,
                w.parent_step_attempt_id,
                w.retry_policy
            FROM kagzi.workflow_runs w
            JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
            WHERE w.run_id = $1 AND w.namespace_id = $2
            "#,
            run_id,
            namespace_id
        )
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
    async fn list(
        &self,
        params: ListWorkflowsParams,
    ) -> Result<PaginatedResult<WorkflowRun, WorkflowCursor>, StoreError> {
        let limit = (params.page_size + 1) as i64;
        let mut builder = QueryBuilder::new("SELECT ");
        builder.push(WORKFLOW_COLUMNS_WITH_PAYLOAD);
        builder.push(" FROM kagzi.workflow_runs w JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id WHERE w.namespace_id = ");
        builder.push_bind(&params.namespace_id);

        if let Some(ref status) = params.filter_status {
            builder.push(" AND w.status = ").push_bind(status);
        }

        if let Some(ref cursor) = params.cursor {
            builder.push(" AND (w.created_at, w.run_id) < (");
            builder.push_bind(cursor.created_at);
            builder.push(", ");
            builder.push_bind(cursor.run_id);
            builder.push(")");
        }

        builder.push(" ORDER BY w.created_at DESC, w.run_id DESC LIMIT ");
        builder.push_bind(limit);

        let rows: Vec<WorkflowRunRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        let has_more = rows.len() > params.page_size as usize;
        let items: Vec<WorkflowRun> = rows
            .into_iter()
            .take(params.page_size as usize)
            .map(|r| r.into_model())
            .collect();

        let next_cursor = items.last().and_then(|w| {
            w.created_at.map(|created_at| WorkflowCursor {
                created_at,
                run_id: w.run_id,
            })
        });

        Ok(PaginatedResult {
            items,
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
                status: Some(
                    r.status
                        .parse()
                        .expect("status should be a valid WorkflowStatus"),
                ),
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
    async fn check_status(
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
            namespace_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(WorkflowExistsResult {
                exists: true,
                status: Some(
                    r.status
                        .parse()
                        .expect("status should be a valid WorkflowStatus"),
                ),
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
        let mut tx = self.pool.begin().await?;

        let running = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'CANCELLED',
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1 
              AND namespace_id = $2
              AND status = 'RUNNING'
            RETURNING run_id, namespace_id, task_queue, workflow_type
            "#,
            run_id,
            namespace_id
        )
        .fetch_optional(tx.as_mut())
        .await?;

        if let Some(row) = running {
            self.decrement_counters_tx(
                &mut tx,
                &row.namespace_id,
                &row.task_queue,
                &row.workflow_type,
            )
            .await?;
            tx.commit().await?;
            return Ok(true);
        }

        // Fallback: cancel non-running without touching counters
        let non_running = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'CANCELLED',
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1 
              AND namespace_id = $2
              AND status IN ('PENDING', 'SLEEPING')
            RETURNING run_id
            "#,
            run_id,
            namespace_id
        )
        .fetch_optional(tx.as_mut())
        .await?;

        tx.commit().await?;

        Ok(non_running.is_some())
    }

    #[instrument(skip(self))]
    async fn complete(&self, run_id: Uuid, output: serde_json::Value) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        let counters = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'COMPLETED',
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1 AND status = 'RUNNING'
            RETURNING namespace_id, task_queue, workflow_type
            "#,
            run_id
        )
        .fetch_optional(tx.as_mut())
        .await?;

        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_payloads
            SET output = $2
            WHERE run_id = $1
            "#,
            run_id,
            output
        )
        .execute(tx.as_mut())
        .await?;

        if let Some(row) = counters {
            self.decrement_counters_tx(
                &mut tx,
                &row.namespace_id,
                &row.task_queue,
                &row.workflow_type,
            )
            .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        if let Some((namespace_id, task_queue, workflow_type)) =
            self.set_failed_tx(&mut tx, run_id, error).await?
        {
            self.decrement_counters_tx(&mut tx, &namespace_id, &task_queue, &workflow_type)
                .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError> {
        let duration = duration_secs as f64;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'SLEEPING',
                wake_up_at = NOW() + ($2 * INTERVAL '1 second'),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1 AND status = 'RUNNING'
            RETURNING namespace_id, task_queue, workflow_type
            "#,
            run_id,
            duration
        )
        .fetch_optional(tx.as_mut())
        .await?;

        if let Some(row) = row {
            self.decrement_counters_tx(
                &mut tx,
                &row.namespace_id,
                &row.task_queue,
                &row.workflow_type,
            )
            .await?;
        }

        tx.commit().await?;

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
        // CTE execution order:
        // 1. `limits` - lock candidate row with FOR UPDATE SKIP LOCKED
        // 2. `queue_counter` - increment ONLY IF under queue limit (atomic conditional update)
        // 3. `type_counter` - increment ONLY IF queue_counter succeeded AND under type limit
        // 4. `revert_queue` - decrement queue counter IF queue succeeded but type failed
        // 5. `claimed` - update workflow IF both counters succeeded
        let row = sqlx::query_as!(
            ClaimedRow,
            r#"
            WITH limits AS (
                SELECT
                    wr.run_id,
                    wr.workflow_type,
                    wr.namespace_id,
                    wr.task_queue,
                    COALESCE(
                        (
                          SELECT max_concurrent
                          FROM kagzi.queue_configs qc
                          WHERE qc.namespace_id = $3 AND qc.task_queue = $2
                        ),
                        $5::INT
                    ) AS queue_limit,
                    COALESCE(
                        (
                          SELECT max_concurrent
                          FROM kagzi.workflow_type_configs cfg
                          WHERE cfg.namespace_id = $3 AND cfg.task_queue = $2 AND cfg.workflow_type = wr.workflow_type
                        ),
                        COALESCE(
                          (
                            SELECT max_concurrent
                            FROM kagzi.queue_configs qc
                            WHERE qc.namespace_id = $3 AND qc.task_queue = $2
                          ),
                          $5::INT
                        )
                    ) AS type_limit
                FROM kagzi.workflow_runs wr
                WHERE wr.task_queue = $2
                  AND wr.namespace_id = $3
                  AND (
                    array_length($4::TEXT[], 1) IS NULL
                    OR array_length($4::TEXT[], 1) = 0
                    OR wr.workflow_type = ANY($4::TEXT[])
                  )
                  AND (
                    (wr.status = 'PENDING' AND (wr.wake_up_at IS NULL OR wr.wake_up_at <= NOW()))
                    OR (wr.status = 'SLEEPING' AND wr.wake_up_at <= NOW())
                  )
                ORDER BY COALESCE(wr.wake_up_at, wr.created_at) ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            ),
            queue_counter AS (
                INSERT INTO kagzi.queue_counters (namespace_id, task_queue, workflow_type, active_count)
                SELECT namespace_id, task_queue, '', 1 FROM limits
                ON CONFLICT (namespace_id, task_queue, workflow_type)
                DO UPDATE SET active_count = queue_counters.active_count + 1
                WHERE queue_counters.active_count < (SELECT queue_limit FROM limits)
                RETURNING 1
            ),
            type_counter AS (
                INSERT INTO kagzi.queue_counters (namespace_id, task_queue, workflow_type, active_count)
                SELECT namespace_id, task_queue, workflow_type, 1 FROM limits
                WHERE EXISTS (SELECT 1 FROM queue_counter)
                ON CONFLICT (namespace_id, task_queue, workflow_type)
                DO UPDATE SET active_count = queue_counters.active_count + 1
                WHERE queue_counters.active_count < (SELECT type_limit FROM limits)
                RETURNING 1
            ),
            revert_queue AS (
                UPDATE kagzi.queue_counters
                SET active_count = GREATEST(active_count - 1, 0)
                WHERE EXISTS (SELECT 1 FROM queue_counter)
                  AND NOT EXISTS (SELECT 1 FROM type_counter)
                  AND namespace_id = (SELECT namespace_id FROM limits)
                  AND task_queue = (SELECT task_queue FROM limits)
                  AND workflow_type = ''
                RETURNING 1
            ),
            claimed AS (
                UPDATE kagzi.workflow_runs
                SET status = 'RUNNING',
                    locked_by = $1,
                    locked_until = NOW() + INTERVAL '30 seconds',
                    started_at = COALESCE(started_at, NOW()),
                    attempts = attempts + 1
                WHERE run_id IN (SELECT run_id FROM limits)
                  AND EXISTS (SELECT 1 FROM queue_counter)
                  AND EXISTS (SELECT 1 FROM type_counter)
                RETURNING run_id, workflow_type, locked_by
            )
            SELECT c.run_id, c.workflow_type, p.input, c.locked_by
            FROM claimed c
            JOIN kagzi.workflow_payloads p ON c.run_id = p.run_id
            "#,
            worker_id,
            task_queue,
            namespace_id,
            supported_types,
            DEFAULT_QUEUE_CONCURRENCY_LIMIT
        )
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
        let rows: Vec<CandidateRow> = sqlx::query_as!(
            CandidateRow,
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
            task_queue,
            namespace_id,
            supported_types,
            limit as i64
        )
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
        // CTE execution order:
        // 1. `limits` - lock candidate row with FOR UPDATE SKIP LOCKED
        // 2. `queue_counter` - increment ONLY IF under queue limit (atomic conditional update)
        // 3. `type_counter` - increment ONLY IF queue_counter succeeded AND under type limit
        // 4. `revert_queue` - decrement queue counter IF queue succeeded but type failed
        // 5. `claimed` - update workflow IF both counters succeeded
        let row = sqlx::query_as!(
            ClaimedRow,
            r#"
            WITH eligible AS (
                SELECT run_id, namespace_id, task_queue, workflow_type
                FROM kagzi.workflow_runs
                WHERE run_id = $1
                  AND (
                    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                  )
            ),
            limits AS (
                SELECT
                    e.run_id,
                    e.namespace_id,
                    e.task_queue,
                    e.workflow_type,
                    COALESCE(
                        (
                          SELECT max_concurrent
                          FROM kagzi.queue_configs qc
                          WHERE qc.namespace_id = e.namespace_id AND qc.task_queue = e.task_queue
                        ),
                        $3::INT
                    ) AS queue_limit,
                    COALESCE(
                        (
                          SELECT max_concurrent
                          FROM kagzi.workflow_type_configs cfg
                          WHERE cfg.namespace_id = e.namespace_id
                            AND cfg.task_queue = e.task_queue
                            AND cfg.workflow_type = e.workflow_type
                        ),
                        COALESCE(
                          (
                            SELECT max_concurrent
                            FROM kagzi.queue_configs qc
                            WHERE qc.namespace_id = e.namespace_id AND qc.task_queue = e.task_queue
                          ),
                          $3::INT
                        )
                    ) AS type_limit
                FROM eligible e
            ),
            queue_counter AS (
                INSERT INTO kagzi.queue_counters (namespace_id, task_queue, workflow_type, active_count)
                SELECT namespace_id, task_queue, '', 1 FROM limits
                ON CONFLICT (namespace_id, task_queue, workflow_type)
                DO UPDATE SET active_count = queue_counters.active_count + 1
                WHERE queue_counters.active_count < (SELECT queue_limit FROM limits)
                RETURNING 1
            ),
            type_counter AS (
                INSERT INTO kagzi.queue_counters (namespace_id, task_queue, workflow_type, active_count)
                SELECT namespace_id, task_queue, workflow_type, 1 FROM limits
                WHERE EXISTS (SELECT 1 FROM queue_counter)
                ON CONFLICT (namespace_id, task_queue, workflow_type)
                DO UPDATE SET active_count = queue_counters.active_count + 1
                WHERE queue_counters.active_count < (SELECT type_limit FROM limits)
                RETURNING 1
            ),
            revert_queue AS (
                UPDATE kagzi.queue_counters
                SET active_count = GREATEST(active_count - 1, 0)
                WHERE EXISTS (SELECT 1 FROM queue_counter)
                  AND NOT EXISTS (SELECT 1 FROM type_counter)
                  AND namespace_id = (SELECT namespace_id FROM limits)
                  AND task_queue = (SELECT task_queue FROM limits)
                  AND workflow_type = ''
                RETURNING 1
            ),
            claimed AS (
                UPDATE kagzi.workflow_runs
                SET status = 'RUNNING',
                    locked_by = $2,
                    locked_until = NOW() + INTERVAL '30 seconds',
                    started_at = COALESCE(started_at, NOW()),
                    attempts = attempts + 1
                WHERE run_id IN (SELECT run_id FROM limits)
                  AND EXISTS (SELECT 1 FROM queue_counter)
                  AND EXISTS (SELECT 1 FROM type_counter)
                RETURNING run_id, workflow_type, locked_by
            )
            SELECT c.run_id, c.workflow_type, p.input, c.locked_by
            FROM claimed c
            JOIN kagzi.workflow_payloads p ON c.run_id = p.run_id
            "#,
            run_id,
            worker_id,
            DEFAULT_QUEUE_CONCURRENCY_LIMIT
        )
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
            let run_id = Uuid::now_v7();
            let row = sqlx::query!(
                r#"
                INSERT INTO kagzi.workflow_runs (
                    run_id,
                    external_id, task_queue, workflow_type, status,
                    namespace_id, idempotency_suffix, deadline_at, version, retry_policy
                )
                VALUES ($1, $2, $3, $4, 'PENDING', $5, $6, $7, $8, $9)
                RETURNING run_id
                "#,
                run_id,
                p.external_id,
                p.task_queue,
                p.workflow_type,
                p.namespace_id,
                p.idempotency_suffix,
                p.deadline_at,
                p.version,
                retry_policy_json
            )
            .fetch_one(tx.as_mut())
            .await?;

            sqlx::query!(
                r#"
                INSERT INTO kagzi.workflow_payloads (run_id, input, context)
                VALUES ($1, $2, $3)
                "#,
                row.run_id,
                p.input,
                p.context
            )
            .execute(tx.as_mut())
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
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'PENDING',
                locked_by = NULL,
                locked_until = NULL,
                wake_up_at = NOW() + ($2 * INTERVAL '1 millisecond'),
                attempts = attempts + 1
            WHERE run_id = $1 AND status = 'RUNNING'
            RETURNING namespace_id, task_queue, workflow_type
            "#,
            run_id,
            delay_ms as f64
        )
        .fetch_optional(tx.as_mut())
        .await?;

        if let Some(row) = row {
            self.decrement_counters_tx(
                &mut tx,
                &row.namespace_id,
                &row.task_queue,
                &row.workflow_type,
            )
            .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        if let Some((namespace_id, task_queue, workflow_type)) =
            self.set_failed_tx(&mut tx, run_id, error).await?
        {
            self.decrement_counters_tx(&mut tx, &namespace_id, &task_queue, &workflow_type)
                .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn increment_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
        max: i32,
    ) -> Result<bool, StoreError> {
        self.increment_counter_row(namespace_id, task_queue, workflow_type, max)
            .await
    }

    #[instrument(skip(self))]
    async fn decrement_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.queue_counters
            SET active_count = GREATEST(active_count - 1, 0)
            WHERE namespace_id = $1 AND task_queue = $2 AND workflow_type = $3
            "#,
            namespace_id,
            task_queue,
            workflow_type
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn reconcile_counters(&self) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.queue_counters qc
            SET active_count = COALESCE((
                SELECT COUNT(*)::INT
                FROM kagzi.workflow_runs wr
                WHERE wr.status = 'RUNNING'
                  AND wr.namespace_id = qc.namespace_id
                  AND wr.task_queue = qc.task_queue
                  AND (qc.workflow_type = '' OR wr.workflow_type = qc.workflow_type)
            ), 0)
            "#
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}
