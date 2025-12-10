use sqlx::{Postgres, Transaction};
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{ClaimedWorkflow, OrphanedWorkflow, RetryPolicy, WorkCandidate};

use super::helpers::{CandidateWithLimits, ClaimedRow, try_increment_counter_tx};
use super::{DEFAULT_QUEUE_CONCURRENCY_LIMIT, PgWorkflowRepository};

#[derive(sqlx::FromRow)]
struct CandidateRow {
    run_id: Uuid,
    workflow_type: String,
    wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn claim_with_counters(
    tx: &mut Transaction<'_, Postgres>,
    candidate: &CandidateWithLimits,
    worker_id: &str,
) -> Result<Option<ClaimedRow>, StoreError> {
    if !try_increment_counter_tx(
        tx,
        &candidate.namespace_id,
        &candidate.task_queue,
        "",
        candidate.queue_limit,
    )
    .await?
    {
        return Ok(None);
    }

    if !try_increment_counter_tx(
        tx,
        &candidate.namespace_id,
        &candidate.task_queue,
        &candidate.workflow_type,
        candidate.type_limit,
    )
    .await?
    {
        return Ok(None);
    }

    let claimed = sqlx::query_as!(
        ClaimedRow,
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'RUNNING', locked_by = $1, locked_until = NOW() + INTERVAL '30 seconds',
            started_at = COALESCE(started_at, NOW()), attempts = attempts + 1
        WHERE run_id = $2
        RETURNING run_id, workflow_type, 
                  (SELECT input FROM kagzi.workflow_payloads WHERE run_id = $2) as "input!",
                  locked_by
        "#,
        worker_id,
        candidate.run_id
    )
    .fetch_one(tx.as_mut())
    .await?;

    Ok(Some(claimed))
}

#[instrument(skip(repo, supported_types))]
pub(super) async fn claim_next_workflow(
    repo: &PgWorkflowRepository,
    task_queue: &str,
    namespace_id: &str,
    worker_id: &str,
    supported_types: &[String],
) -> Result<Option<ClaimedWorkflow>, StoreError> {
    let mut tx = repo.pool.begin().await?;

    let candidate = sqlx::query_as!(
        CandidateWithLimits,
        r#"
        SELECT
            wr.run_id,
            wr.namespace_id,
            wr.task_queue,
            wr.workflow_type,
            COALESCE(qc.max_concurrent, $4) AS "queue_limit!",
            COALESCE(tc.max_concurrent, qc.max_concurrent, $4) AS "type_limit!"
        FROM kagzi.workflow_runs wr
        LEFT JOIN kagzi.queue_configs qc ON qc.namespace_id = wr.namespace_id AND qc.task_queue = wr.task_queue
        LEFT JOIN kagzi.workflow_type_configs tc ON tc.namespace_id = wr.namespace_id 
            AND tc.task_queue = wr.task_queue AND tc.workflow_type = wr.workflow_type
        WHERE wr.task_queue = $1 AND wr.namespace_id = $2
          AND (array_length($3::TEXT[], 1) IS NULL OR array_length($3::TEXT[], 1) = 0 OR wr.workflow_type = ANY($3))
          AND ((wr.status = 'PENDING' AND (wr.wake_up_at IS NULL OR wr.wake_up_at <= NOW()))
               OR (wr.status = 'SLEEPING' AND wr.wake_up_at <= NOW()))
        ORDER BY COALESCE(wr.wake_up_at, wr.created_at) ASC
        FOR UPDATE OF wr SKIP LOCKED
        LIMIT 1
        "#,
        task_queue,
        namespace_id,
        supported_types,
        DEFAULT_QUEUE_CONCURRENCY_LIMIT
    )
    .fetch_optional(tx.as_mut())
    .await?;

    let Some(c) = candidate else {
        return Ok(None);
    };

    let claimed = claim_with_counters(&mut tx, &c, worker_id).await?;
    if let Some(r) = claimed {
        tx.commit().await?;
        return Ok(Some(ClaimedWorkflow {
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            input: r.input,
            locked_by: r.locked_by,
        }));
    }

    Ok(None)
}

#[instrument(skip(repo))]
pub(super) async fn list_available_workflows(
    repo: &PgWorkflowRepository,
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
    .fetch_all(&repo.pool)
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

#[instrument(skip(repo))]
pub(super) async fn claim_specific_workflow(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    worker_id: &str,
) -> Result<Option<ClaimedWorkflow>, StoreError> {
    let mut tx = repo.pool.begin().await?;

    let candidate = sqlx::query_as!(
        CandidateWithLimits,
        r#"
        SELECT
            wr.run_id,
            wr.namespace_id,
            wr.task_queue,
            wr.workflow_type,
            COALESCE(qc.max_concurrent, $2) AS "queue_limit!",
            COALESCE(tc.max_concurrent, qc.max_concurrent, $2) AS "type_limit!"
        FROM kagzi.workflow_runs wr
        LEFT JOIN kagzi.queue_configs qc ON qc.namespace_id = wr.namespace_id AND qc.task_queue = wr.task_queue
        LEFT JOIN kagzi.workflow_type_configs tc ON tc.namespace_id = wr.namespace_id 
            AND tc.task_queue = wr.task_queue AND tc.workflow_type = wr.workflow_type
        WHERE wr.run_id = $1
          AND ((wr.status = 'PENDING' AND (wr.wake_up_at IS NULL OR wr.wake_up_at <= NOW()))
               OR (wr.status = 'SLEEPING' AND wr.wake_up_at <= NOW()))
        FOR UPDATE OF wr SKIP LOCKED
        "#,
        run_id,
        DEFAULT_QUEUE_CONCURRENCY_LIMIT
    )
    .fetch_optional(tx.as_mut())
    .await?;

    let Some(c) = candidate else {
        return Ok(None);
    };

    let claimed = claim_with_counters(&mut tx, &c, worker_id).await?;
    if let Some(r) = claimed {
        tx.commit().await?;
        return Ok(Some(ClaimedWorkflow {
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            input: r.input,
            locked_by: r.locked_by,
        }));
    }

    Ok(None)
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
pub(super) async fn wake_sleeping(repo: &PgWorkflowRepository) -> Result<u64, StoreError> {
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
    .execute(&repo.pool)
    .await?;

    Ok(result.rows_affected())
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
            retry_policy: r
                .retry_policy
                .and_then(|v| serde_json::from_value::<RetryPolicy>(v).ok()),
        })
        .collect())
}

#[instrument(skip(repo))]
pub(super) async fn increment_queue_counter(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    task_queue: &str,
    workflow_type: &str,
    max: i32,
) -> Result<bool, StoreError> {
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
    .fetch_optional(&repo.pool)
    .await?;

    Ok(result.is_some())
}

#[instrument(skip(repo))]
pub(super) async fn decrement_queue_counter(
    repo: &PgWorkflowRepository,
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
    .execute(&repo.pool)
    .await?;

    Ok(())
}

#[instrument(skip(repo))]
pub(super) async fn reconcile_queue_counters(
    repo: &PgWorkflowRepository,
) -> Result<u64, StoreError> {
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
    .execute(&repo.pool)
    .await?;

    Ok(result.rows_affected())
}
