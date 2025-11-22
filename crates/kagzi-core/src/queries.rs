//! Database queries for workflow and step management

use crate::error::{Error, Result};
use crate::models::*;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::{debug, warn};
use uuid::Uuid;

/// Create a new workflow run
pub async fn create_workflow_run(pool: &PgPool, create: CreateWorkflowRun) -> Result<WorkflowRun> {
    let workflow_version = create.workflow_version.unwrap_or_else(|| "v1".to_string());

    let run = sqlx::query_as::<_, WorkflowRun>(
        r#"
        INSERT INTO workflow_runs (workflow_name, workflow_version, input, status)
        VALUES ($1, $2, $3, 'PENDING')
        RETURNING *
        "#,
    )
    .bind(&create.workflow_name)
    .bind(&workflow_version)
    .bind(&create.input)
    .fetch_one(pool)
    .await?;

    debug!("Created workflow run: {}", run.id);
    Ok(run)
}

/// Get a workflow run by ID
pub async fn get_workflow_run(pool: &PgPool, id: Uuid) -> Result<WorkflowRun> {
    sqlx::query_as::<_, WorkflowRun>("SELECT * FROM workflow_runs WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(Error::WorkflowNotFound(id))
}

/// Update workflow run status
pub async fn update_workflow_status(
    pool: &PgPool,
    id: Uuid,
    status: WorkflowStatus,
    error: Option<serde_json::Value>,
) -> Result<WorkflowRun> {
    let completed_at = if matches!(
        status,
        WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
    ) {
        Some(Utc::now())
    } else {
        None
    };

    let run = sqlx::query_as::<_, WorkflowRun>(
        r#"
        UPDATE workflow_runs
        SET status = $2, error = $3, completed_at = $4
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(error)
    .bind(completed_at)
    .fetch_one(pool)
    .await?;

    debug!("Updated workflow {} status to {}", id, run.status);
    Ok(run)
}

/// Update workflow run output
pub async fn update_workflow_output(
    pool: &PgPool,
    id: Uuid,
    output: serde_json::Value,
) -> Result<WorkflowRun> {
    let run = sqlx::query_as::<_, WorkflowRun>(
        "UPDATE workflow_runs SET output = $2 WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(output)
    .fetch_one(pool)
    .await?;

    debug!("Updated workflow {} output", id);
    Ok(run)
}

/// Set workflow to sleep until a specific time
pub async fn set_workflow_sleep(
    pool: &PgPool,
    id: Uuid,
    sleep_until: DateTime<Utc>,
) -> Result<WorkflowRun> {
    let run = sqlx::query_as::<_, WorkflowRun>(
        r#"
        UPDATE workflow_runs
        SET status = 'SLEEPING', sleep_until = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(sleep_until)
    .fetch_one(pool)
    .await?;

    debug!("Set workflow {} to sleep until {}", id, sleep_until);
    Ok(run)
}

/// Poll for next available workflow to execute
/// Uses FOR UPDATE SKIP LOCKED to ensure only one worker gets each workflow
pub async fn poll_next_workflow(pool: &PgPool) -> Result<Option<WorkflowRun>> {
    let now = Utc::now();

    let run = sqlx::query_as::<_, WorkflowRun>(
        r#"
        SELECT * FROM workflow_runs
        WHERE status = 'PENDING'
           OR (status = 'SLEEPING' AND sleep_until <= $1)
        ORDER BY created_at ASC
        LIMIT 1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(now)
    .fetch_optional(pool)
    .await?;

    if let Some(ref r) = run {
        debug!("Polled workflow: {}", r.id);
    }

    Ok(run)
}

/// Create or get a step run (for memoization)
pub async fn get_step_run(
    pool: &PgPool,
    workflow_run_id: Uuid,
    step_id: &str,
) -> Result<Option<StepRun>> {
    let step = sqlx::query_as::<_, StepRun>(
        "SELECT * FROM step_runs WHERE workflow_run_id = $1 AND step_id = $2",
    )
    .bind(workflow_run_id)
    .bind(step_id)
    .fetch_optional(pool)
    .await?;

    if step.is_some() {
        debug!(
            "Found cached step: workflow={}, step={}",
            workflow_run_id, step_id
        );
    }

    Ok(step)
}

/// Create a new step run
pub async fn create_step_run(pool: &PgPool, create: CreateStepRun) -> Result<StepRun> {
    let completed_at = if matches!(create.status, StepStatus::Completed | StepStatus::Failed) {
        Some(Utc::now())
    } else {
        None
    };

    let step = sqlx::query_as::<_, StepRun>(
        r#"
        INSERT INTO step_runs (workflow_run_id, step_id, input_hash, output, error, status, completed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
        "#,
    )
    .bind(create.workflow_run_id)
    .bind(&create.step_id)
    .bind(create.input_hash)
    .bind(create.output)
    .bind(create.error)
    .bind(create.status)
    .bind(completed_at)
    .fetch_one(pool)
    .await?;

    debug!(
        "Created step run: workflow={}, step={}",
        create.workflow_run_id, create.step_id
    );
    Ok(step)
}

/// Get all step runs for a workflow
pub async fn get_workflow_steps(pool: &PgPool, workflow_run_id: Uuid) -> Result<Vec<StepRun>> {
    let steps = sqlx::query_as::<_, StepRun>(
        "SELECT * FROM step_runs WHERE workflow_run_id = $1 ORDER BY created_at ASC",
    )
    .bind(workflow_run_id)
    .fetch_all(pool)
    .await?;

    Ok(steps)
}

/// Acquire a worker lease for a workflow
pub async fn acquire_worker_lease(
    pool: &PgPool,
    workflow_run_id: Uuid,
    worker_id: &str,
    lease_duration_secs: i64,
) -> Result<WorkerLease> {
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(lease_duration_secs);

    // Try to insert a new lease
    let result = sqlx::query_as::<_, WorkerLease>(
        r#"
        INSERT INTO worker_leases (workflow_run_id, worker_id, expires_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (workflow_run_id)
        DO UPDATE SET
            worker_id = EXCLUDED.worker_id,
            acquired_at = NOW(),
            expires_at = EXCLUDED.expires_at,
            heartbeat_at = NOW()
        WHERE worker_leases.expires_at < NOW()
        RETURNING *
        "#,
    )
    .bind(workflow_run_id)
    .bind(worker_id)
    .bind(expires_at)
    .fetch_optional(pool)
    .await?;

    match result {
        Some(lease) => {
            debug!("Acquired lease for workflow {}", workflow_run_id);
            Ok(lease)
        }
        None => {
            warn!("Failed to acquire lease for workflow {}", workflow_run_id);
            Err(Error::LeaseNotAcquired)
        }
    }
}

/// Release a worker lease
pub async fn release_worker_lease(pool: &PgPool, workflow_run_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM worker_leases WHERE workflow_run_id = $1")
        .bind(workflow_run_id)
        .execute(pool)
        .await?;

    debug!("Released lease for workflow {}", workflow_run_id);
    Ok(())
}

/// Update worker heartbeat
pub async fn update_worker_heartbeat(pool: &PgPool, workflow_run_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE worker_leases SET heartbeat_at = NOW() WHERE workflow_run_id = $1")
        .bind(workflow_run_id)
        .execute(pool)
        .await?;

    Ok(())
}

// ============================================================================
// Retry-related queries
// ============================================================================

/// Update step retry state (attempts, next_retry_at, retry_policy)
pub async fn update_step_retry_state(
    pool: &PgPool,
    workflow_run_id: Uuid,
    step_id: &str,
    attempts: i32,
    next_retry_at: Option<DateTime<Utc>>,
    retry_policy: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
) -> Result<StepRun> {
    let step = sqlx::query_as::<_, StepRun>(
        r#"
        UPDATE step_runs
        SET attempts = $3,
            next_retry_at = $4,
            retry_policy = $5,
            error = $6,
            status = 'FAILED'
        WHERE workflow_run_id = $1 AND step_id = $2
        RETURNING *
        "#,
    )
    .bind(workflow_run_id)
    .bind(step_id)
    .bind(attempts)
    .bind(next_retry_at)
    .bind(retry_policy)
    .bind(error)
    .fetch_one(pool)
    .await?;

    debug!(
        "Updated retry state: workflow={}, step={}, attempts={}, next_retry={:?}",
        workflow_run_id, step_id, attempts, next_retry_at
    );

    Ok(step)
}

/// Create a step attempt record
pub async fn create_step_attempt(
    pool: &PgPool,
    step_run_id: i64,
    attempt_number: i32,
) -> Result<StepAttempt> {
    let attempt = sqlx::query_as::<_, StepAttempt>(
        r#"
        INSERT INTO step_attempts (step_run_id, attempt_number, status)
        VALUES ($1, $2, 'running')
        RETURNING *
        "#,
    )
    .bind(step_run_id)
    .bind(attempt_number)
    .fetch_one(pool)
    .await?;

    debug!(
        "Created step attempt: step_run_id={}, attempt={}",
        step_run_id, attempt_number
    );

    Ok(attempt)
}

/// Update step attempt with result
pub async fn update_step_attempt(
    pool: &PgPool,
    id: i64,
    status: &str,
    error: Option<serde_json::Value>,
) -> Result<StepAttempt> {
    let attempt = sqlx::query_as::<_, StepAttempt>(
        r#"
        UPDATE step_attempts
        SET status = $2,
            error = $3,
            completed_at = NOW()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(error)
    .fetch_one(pool)
    .await?;

    debug!("Updated step attempt {}: status={}", id, status);

    Ok(attempt)
}

/// Get all attempts for a step run
pub async fn get_step_attempts(pool: &PgPool, step_run_id: i64) -> Result<Vec<StepAttempt>> {
    let attempts = sqlx::query_as::<_, StepAttempt>(
        "SELECT * FROM step_attempts WHERE step_run_id = $1 ORDER BY attempt_number ASC",
    )
    .bind(step_run_id)
    .fetch_all(pool)
    .await?;

    Ok(attempts)
}

/// Get workflows that are ready for retry
/// This includes workflows in PENDING state that have steps with next_retry_at in the past
pub async fn poll_workflows_ready_for_retry(pool: &PgPool) -> Result<Vec<Uuid>> {
    let now = Utc::now();

    let workflow_ids: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT sr.workflow_run_id
        FROM step_runs sr
        JOIN workflow_runs wr ON wr.id = sr.workflow_run_id
        WHERE sr.next_retry_at IS NOT NULL
          AND sr.next_retry_at <= $1
          AND wr.status = 'PENDING'
        "#,
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    Ok(workflow_ids.into_iter().map(|(id,)| id).collect())
}

/// Clear retry schedule for a step (when it succeeds or exhausts retries)
pub async fn clear_step_retry_schedule(
    pool: &PgPool,
    workflow_run_id: Uuid,
    step_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE step_runs
        SET next_retry_at = NULL
        WHERE workflow_run_id = $1 AND step_id = $2
        "#,
    )
    .bind(workflow_run_id)
    .bind(step_id)
    .execute(pool)
    .await?;

    debug!(
        "Cleared retry schedule: workflow={}, step={}",
        workflow_run_id, step_id
    );

    Ok(())
}

/// Get step run by internal ID (needed for step_attempts foreign key)
pub async fn get_step_run_by_id(pool: &PgPool, step_run_id: i64) -> Result<Option<StepRun>> {
    // Note: We need to add an 'id' column to step_runs table
    // For now, this is a placeholder that will need migration support
    let step = sqlx::query_as::<_, StepRun>(
        "SELECT * FROM step_runs WHERE workflow_run_id::text || step_id = $1",
    )
    .bind(step_run_id.to_string())
    .fetch_optional(pool)
    .await?;

    Ok(step)
}
