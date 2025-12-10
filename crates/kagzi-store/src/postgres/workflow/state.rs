use sqlx::QueryBuilder;
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    CreateWorkflow, ListWorkflowsParams, PaginatedResult, RetryPolicy, WorkflowCursor,
    WorkflowExistsResult, WorkflowRun,
};

use super::PgWorkflowRepository;
use super::helpers::{WorkflowRunRow, decrement_counters_tx, set_failed_tx};

const WORKFLOW_COLUMNS_WITH_PAYLOAD: &str = "\
    w.run_id, w.namespace_id, w.external_id, w.task_queue, w.workflow_type, \
    w.status, p.input, p.output, p.context, w.locked_by, w.attempts, w.error, \
    w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at, \
    w.version, w.parent_step_attempt_id, w.retry_policy";

#[instrument(skip(repo, params))]
pub(super) async fn create(
    repo: &PgWorkflowRepository,
    params: CreateWorkflow,
) -> Result<Uuid, StoreError> {
    let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;
    let mut tx = repo.pool.begin().await?;
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
    .fetch_one(&mut *tx)
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
            locked_until = NULL
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

    if let Some(row) = running {
        decrement_counters_tx(
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
    let mut tx = repo.pool.begin().await?;

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

    if let Some(row) = counters {
        decrement_counters_tx(
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

#[instrument(skip(repo))]
pub(super) async fn fail(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    error: &str,
) -> Result<(), StoreError> {
    let mut tx = repo.pool.begin().await?;

    if let Some((namespace_id, task_queue, workflow_type)) =
        set_failed_tx(&mut tx, run_id, error).await?
    {
        decrement_counters_tx(&mut tx, &namespace_id, &task_queue, &workflow_type).await?;
    }

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
            wake_up_at = NOW() + ($2 * INTERVAL '1 second'),
            locked_by = NULL,
            locked_until = NULL
        WHERE run_id = $1 AND status = 'RUNNING'
        RETURNING namespace_id, task_queue, workflow_type
        "#,
        run_id,
        duration
    )
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(row) = row {
        decrement_counters_tx(
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
            locked_until = NULL,
            wake_up_at = NOW() + ($2 * INTERVAL '1 millisecond'),
            attempts = attempts + 1
        WHERE run_id = $1 AND status = 'RUNNING'
        RETURNING namespace_id, task_queue, workflow_type
        "#,
        run_id,
        delay_ms as f64
    )
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(row) = row {
        decrement_counters_tx(
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

#[instrument(skip(repo))]
pub(super) async fn mark_exhausted(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    error: &str,
) -> Result<(), StoreError> {
    let mut tx = repo.pool.begin().await?;

    if let Some((namespace_id, task_queue, workflow_type)) =
        set_failed_tx(&mut tx, run_id, error).await?
    {
        decrement_counters_tx(&mut tx, &namespace_id, &task_queue, &workflow_type).await?;
    }

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
        .fetch_one(&mut *tx)
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
        .execute(&mut *tx)
        .await?;

        ids.push(row.run_id);
    }

    tx.commit().await?;

    Ok(ids)
}
