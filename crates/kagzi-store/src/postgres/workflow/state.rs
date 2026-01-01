use sqlx::QueryBuilder;
use tracing::{instrument, warn};
use uuid::Uuid;

use super::super::StoreConfig;
use super::PgWorkflowRepository;
use super::helpers::{WorkflowRunRow, set_failed_tx};
use crate::error::StoreError;
use crate::models::{
    CreateWorkflow, ListWorkflowsParams, PaginatedResult, RetryPolicy, WorkflowCursor,
    WorkflowExistsResult, WorkflowRun,
};

const WORKFLOW_COLUMNS_WITH_PAYLOAD: &str = "\
    w.run_id, w.namespace_id, w.external_id, w.task_queue, w.workflow_type, \
    w.status, p.input, p.output, w.locked_by, w.attempts, w.error, \
    w.created_at, w.started_at, w.finished_at, w.available_at, \
    w.version, w.parent_step_attempt_id, w.retry_policy, w.cron_expr, w.schedule_id";

fn validate_payload_size(
    config: &StoreConfig,
    bytes: &[u8],
    context: &str,
) -> Result<(), StoreError> {
    let size = bytes.len();
    if size > config.payload_max_size_bytes {
        return Err(StoreError::invalid_argument(format!(
            "{} exceeds maximum size of {} bytes ({} bytes). Do not use Kagzi for blob storage.",
            context, config.payload_max_size_bytes, size
        )));
    }

    if size > config.payload_warn_threshold_bytes {
        warn!(
            size_bytes = size,
            context = context,
            "Payload exceeds {} bytes. Consider storing large data externally.",
            config.payload_warn_threshold_bytes
        );
    }

    Ok(())
}

#[instrument(skip(repo, params))]
pub(super) async fn create(
    repo: &PgWorkflowRepository,
    params: CreateWorkflow,
) -> Result<Uuid, StoreError> {
    let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;
    validate_payload_size(&repo.config, &params.input, "Workflow input")?;
    let mut tx = repo.pool.begin().await?;
    let run_id = params.run_id;

    let row = sqlx::query!(
        r#"
        INSERT INTO kagzi.workflow_runs (
            run_id,
            external_id, task_queue, workflow_type, status,
            namespace_id, version, retry_policy, available_at,
            cron_expr, schedule_id
        )
        VALUES ($1, $2, $3, $4, 'PENDING', $5, $6, $7, NOW(), $8, $9)
        RETURNING run_id
        "#,
        run_id,
        params.external_id,
        params.task_queue,
        params.workflow_type,
        params.namespace_id,
        params.version,
        retry_policy_json,
        params.cron_expr,
        params.schedule_id
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO kagzi.workflow_payloads (run_id, input)
        VALUES ($1, $2)
        "#,
        row.run_id,
        params.input
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(row.run_id)
}

#[instrument(skip(repo))]
pub(super) async fn find_by_id(
    repo: &PgWorkflowRepository,
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
            w.locked_by,
            w.attempts,
            w.error,
            w.created_at,
            w.started_at,
            w.finished_at,
            w.available_at,
            w.version,
            w.parent_step_attempt_id,
            w.retry_policy,
            w.cron_expr,
            w.schedule_id
        FROM kagzi.workflow_runs w
        JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
        WHERE w.run_id = $1 AND w.namespace_id = $2
        "#,
        run_id,
        namespace_id
    )
    .fetch_optional(&repo.pool)
    .await?;

    row.map(|r| r.into_model()).transpose()
}

#[instrument(skip(repo))]
pub(super) async fn get_retry_policy(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
) -> Result<Option<RetryPolicy>, StoreError> {
    let row = sqlx::query!(
        r#"
        SELECT retry_policy
        FROM kagzi.workflow_runs
        WHERE run_id = $1
        "#,
        run_id
    )
    .fetch_optional(&repo.pool)
    .await?;

    Ok(row.and_then(|r| r.retry_policy).and_then(|v| {
        serde_json::from_value::<RetryPolicy>(v)
            .map_err(|e| {
                tracing::warn!(
                    run_id = %run_id,
                    error = %e,
                    "Failed to deserialize retry_policy; defaulting to None"
                );
            })
            .ok()
    }))
}

#[instrument(skip(repo))]
pub(super) async fn find_active_by_external_id(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    external_id: &str,
) -> Result<Option<Uuid>, StoreError> {
    let row = sqlx::query!(
        r#"
        SELECT run_id FROM kagzi.workflow_runs
        WHERE namespace_id = $1
          AND external_id = $2
          AND status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED')
        "#,
        namespace_id,
        external_id
    )
    .fetch_optional(&repo.pool)
    .await?;

    Ok(row.map(|r| r.run_id))
}

#[instrument(skip(repo, params))]
pub(super) async fn list(
    repo: &PgWorkflowRepository,
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

    if let Some(schedule_id) = params.schedule_id {
        builder.push(" AND w.schedule_id = ").push_bind(schedule_id);
    }

    builder.push(" ORDER BY w.created_at DESC, w.run_id DESC LIMIT ");
    builder.push_bind(limit);

    let rows: Vec<WorkflowRunRow> = builder.build_query_as().fetch_all(&repo.pool).await?;

    let has_more = rows.len() > params.page_size as usize;
    let items: Vec<WorkflowRun> = rows
        .into_iter()
        .take(params.page_size as usize)
        .map(|r| r.into_model())
        .collect::<Result<_, _>>()?;

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

#[instrument(skip(repo, filter_status))]
pub(super) async fn count(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    filter_status: Option<&str>,
) -> Result<i64, StoreError> {
    let count = if let Some(status) = filter_status {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) as count
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1
              AND status = $2
            "#,
        )
        .bind(namespace_id)
        .bind(status)
        .fetch_one(&repo.pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) as count
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1
            "#,
        )
        .bind(namespace_id)
        .fetch_one(&repo.pool)
        .await?
    };

    Ok(count)
}

#[instrument(skip(repo))]
pub(super) async fn check_exists(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    namespace_id: &str,
) -> Result<WorkflowExistsResult, StoreError> {
    check_workflow_state(repo, run_id, namespace_id).await
}

#[instrument(skip(repo))]
pub(super) async fn check_status(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    namespace_id: &str,
) -> Result<WorkflowExistsResult, StoreError> {
    check_workflow_state(repo, run_id, namespace_id).await
}

#[instrument(skip(repo))]
async fn check_workflow_state(
    repo: &PgWorkflowRepository,
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
    .fetch_optional(&repo.pool)
    .await?;

    match row {
        Some(r) => Ok(WorkflowExistsResult {
            exists: true,
            status: Some(r.status.parse().map_err(|_| {
                StoreError::invalid_state(format!("invalid workflow status: {}", r.status))
            })?),
            locked_by: r.locked_by,
        }),
        None => Ok(WorkflowExistsResult {
            exists: false,
            status: None,
            locked_by: None,
        }),
    }
}

#[instrument(skip(repo))]
pub(super) async fn cancel(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    namespace_id: &str,
) -> Result<bool, StoreError> {
    let mut tx = repo.pool.begin().await?;

    let running = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'CANCELLED',
            finished_at = NOW(),
            locked_by = NULL,
            available_at = NULL
        WHERE run_id = $1 
          AND namespace_id = $2
          AND status = 'RUNNING'
        RETURNING run_id, namespace_id, task_queue, workflow_type
        "#,
        run_id,
        namespace_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    if running.is_some() {
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
            available_at = NULL
        WHERE run_id = $1 
          AND namespace_id = $2
          AND status IN ('PENDING', 'SLEEPING')
        RETURNING run_id
        "#,
        run_id,
        namespace_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(non_running.is_some())
}

#[instrument(skip(repo))]
pub(super) async fn complete(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    output: Vec<u8>,
) -> Result<(), StoreError> {
    validate_payload_size(&repo.config, &output, "Workflow output")?;
    let mut tx = repo.pool.begin().await?;

    let counters = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'COMPLETED',
            finished_at = NOW(),
            locked_by = NULL,
            available_at = NULL
        WHERE run_id = $1 AND status = 'RUNNING'
        RETURNING namespace_id, task_queue, workflow_type
        "#,
        run_id
    )
    .fetch_optional(&mut *tx)
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
    .execute(&mut *tx)
    .await?;

    let _ = counters;

    tx.commit().await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn fail(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    error: &str,
) -> Result<(), StoreError> {
    let mut tx = repo.pool.begin().await?;

    let _ = set_failed_tx(&mut tx, run_id, error).await?;

    tx.commit().await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn schedule_sleep(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    duration_secs: u64,
) -> Result<(), StoreError> {
    let duration = duration_secs as f64;
    let mut tx = repo.pool.begin().await?;
    let row = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'SLEEPING',
            available_at = NOW() + ($2 * INTERVAL '1 second'),
            locked_by = NULL
        WHERE run_id = $1 AND status = 'RUNNING'
        RETURNING namespace_id, task_queue, workflow_type
        "#,
        run_id,
        duration
    )
    .fetch_optional(&mut *tx)
    .await?;

    let _ = row;

    tx.commit().await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn schedule_retry(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    delay_ms: u64,
) -> Result<(), StoreError> {
    let mut tx = repo.pool.begin().await?;
    let row = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'PENDING',
            locked_by = NULL,
            available_at = NOW() + ($2 * INTERVAL '1 millisecond')
        WHERE run_id = $1 AND status = 'RUNNING'
        RETURNING namespace_id, task_queue, workflow_type
        "#,
        run_id,
        delay_ms as f64
    )
    .fetch_optional(&mut *tx)
    .await?;

    let _ = row;

    tx.commit().await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn mark_exhausted(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    error: &str,
) -> Result<(), StoreError> {
    let mut tx = repo.pool.begin().await?;

    let _ = set_failed_tx(&mut tx, run_id, error).await?;

    tx.commit().await?;

    Ok(())
}

#[instrument(skip(repo, params))]
pub(super) async fn create_batch(
    repo: &PgWorkflowRepository,
    params: Vec<CreateWorkflow>,
) -> Result<Vec<Uuid>, StoreError> {
    if params.is_empty() {
        return Ok(Vec::new());
    }

    let mut tx = repo.pool.begin().await?;
    let mut ids = Vec::with_capacity(params.len());

    for p in params {
        validate_payload_size(&repo.config, &p.input, "Workflow input")?;
        let retry_policy_json = p.retry_policy.map(serde_json::to_value).transpose()?;
        let run_id = p.run_id;
        let row = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                run_id,
                external_id, task_queue, workflow_type, status,
                namespace_id, version, retry_policy, available_at,
                cron_expr, schedule_id
            )
            VALUES ($1, $2, $3, $4, 'PENDING', $5, $6, $7, NOW(), $8, $9)
            RETURNING run_id
            "#,
            run_id,
            p.external_id,
            p.task_queue,
            p.workflow_type,
            p.namespace_id,
            p.version,
            retry_policy_json,
            p.cron_expr,
            p.schedule_id
        )
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_payloads (run_id, input)
            VALUES ($1, $2)
            "#,
            row.run_id,
            p.input
        )
        .execute(&mut *tx)
        .await?;

        ids.push(row.run_id);
    }

    tx.commit().await?;

    Ok(ids)
}

#[instrument(skip(repo))]
pub(super) async fn find_due_schedules(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    now: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<Vec<WorkflowRun>, StoreError> {
    let rows = if namespace_id == "*" {
        // Query all namespaces
        sqlx::query_as!(
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
                w.locked_by,
                w.attempts,
                w.error,
                w.created_at,
                w.started_at,
                w.finished_at,
                w.available_at,
                w.version,
                w.parent_step_attempt_id,
                w.retry_policy,
                w.cron_expr,
                w.schedule_id
            FROM kagzi.workflow_runs w
            JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
            WHERE w.status = 'SCHEDULED'
              AND w.available_at <= $1
            ORDER BY w.available_at ASC
            LIMIT $2
            "#,
            now,
            limit
        )
        .fetch_all(&repo.pool)
        .await?
    } else {
        // Query specific namespace
        sqlx::query_as!(
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
                w.locked_by,
                w.attempts,
                w.error,
                w.created_at,
                w.started_at,
                w.finished_at,
                w.available_at,
                w.version,
                w.parent_step_attempt_id,
                w.retry_policy,
                w.cron_expr,
                w.schedule_id
            FROM kagzi.workflow_runs w
            JOIN kagzi.workflow_payloads p ON w.run_id = p.run_id
            WHERE w.namespace_id = $1
              AND w.status = 'SCHEDULED'
              AND w.available_at <= $2
            ORDER BY w.available_at ASC
            LIMIT $3
            "#,
            namespace_id,
            now,
            limit
        )
        .fetch_all(&repo.pool)
        .await?
    };

    rows.into_iter().map(|r| r.into_model()).collect()
}

#[instrument(skip(repo))]
pub(super) async fn create_schedule_instance(
    repo: &PgWorkflowRepository,
    template_run_id: Uuid,
    fire_at: chrono::DateTime<chrono::Utc>,
) -> Result<Option<Uuid>, StoreError> {
    let new_run_id = Uuid::now_v7();

    let result = sqlx::query!(
        r#"
        WITH template AS (
            SELECT * FROM kagzi.workflow_runs WHERE run_id = $1
        ),
        payload AS (
            SELECT input FROM kagzi.workflow_payloads WHERE run_id = $1
        ),
        inserted AS (
            INSERT INTO kagzi.workflow_runs (
                run_id, namespace_id, external_id, task_queue, workflow_type,
                status, available_at, schedule_id, version, retry_policy
            )
            SELECT
                $2,                          -- new run_id
                t.namespace_id,
                $3,                          -- generated external_id
                t.task_queue,
                t.workflow_type,
                'PENDING',
                $4,                          -- fire_at
                t.run_id,                    -- schedule_id = template
                t.version,
                t.retry_policy
            FROM template t
            ON CONFLICT (schedule_id, available_at)
                WHERE schedule_id IS NOT NULL
                DO NOTHING
            RETURNING run_id
        )
        INSERT INTO kagzi.workflow_payloads (run_id, input)
        SELECT i.run_id, p.input
        FROM inserted i, payload p
        RETURNING run_id
        "#,
        template_run_id,
        new_run_id,
        format!("schedule-instance-{}", new_run_id),
        fire_at
    )
    .fetch_optional(&repo.pool)
    .await?;

    Ok(result.map(|r| r.run_id))
}

#[instrument(skip(repo))]
pub(super) async fn update_next_fire(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    next_fire_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), StoreError> {
    sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET available_at = $2
        WHERE run_id = $1
        "#,
        run_id,
        next_fire_at
    )
    .execute(&repo.pool)
    .await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn update(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    workflow: WorkflowRun,
) -> Result<(), StoreError> {
    sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = $2,
            available_at = $3,
            cron_expr = $4,
            schedule_id = $5
        WHERE run_id = $1
        "#,
        run_id,
        workflow.status.to_string(),
        workflow.available_at,
        workflow.cron_expr,
        workflow.schedule_id
    )
    .execute(&repo.pool)
    .await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn delete(repo: &PgWorkflowRepository, run_id: Uuid) -> Result<(), StoreError> {
    sqlx::query!(
        r#"
        DELETE FROM kagzi.workflow_payloads WHERE run_id = $1
        "#,
        run_id
    )
    .execute(&repo.pool)
    .await?;

    sqlx::query!(
        r#"
        DELETE FROM kagzi.workflow_runs WHERE run_id = $1
        "#,
        run_id
    )
    .execute(&repo.pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(max: usize, warn: usize) -> StoreConfig {
        StoreConfig {
            payload_warn_threshold_bytes: warn,
            payload_max_size_bytes: max,
        }
    }

    #[test]
    fn workflow_payload_rejects_when_over_limit() {
        let cfg = config(2, 1);
        let data = vec![0u8; 3];
        let err = validate_payload_size(&cfg, &data, "Workflow input")
            .expect_err("should reject oversized payload");
        assert!(matches!(err, StoreError::InvalidArgument { .. }));
    }
}
