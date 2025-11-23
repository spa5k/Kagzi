//! Database queries for workflow and step management

use crate::error::{Error, Result};
use crate::models::*;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::{debug, warn};
use uuid::Uuid;

/// Create a new workflow run
pub async fn create_workflow_run(pool: &PgPool, create: CreateWorkflowRun) -> Result<WorkflowRun> {
    let workflow_version = create.workflow_version.unwrap_or(1);

    let run = sqlx::query_as::<_, WorkflowRun>(
        r#"
        INSERT INTO workflow_runs (workflow_name, workflow_version, input, status)
        VALUES ($1, $2, $3, 'PENDING')
        RETURNING *
        "#,
    )
    .bind(&create.workflow_name)
    .bind(workflow_version)
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
        INSERT INTO step_runs (workflow_run_id, step_id, input_hash, output, error, status, completed_at, parent_step_id, parallel_group_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (workflow_run_id, step_id)
        DO UPDATE SET
            output = EXCLUDED.output,
            error = EXCLUDED.error,
            status = EXCLUDED.status,
            completed_at = EXCLUDED.completed_at
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
    .bind(create.parent_step_id)
    .bind(create.parallel_group_id)
    .fetch_one(pool)
    .await?;

    debug!(
        "Created step run: workflow={}, step={}, parallel_group={:?}",
        create.workflow_run_id, create.step_id, create.parallel_group_id
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

/// Update worker lease heartbeat
pub async fn update_lease_heartbeat(pool: &PgPool, workflow_run_id: Uuid) -> Result<()> {
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

// ============================================================================
// Parallel execution queries
// ============================================================================

/// Get all steps in a parallel group
pub async fn get_parallel_group_steps(
    pool: &PgPool,
    parallel_group_id: Uuid,
) -> Result<Vec<StepRun>> {
    let steps = sqlx::query_as::<_, StepRun>(
        "SELECT * FROM step_runs WHERE parallel_group_id = $1 ORDER BY created_at ASC",
    )
    .bind(parallel_group_id)
    .fetch_all(pool)
    .await?;

    debug!(
        "Found {} steps in parallel group {}",
        steps.len(),
        parallel_group_id
    );

    Ok(steps)
}

/// Check if all steps in a parallel group have completed
pub async fn is_parallel_group_complete(pool: &PgPool, parallel_group_id: Uuid) -> Result<bool> {
    let incomplete_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM step_runs
        WHERE parallel_group_id = $1
          AND status != 'COMPLETED'
        "#,
    )
    .bind(parallel_group_id)
    .fetch_one(pool)
    .await?;

    Ok(incomplete_count == 0)
}

/// Get all failed steps in a parallel group
pub async fn get_parallel_group_failures(
    pool: &PgPool,
    parallel_group_id: Uuid,
) -> Result<Vec<StepRun>> {
    let failed_steps = sqlx::query_as::<_, StepRun>(
        "SELECT * FROM step_runs WHERE parallel_group_id = $1 AND status = 'FAILED'",
    )
    .bind(parallel_group_id)
    .fetch_all(pool)
    .await?;

    Ok(failed_steps)
}

/// Bulk check for cached steps
/// Returns a map of step_id -> cached StepRun for any steps that exist in cache
pub async fn bulk_check_step_cache(
    pool: &PgPool,
    workflow_run_id: Uuid,
    step_ids: &[String],
) -> Result<Vec<StepRun>> {
    if step_ids.is_empty() {
        return Ok(vec![]);
    }

    let steps = sqlx::query_as::<_, StepRun>(
        r#"
        SELECT * FROM step_runs
        WHERE workflow_run_id = $1
          AND step_id = ANY($2)
        "#,
    )
    .bind(workflow_run_id)
    .bind(step_ids)
    .fetch_all(pool)
    .await?;

    debug!(
        "Bulk cache check: workflow={}, requested={}, found={}",
        workflow_run_id,
        step_ids.len(),
        steps.len()
    );

    Ok(steps)
}

/// Get child steps of a parent step (for nested parallelism)
pub async fn get_child_steps(
    pool: &PgPool,
    workflow_run_id: Uuid,
    parent_step_id: &str,
) -> Result<Vec<StepRun>> {
    let steps = sqlx::query_as::<_, StepRun>(
        r#"
        SELECT * FROM step_runs
        WHERE workflow_run_id = $1
          AND parent_step_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(workflow_run_id)
    .bind(parent_step_id)
    .fetch_all(pool)
    .await?;

    Ok(steps)
}

// ============================================================================
// Workflow Version Management Queries
// ============================================================================

/// Register a new workflow version
pub async fn create_workflow_version(
    pool: &PgPool,
    workflow_name: &str,
    version: i32,
    is_default: bool,
    description: Option<String>,
) -> Result<WorkflowVersion> {
    let workflow_version = sqlx::query_as::<_, WorkflowVersion>(
        r#"
        INSERT INTO workflow_versions (workflow_name, version, is_default, description)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (workflow_name, version)
        DO UPDATE SET
            is_default = EXCLUDED.is_default,
            description = EXCLUDED.description
        RETURNING *
        "#,
    )
    .bind(workflow_name)
    .bind(version)
    .bind(is_default)
    .bind(description)
    .fetch_one(pool)
    .await?;

    debug!("Created/updated workflow version: {} v{}", workflow_name, version);
    Ok(workflow_version)
}

/// Get the default version for a workflow
pub async fn get_default_version(
    pool: &PgPool,
    workflow_name: &str,
) -> Result<Option<WorkflowVersion>> {
    let version = sqlx::query_as::<_, WorkflowVersion>(
        "SELECT * FROM workflow_versions WHERE workflow_name = $1 AND is_default = TRUE"
    )
    .bind(workflow_name)
    .fetch_optional(pool)
    .await?;

    Ok(version)
}

/// Set a version as the default for a workflow
/// This will unset any existing default
pub async fn set_default_version(
    pool: &PgPool,
    workflow_name: &str,
    version: i32,
) -> Result<WorkflowVersion> {
    // First, unset any existing default
    sqlx::query(
        "UPDATE workflow_versions SET is_default = FALSE WHERE workflow_name = $1 AND is_default = TRUE"
    )
    .bind(workflow_name)
    .execute(pool)
    .await?;

    // Then set the new default
    let workflow_version = sqlx::query_as::<_, WorkflowVersion>(
        r#"
        UPDATE workflow_versions
        SET is_default = TRUE
        WHERE workflow_name = $1 AND version = $2
        RETURNING *
        "#,
    )
    .bind(workflow_name)
    .bind(version)
    .fetch_one(pool)
    .await?;

    debug!("Set default version for {} to v{}", workflow_name, version);
    Ok(workflow_version)
}

/// Get all versions for a workflow
pub async fn get_workflow_versions(
    pool: &PgPool,
    workflow_name: &str,
) -> Result<Vec<WorkflowVersion>> {
    let versions = sqlx::query_as::<_, WorkflowVersion>(
        "SELECT * FROM workflow_versions WHERE workflow_name = $1 ORDER BY version ASC"
    )
    .bind(workflow_name)
    .fetch_all(pool)
    .await?;

    Ok(versions)
}

/// Get a specific workflow version
pub async fn get_workflow_version(
    pool: &PgPool,
    workflow_name: &str,
    version: i32,
) -> Result<Option<WorkflowVersion>> {
    let workflow_version = sqlx::query_as::<_, WorkflowVersion>(
        "SELECT * FROM workflow_versions WHERE workflow_name = $1 AND version = $2"
    )
    .bind(workflow_name)
    .bind(version)
    .fetch_optional(pool)
    .await?;

    Ok(workflow_version)
}

/// Deprecate a workflow version
pub async fn deprecate_workflow_version(
    pool: &PgPool,
    workflow_name: &str,
    version: i32,
) -> Result<WorkflowVersion> {
    let workflow_version = sqlx::query_as::<_, WorkflowVersion>(
        r#"
        UPDATE workflow_versions
        SET deprecated_at = NOW()
        WHERE workflow_name = $1 AND version = $2
        RETURNING *
        "#,
    )
    .bind(workflow_name)
    .bind(version)
    .fetch_one(pool)
    .await?;

    debug!("Deprecated workflow version: {} v{}", workflow_name, version);
    Ok(workflow_version)
}

/// Count workflow runs by name and version
pub async fn count_workflow_runs_by_version(
    pool: &PgPool,
    workflow_name: &str,
    version: i32,
    status: Option<WorkflowStatus>,
) -> Result<i64> {
    let count: (i64,) = if let Some(status) = status {
        sqlx::query_as(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_name = $1 AND workflow_version = $2 AND status = $3"
        )
        .bind(workflow_name)
        .bind(version)
        .bind(status)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_name = $1 AND workflow_version = $2"
        )
        .bind(workflow_name)
        .bind(version)
        .fetch_one(pool)
        .await?
    };

    Ok(count.0)
}

/// Get all workflow runs for a specific version
pub async fn get_workflow_runs_by_version(
    pool: &PgPool,
    workflow_name: &str,
    version: i32,
    limit: Option<i64>,
) -> Result<Vec<WorkflowRun>> {
    let limit = limit.unwrap_or(100);

    let runs = sqlx::query_as::<_, WorkflowRun>(
        r#"
        SELECT * FROM workflow_runs
        WHERE workflow_name = $1 AND workflow_version = $2
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(workflow_name)
    .bind(version)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(runs)
}

// ============================================================================
// Worker Management Queries
// ============================================================================

/// Register a new worker
pub async fn register_worker(
    pool: &PgPool,
    worker_name: &str,
    hostname: Option<String>,
    process_id: Option<i32>,
    config: Option<serde_json::Value>,
    metadata: Option<serde_json::Value>,
) -> Result<crate::models::Worker> {
    let worker = sqlx::query_as::<_, crate::models::Worker>(
        r#"
        INSERT INTO workers (worker_name, hostname, process_id, status, config, metadata)
        VALUES ($1, $2, $3, 'RUNNING', $4, $5)
        ON CONFLICT (worker_name)
        DO UPDATE SET
            hostname = EXCLUDED.hostname,
            process_id = EXCLUDED.process_id,
            started_at = NOW(),
            last_heartbeat = NOW(),
            status = 'RUNNING',
            config = EXCLUDED.config,
            metadata = EXCLUDED.metadata,
            stopped_at = NULL
        RETURNING *
        "#,
    )
    .bind(worker_name)
    .bind(hostname)
    .bind(process_id)
    .bind(config)
    .bind(metadata)
    .fetch_one(pool)
    .await?;

    debug!("Registered worker: {}", worker_name);
    Ok(worker)
}

/// Update worker heartbeat
pub async fn update_worker_heartbeat(pool: &PgPool, worker_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE workers SET last_heartbeat = NOW() WHERE id = $1")
        .bind(worker_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Update worker status
pub async fn update_worker_status(
    pool: &PgPool,
    worker_id: Uuid,
    status: crate::models::WorkerStatus,
) -> Result<crate::models::Worker> {
    let stopped_at = if matches!(status, crate::models::WorkerStatus::Stopped) {
        Some("NOW()")
    } else {
        None
    };

    let query = if let Some(_) = stopped_at {
        r#"
        UPDATE workers
        SET status = $2, stopped_at = NOW()
        WHERE id = $1
        RETURNING *
        "#
    } else {
        r#"
        UPDATE workers
        SET status = $2
        WHERE id = $1
        RETURNING *
        "#
    };

    let worker = sqlx::query_as::<_, crate::models::Worker>(query)
        .bind(worker_id)
        .bind(status)
        .fetch_one(pool)
        .await?;

    debug!("Updated worker {} status to {}", worker_id, worker.status);
    Ok(worker)
}

/// Get a worker by ID
pub async fn get_worker(pool: &PgPool, worker_id: Uuid) -> Result<Option<crate::models::Worker>> {
    let worker = sqlx::query_as::<_, crate::models::Worker>(
        "SELECT * FROM workers WHERE id = $1"
    )
    .bind(worker_id)
    .fetch_optional(pool)
    .await?;

    Ok(worker)
}

/// Get a worker by name
pub async fn get_worker_by_name(pool: &PgPool, worker_name: &str) -> Result<Option<crate::models::Worker>> {
    let worker = sqlx::query_as::<_, crate::models::Worker>(
        "SELECT * FROM workers WHERE worker_name = $1"
    )
    .bind(worker_name)
    .fetch_optional(pool)
    .await?;

    Ok(worker)
}

/// Get all active workers
pub async fn get_active_workers(pool: &PgPool) -> Result<Vec<crate::models::Worker>> {
    let workers = sqlx::query_as::<_, crate::models::Worker>(
        "SELECT * FROM workers WHERE status = 'RUNNING' ORDER BY started_at DESC"
    )
    .fetch_all(pool)
    .await?;

    Ok(workers)
}

/// Cleanup stale workers (no heartbeat for N seconds)
pub async fn cleanup_stale_workers(pool: &PgPool, stale_threshold_secs: i64) -> Result<i64> {
    let result = sqlx::query(
        r#"
        UPDATE workers
        SET status = 'STOPPED', stopped_at = NOW()
        WHERE status != 'STOPPED'
          AND last_heartbeat < NOW() - INTERVAL '1 second' * $1
        "#,
    )
    .bind(stale_threshold_secs)
    .execute(pool)
    .await?;

    let rows_affected = result.rows_affected() as i64;
    if rows_affected > 0 {
        warn!("Cleaned up {} stale workers", rows_affected);
    }

    Ok(rows_affected)
}

/// Record a worker event
pub async fn record_worker_event(
    pool: &PgPool,
    worker_id: Uuid,
    event_type: crate::models::WorkerEventType,
    event_data: Option<serde_json::Value>,
) -> Result<crate::models::WorkerEvent> {
    let event = sqlx::query_as::<_, crate::models::WorkerEvent>(
        r#"
        INSERT INTO worker_events (worker_id, event_type, event_data)
        VALUES ($1, $2, $3)
        RETURNING *
        "#,
    )
    .bind(worker_id)
    .bind(event_type)
    .bind(event_data)
    .fetch_one(pool)
    .await?;

    Ok(event)
}

/// Get worker events
pub async fn get_worker_events(
    pool: &PgPool,
    worker_id: Uuid,
    limit: Option<i64>,
) -> Result<Vec<crate::models::WorkerEvent>> {
    let limit = limit.unwrap_or(100);

    let events = sqlx::query_as::<_, crate::models::WorkerEvent>(
        "SELECT * FROM worker_events WHERE worker_id = $1 ORDER BY created_at DESC LIMIT $2"
    )
    .bind(worker_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(events)
}

/// Update worker lease with worker name
pub async fn update_worker_lease_name(
    pool: &PgPool,
    workflow_run_id: Uuid,
    worker_name: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE worker_leases SET worker_name = $2 WHERE workflow_run_id = $1"
    )
    .bind(workflow_run_id)
    .bind(worker_name)
    .execute(pool)
    .await?;

    Ok(())
}
