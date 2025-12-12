use tracing::instrument;
use uuid::Uuid;

use super::PgWorkflowRepository;
use super::helpers::ClaimedRow;
use crate::error::StoreError;
use crate::models::{ClaimedWorkflow, OrphanedWorkflow, RetryPolicy};

#[instrument(skip(repo, types))]
pub(super) async fn poll_workflow(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    task_queue: &str,
    worker_id: &str,
    types: &[String],
    lock_secs: i64,
) -> Result<Option<ClaimedWorkflow>, StoreError> {
    let row = sqlx::query_as!(
        ClaimedRow,
        r#"
        WITH task AS (
            SELECT run_id
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1 AND task_queue = $2
              AND (
                  (status = 'PENDING') OR
                  (status = 'SLEEPING' AND wake_up_at <= NOW()) OR
                  (status = 'RUNNING' AND locked_until < NOW())
              )
              AND (array_length($3::TEXT[], 1) IS NULL OR array_length($3::TEXT[], 1) = 0 OR workflow_type = ANY($3))
            ORDER BY wake_up_at ASC NULLS FIRST, created_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE kagzi.workflow_runs w
        SET status = 'RUNNING',
            locked_by = $4,
            locked_until = NOW() + ($5 * interval '1 second'),
            started_at = COALESCE(started_at, NOW()),
            wake_up_at = NULL
        FROM task
        WHERE w.run_id = task.run_id
        RETURNING w.run_id,
                  w.workflow_type,
                  (SELECT input FROM kagzi.workflow_payloads p WHERE p.run_id = w.run_id) as "input!",
                  w.locked_by
        "#,
        namespace_id,
        task_queue,
        types,
        worker_id,
        lock_secs as f64
    )
    .fetch_optional(&repo.pool)
    .await?;

    Ok(row.map(|r| ClaimedWorkflow {
        run_id: r.run_id,
        workflow_type: r.workflow_type,
        input: r.input,
        locked_by: r.locked_by,
    }))
}

#[instrument(skip(repo))]
pub(super) async fn extend_worker_locks(
    repo: &PgWorkflowRepository,
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
    .execute(&repo.pool)
    .await?;

    Ok(result.rows_affected())
}

#[instrument(skip(repo, run_ids))]
pub(super) async fn extend_locks_for_runs(
    repo: &PgWorkflowRepository,
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
    .execute(&repo.pool)
    .await?;

    Ok(result.rows_affected())
}

#[instrument(skip(repo))]
pub(super) async fn wake_sleeping(
    repo: &PgWorkflowRepository,
    batch_size: i64,
) -> Result<u64, StoreError> {
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
            LIMIT $1
        )
        "#,
        batch_size
    )
    .execute(&repo.pool)
    .await?;

    let rows_affected = result.rows_affected();
    if rows_affected > 0 {
        tracing::info!(rows_affected, "Woke up sleeping workflows");
    }

    Ok(rows_affected)
}

#[instrument(skip(repo))]
pub(super) async fn find_orphaned(
    repo: &PgWorkflowRepository,
) -> Result<Vec<OrphanedWorkflow>, StoreError> {
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
    .fetch_all(&repo.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| OrphanedWorkflow {
            run_id: r.run_id,
            locked_by: r.locked_by,
            attempts: r.attempts,
            retry_policy: r.retry_policy.and_then(|v| {
                serde_json::from_value::<RetryPolicy>(v)
                    .map_err(|e| {
                        tracing::warn!(
                            run_id = %r.run_id,
                            error = %e,
                            "Failed to deserialize retry_policy; defaulting to None"
                        );
                    })
                    .ok()
            }),
        })
        .collect())
}
