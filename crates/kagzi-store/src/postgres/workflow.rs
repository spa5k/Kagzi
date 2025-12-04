use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedWorkflows,
    RetryPolicy, WorkflowCursor, WorkflowExistsResult, WorkflowRun, WorkflowStatus,
};
use crate::repository::WorkflowRepository;

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

#[derive(Clone)]
pub struct PgWorkflowRepository {
    pool: PgPool,
}

impl PgWorkflowRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowRepository for PgWorkflowRepository {
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError> {
        let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;

        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                business_id, task_queue, workflow_type, status, input,
                namespace_id, idempotency_key, context, deadline_at, version, retry_policy
            )
            VALUES ($1, $2, $3, 'PENDING', $4, $5, $6, $7, $8, $9, $10)
            RETURNING run_id
            "#,
            params.business_id,
            params.task_queue,
            params.workflow_type,
            params.input,
            params.namespace_id,
            params.idempotency_key,
            params.context,
            params.deadline_at,
            params.version,
            retry_policy_json
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.run_id)
    }

    async fn find_by_id(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<WorkflowRun>, StoreError> {
        let row = sqlx::query_as::<_, WorkflowRunRow>(
            r#"
            SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                   input, output, context, locked_by, attempts, error,
                   created_at, started_at, finished_at, wake_up_at, deadline_at,
                   version, parent_step_attempt_id, retry_policy
            FROM kagzi.workflow_runs
            WHERE run_id = $1 AND namespace_id = $2
            "#,
        )
        .bind(run_id)
        .bind(namespace_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()))
    }

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
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id, retry_policy
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1
                    ORDER BY created_at DESC, run_id DESC
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
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id, retry_policy
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1 AND status = $2
                    ORDER BY created_at DESC, run_id DESC
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
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id, retry_policy
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1 AND (created_at, run_id) < ($2, $3)
                    ORDER BY created_at DESC, run_id DESC
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
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id, retry_policy
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1 AND status = $2 AND (created_at, run_id) < ($3, $4)
                    ORDER BY created_at DESC, run_id DESC
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
        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'COMPLETED',
                output = $2,
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL
            WHERE run_id = $1
            "#,
            run_id,
            output
        )
        .execute(&self.pool)
        .await?;

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

    async fn claim_next(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        let row = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'RUNNING',
                locked_by = $1,
                locked_until = NOW() + INTERVAL '30 seconds',
                started_at = COALESCE(started_at, NOW()),
                attempts = attempts + 1
            WHERE run_id = (
                SELECT run_id
                FROM kagzi.workflow_runs
                WHERE task_queue = $2
                  AND namespace_id = $3
                  AND (
                    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                  )
                ORDER BY COALESCE(wake_up_at, created_at) ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING run_id, workflow_type, input
            "#,
            worker_id,
            task_queue,
            namespace_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ClaimedWorkflow {
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            input: r.input,
        }))
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

    async fn wake_sleeping(&self) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'PENDING',
                wake_up_at = NULL
            WHERE status = 'SLEEPING'
              AND wake_up_at <= NOW()
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
