use tracing::instrument;

use super::PgWorkflowRepository;
use super::helpers::ClaimedRow;
use crate::error::StoreError;
use crate::models::ClaimedWorkflow;

/// Poll for an available workflow to execute.
///
/// Claims a workflow by setting status to RUNNING and available_at to the future.
/// Uses FOR UPDATE SKIP LOCKED to prevent race conditions between workers.
///
/// A workflow is available when:
/// - status is PENDING, SLEEPING, or RUNNING (orphaned)
/// - available_at <= NOW()
#[instrument(skip(repo, types))]
pub(super) async fn poll_workflow(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    task_queue: &str,
    worker_id: &str,
    types: &[String],
    visibility_timeout_secs: i64,
) -> Result<Option<ClaimedWorkflow>, StoreError> {
    let row = sqlx::query_as!(
        ClaimedRow,
        r#"
        WITH task AS (
            SELECT run_id
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1 
              AND task_queue = $2
              AND status IN ('PENDING', 'SLEEPING', 'RUNNING')
              AND available_at <= NOW()
              AND (array_length($3::TEXT[], 1) IS NULL 
                   OR array_length($3::TEXT[], 1) = 0 
                   OR workflow_type = ANY($3))
            ORDER BY available_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE kagzi.workflow_runs w
        SET status = 'RUNNING',
            locked_by = $4,
            available_at = NOW() + ($5 * interval '1 second'),
            started_at = COALESCE(started_at, NOW()),
            attempts = attempts + 1
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
        visibility_timeout_secs as f64
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
