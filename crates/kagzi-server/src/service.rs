use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CancelWorkflowRunRequest, CompleteStepRequest,
    CompleteWorkflowRequest, Empty, FailStepRequest, FailWorkflowRequest, GetStepAttemptRequest,
    GetStepAttemptResponse, GetWorkflowRunRequest, GetWorkflowRunResponse, ListStepAttemptsRequest,
    ListStepAttemptsResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
    PollActivityRequest, PollActivityResponse, RecordHeartbeatRequest, ScheduleSleepRequest,
    StartWorkflowRequest, StartWorkflowResponse, StepAttempt, StepAttemptStatus, StepKind,
    WorkflowRun, WorkflowStatus,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::info;

/// Database row for workflow_runs table
#[derive(sqlx::FromRow)]
struct WorkflowRunRow {
    run_id: uuid::Uuid,
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
}

impl WorkflowRunRow {
    fn into_proto(self) -> WorkflowRun {
        let status = match self.status.as_str() {
            "PENDING" => WorkflowStatus::Pending,
            "RUNNING" => WorkflowStatus::Running,
            "SLEEPING" => WorkflowStatus::Sleeping,
            "COMPLETED" => WorkflowStatus::Completed,
            "FAILED" => WorkflowStatus::Failed,
            "CANCELLED" => WorkflowStatus::Cancelled,
            _ => WorkflowStatus::Unspecified,
        };

        WorkflowRun {
            run_id: self.run_id.to_string(),
            business_id: self.business_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            status: status.into(),
            input: serde_json::to_vec(&self.input).unwrap_or_default(),
            output: self
                .output
                .map(|o| serde_json::to_vec(&o).unwrap_or_default())
                .unwrap_or_default(),
            error: self.error.unwrap_or_default(),
            attempts: self.attempts,
            created_at: self.created_at.map(|t| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: t.timestamp_subsec_nanos() as i32,
            }),
            started_at: self.started_at.map(|t| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: t.timestamp_subsec_nanos() as i32,
            }),
            finished_at: self.finished_at.map(|t| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: t.timestamp_subsec_nanos() as i32,
            }),
            wake_up_at: self.wake_up_at.map(|t| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: t.timestamp_subsec_nanos() as i32,
            }),
            namespace_id: self.namespace_id,
            context: self
                .context
                .map(|c| serde_json::to_vec(&c).unwrap_or_default())
                .unwrap_or_default(),
            deadline_at: self.deadline_at.map(|t| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: t.timestamp_subsec_nanos() as i32,
            }),
            worker_id: self.locked_by.unwrap_or_default(),
            version: self.version.unwrap_or_default(),
            parent_step_attempt_id: self.parent_step_attempt_id.unwrap_or_default(),
        }
    }
}

/// Map database step status string to StepAttemptStatus enum
fn map_step_status(status: &str) -> StepAttemptStatus {
    match status {
        "PENDING" => StepAttemptStatus::Pending,
        "RUNNING" => StepAttemptStatus::Running,
        "COMPLETED" => StepAttemptStatus::Completed,
        "FAILED" => StepAttemptStatus::Failed,
        _ => StepAttemptStatus::Unspecified,
    }
}

pub struct MyWorkflowService {
    pub pool: PgPool,
}

#[tonic::async_trait]
impl WorkflowService for MyWorkflowService {
    async fn start_workflow(
        &self,
        request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        let req = request.into_inner();
        info!("StartWorkflow request: {:?}", req);

        let input_json: serde_json::Value = if req.input.is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_slice(&req.input)
                .map_err(|e| Status::invalid_argument(format!("Input must be valid JSON: {}", e)))?
        };

        let context_json: serde_json::Value = if req.context.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&req.context).map_err(|e| {
                Status::invalid_argument(format!("Context must be valid JSON: {}", e))
            })?
        };

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        // Check for idempotency
        if !req.idempotency_key.is_empty() {
            let existing = sqlx::query!(
                r#"
                SELECT run_id FROM kagzi.workflow_runs
                WHERE namespace_id = $1 AND idempotency_key = $2
                "#,
                namespace_id,
                req.idempotency_key
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check idempotency: {:?}", e);
                Status::internal("Failed to check idempotency")
            })?;

            if let Some(row) = existing {
                return Ok(Response::new(StartWorkflowResponse {
                    run_id: row.run_id.to_string(),
                }));
            }
        }

        let run_id = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                business_id, task_queue, workflow_type, status, input,
                namespace_id, idempotency_key, context, deadline_at, version
            )
            VALUES ($1, $2, $3, 'PENDING', $4, $5, $6, $7, $8, $9)
            RETURNING run_id
            "#,
            req.workflow_id,
            req.task_queue,
            req.workflow_type,
            input_json,
            namespace_id,
            if req.idempotency_key.is_empty() {
                None
            } else {
                Some(req.idempotency_key)
            },
            context_json,
            req.deadline_at
                .map(|ts| std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::new(ts.seconds as u64, ts.nanos as u32))
                .map(chrono::DateTime::<chrono::Utc>::from),
            req.version
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to insert workflow run: {:?}", e);
            Status::internal("Failed to start workflow")
        })?
        .run_id;

        Ok(Response::new(StartWorkflowResponse {
            run_id: run_id.to_string(),
        }))
    }

    async fn get_workflow_run(
        &self,
        request: Request<GetWorkflowRunRequest>,
    ) -> Result<Response<GetWorkflowRunResponse>, Status> {
        let req = request.into_inner();
        info!("GetWorkflowRun request: {:?}", req);

        // Validate UUID format
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        // Query the workflow_runs table
        let row = sqlx::query!(
            r#"
            SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                   input, output, context, locked_by, attempts, error,
                   created_at, started_at, finished_at, wake_up_at, deadline_at,
                   version, parent_step_attempt_id
            FROM kagzi.workflow_runs
            WHERE run_id = $1 AND namespace_id = $2
            "#,
            run_id,
            namespace_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get workflow run: {:?}", e);
            Status::internal("Failed to get workflow run")
        })?;

        match row {
            Some(r) => {
                let status = match r.status.as_str() {
                    "PENDING" => WorkflowStatus::Pending,
                    "RUNNING" => WorkflowStatus::Running,
                    "SLEEPING" => WorkflowStatus::Sleeping,
                    "COMPLETED" => WorkflowStatus::Completed,
                    "FAILED" => WorkflowStatus::Failed,
                    "CANCELLED" => WorkflowStatus::Cancelled,
                    _ => WorkflowStatus::Unspecified,
                };

                let workflow_run = WorkflowRun {
                    run_id: r.run_id.to_string(),
                    business_id: r.business_id,
                    task_queue: r.task_queue,
                    workflow_type: r.workflow_type,
                    status: status.into(),
                    input: serde_json::to_vec(&r.input).unwrap_or_default(),
                    output: r
                        .output
                        .map(|o| serde_json::to_vec(&o).unwrap_or_default())
                        .unwrap_or_default(),
                    error: r.error.unwrap_or_default(),
                    attempts: r.attempts,
                    created_at: r.created_at.map(|t| prost_types::Timestamp {
                        seconds: t.timestamp(),
                        nanos: t.timestamp_subsec_nanos() as i32,
                    }),
                    started_at: r.started_at.map(|t| prost_types::Timestamp {
                        seconds: t.timestamp(),
                        nanos: t.timestamp_subsec_nanos() as i32,
                    }),
                    finished_at: r.finished_at.map(|t| prost_types::Timestamp {
                        seconds: t.timestamp(),
                        nanos: t.timestamp_subsec_nanos() as i32,
                    }),
                    wake_up_at: r.wake_up_at.map(|t| prost_types::Timestamp {
                        seconds: t.timestamp(),
                        nanos: t.timestamp_subsec_nanos() as i32,
                    }),
                    namespace_id: r.namespace_id,
                    context: r
                        .context
                        .map(|c| serde_json::to_vec(&c).unwrap_or_default())
                        .unwrap_or_default(),
                    deadline_at: r.deadline_at.map(|t| prost_types::Timestamp {
                        seconds: t.timestamp(),
                        nanos: t.timestamp_subsec_nanos() as i32,
                    }),
                    worker_id: r.locked_by.unwrap_or_default(),
                    version: r.version.unwrap_or_default(),
                    parent_step_attempt_id: r.parent_step_attempt_id.unwrap_or_default(),
                };

                Ok(Response::new(GetWorkflowRunResponse {
                    workflow_run: Some(workflow_run),
                }))
            }
            None => Err(Status::not_found(format!(
                "Workflow run not found: run_id={}, namespace_id={}",
                run_id, namespace_id
            ))),
        }
    }

    async fn list_workflow_runs(
        &self,
        request: Request<ListWorkflowRunsRequest>,
    ) -> Result<Response<ListWorkflowRunsResponse>, Status> {
        use base64::Engine;

        let req = request.into_inner();
        info!("ListWorkflowRuns request: {:?}", req);

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        // Validate and set page size (default: 20, max: 100)
        let page_size = if req.page_size <= 0 {
            20
        } else if req.page_size > 100 {
            100
        } else {
            req.page_size
        };

        // Parse cursor from page_token (format: "created_at_millis:run_id")
        let cursor: Option<(chrono::DateTime<chrono::Utc>, uuid::Uuid)> =
            if req.page_token.is_empty() {
                None
            } else {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&req.page_token)
                    .map_err(|_| Status::invalid_argument("Invalid page_token"))?;
                let cursor_str = String::from_utf8(decoded)
                    .map_err(|_| Status::invalid_argument("Invalid page_token encoding"))?;
                let parts: Vec<&str> = cursor_str.splitn(2, ':').collect();
                if parts.len() != 2 {
                    return Err(Status::invalid_argument("Invalid page_token format"));
                }
                let millis: i64 = parts[0]
                    .parse()
                    .map_err(|_| Status::invalid_argument("Invalid page_token timestamp"))?;
                let run_id = uuid::Uuid::parse_str(parts[1])
                    .map_err(|_| Status::invalid_argument("Invalid page_token run_id"))?;
                let cursor_time = chrono::DateTime::from_timestamp_millis(millis)
                    .ok_or_else(|| Status::invalid_argument("Invalid cursor timestamp"))?;
                Some((cursor_time, run_id))
            };

        // Build query dynamically
        let limit = (page_size + 1) as i64;
        let rows: Vec<WorkflowRunRow> = match (&cursor, req.filter_status.is_empty()) {
            (None, true) => {
                // No cursor, no status filter
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1
                    ORDER BY created_at DESC, run_id DESC
                    LIMIT $2
                    "#,
                )
                .bind(&namespace_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (None, false) => {
                // No cursor, with status filter
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1 AND status = $2
                    ORDER BY created_at DESC, run_id DESC
                    LIMIT $3
                    "#,
                )
                .bind(&namespace_id)
                .bind(&req.filter_status)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (Some((cursor_time, cursor_run_id)), true) => {
                // With cursor, no status filter
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1 AND (created_at, run_id) < ($2, $3)
                    ORDER BY created_at DESC, run_id DESC
                    LIMIT $4
                    "#,
                )
                .bind(&namespace_id)
                .bind(cursor_time)
                .bind(cursor_run_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (Some((cursor_time, cursor_run_id)), false) => {
                // With cursor and status filter
                sqlx::query_as::<_, WorkflowRunRow>(
                    r#"
                    SELECT run_id, namespace_id, business_id, task_queue, workflow_type, status,
                           input, output, context, locked_by, attempts, error,
                           created_at, started_at, finished_at, wake_up_at, deadline_at,
                           version, parent_step_attempt_id
                    FROM kagzi.workflow_runs
                    WHERE namespace_id = $1 AND status = $2 AND (created_at, run_id) < ($3, $4)
                    ORDER BY created_at DESC, run_id DESC
                    LIMIT $5
                    "#,
                )
                .bind(&namespace_id)
                .bind(&req.filter_status)
                .bind(cursor_time)
                .bind(cursor_run_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| {
            tracing::error!("Failed to list workflow runs: {:?}", e);
            Status::internal("Failed to list workflow runs")
        })?;

        // Determine if there are more results
        let has_more = rows.len() > page_size as usize;
        let result_rows: Vec<_> = rows.into_iter().take(page_size as usize).collect();

        // Generate next page token from last result
        let next_page_token = if has_more {
            if let Some(last) = result_rows.last() {
                if let Some(created_at) = last.created_at {
                    let cursor_str = format!("{}:{}", created_at.timestamp_millis(), last.run_id);
                    base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Map database rows to WorkflowRun messages
        let workflow_runs: Vec<WorkflowRun> =
            result_rows.into_iter().map(|r| r.into_proto()).collect();

        Ok(Response::new(ListWorkflowRunsResponse {
            workflow_runs,
            next_page_token,
            prev_page_token: String::new(), // Backward pagination not implemented yet
            has_more,
        }))
    }

    async fn cancel_workflow_run(
        &self,
        request: Request<CancelWorkflowRunRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("CancelWorkflowRun request: {:?}", req);

        // Validate UUID format
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        // Check current status and update to CANCELLED if allowed
        // Only PENDING, RUNNING, or SLEEPING workflows can be cancelled
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
        .await
        .map_err(|e| {
            tracing::error!("Failed to cancel workflow run: {:?}", e);
            Status::internal("Failed to cancel workflow run")
        })?;

        match result {
            Some(_) => {
                info!("Workflow {} cancelled successfully", run_id);
                Ok(Response::new(Empty {}))
            }
            None => {
                // Check if workflow exists to provide better error message
                let exists = sqlx::query!(
                    r#"
                    SELECT status FROM kagzi.workflow_runs
                    WHERE run_id = $1 AND namespace_id = $2
                    "#,
                    run_id,
                    namespace_id
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to check workflow existence: {:?}", e);
                    Status::internal("Failed to check workflow existence")
                })?;

                match exists {
                    Some(row) => Err(Status::failed_precondition(format!(
                        "Cannot cancel workflow with status '{}'. Only PENDING, RUNNING, or SLEEPING workflows can be cancelled.",
                        row.status
                    ))),
                    None => Err(Status::not_found(format!(
                        "Workflow run not found: run_id={}, namespace_id={}",
                        run_id, namespace_id
                    ))),
                }
            }
        }
    }

    async fn poll_activity(
        &self,
        request: Request<PollActivityRequest>,
    ) -> Result<Response<PollActivityResponse>, Status> {
        let req = request.into_inner();
        info!("PollActivity request: {:?}", req);

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(60); // 60s long poll

        loop {
            // Try to claim a task
            let work = sqlx::query!(
                r#"
                UPDATE kagzi.workflow_runs
                SET status = 'RUNNING',
                    locked_by = $1,
                    locked_until = NOW() + INTERVAL '30 seconds',
                    started_at = COALESCE(started_at, NOW())
                WHERE run_id = (
                    SELECT run_id
                    FROM kagzi.workflow_runs
                    WHERE task_queue = $2
                      AND namespace_id = $3
                      AND status = 'PENDING'
                      AND (wake_up_at IS NULL OR wake_up_at <= NOW())
                    ORDER BY created_at ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                )
                RETURNING run_id, workflow_type, input
                "#,
                req.worker_id,
                req.task_queue,
                if req.namespace_id.is_empty() {
                    "default".to_string()
                } else {
                    req.namespace_id.clone()
                }
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                tracing::error!("Failed to poll activity: {:?}", e);
                Status::internal("Failed to poll activity")
            })?;

            if let Some(w) = work {
                info!("Worker {} claimed workflow {}", req.worker_id, w.run_id);
                // Convert JSONB input back to bytes
                let input_bytes = serde_json::to_vec(&w.input).unwrap_or_default();

                return Ok(Response::new(PollActivityResponse {
                    run_id: w.run_id.to_string(),
                    workflow_type: w.workflow_type,
                    workflow_input: input_bytes,
                }));
            }

            if start.elapsed() > timeout {
                return Err(Status::deadline_exceeded("No work available"));
            }

            // Wait a bit before retrying
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    async fn record_heartbeat(
        &self,
        request: Request<RecordHeartbeatRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("RecordHeartbeat request: {:?}", req);

        // Validate UUID format
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        if req.worker_id.is_empty() {
            return Err(Status::invalid_argument("worker_id is required"));
        }

        // Extend lock only if worker still owns it
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
            req.worker_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to record heartbeat: {:?}", e);
            Status::internal("Failed to record heartbeat")
        })?;

        match result {
            Some(_) => {
                info!(
                    "Heartbeat recorded for workflow {} by worker {}",
                    run_id, req.worker_id
                );
                Ok(Response::new(Empty {}))
            }
            None => {
                // Check why the update failed
                let workflow = sqlx::query!(
                    r#"
                    SELECT status, locked_by FROM kagzi.workflow_runs
                    WHERE run_id = $1
                    "#,
                    run_id
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to check workflow: {:?}", e);
                    Status::internal("Failed to check workflow")
                })?;

                match workflow {
                    Some(row) => {
                        if row.status != "RUNNING" {
                            Err(Status::failed_precondition(format!(
                                "Workflow is not running (status: {})",
                                row.status
                            )))
                        } else if row.locked_by.as_deref() != Some(&req.worker_id) {
                            Err(Status::failed_precondition(format!(
                                "Lock stolen by another worker: {:?}",
                                row.locked_by
                            )))
                        } else {
                            Err(Status::internal("Unexpected heartbeat failure"))
                        }
                    }
                    None => Err(Status::not_found(format!(
                        "Workflow run not found: {}",
                        run_id
                    ))),
                }
            }
        }
    }

    async fn get_step_attempt(
        &self,
        request: Request<GetStepAttemptRequest>,
    ) -> Result<Response<GetStepAttemptResponse>, Status> {
        let req = request.into_inner();
        let attempt_id = uuid::Uuid::parse_str(&req.step_attempt_id)
            .map_err(|_| Status::invalid_argument("Invalid step_attempt_id"))?;

        let row = sqlx::query!(
            r#"
            SELECT attempt_id, run_id as workflow_run_id, step_id, attempt_number, status,
                   input, output, error, child_workflow_run_id,
                   started_at, finished_at, created_at, namespace_id
            FROM kagzi.step_runs
            WHERE attempt_id = $1
            "#,
            attempt_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get step attempt: {:?}", e);
            Status::internal("Failed to get step attempt")
        })?;

        if let Some(r) = row {
            // Determine step kind from step_id patterns or other logic
            let kind = if r.step_id.contains("sleep") || r.step_id.contains("wait") {
                StepKind::Sleep
            } else if r.step_id.contains("function") || r.step_id.contains("task") {
                StepKind::Function
            } else {
                StepKind::Unspecified
            };

            let attempt = StepAttempt {
                step_attempt_id: r.attempt_id.to_string(),
                workflow_run_id: r.workflow_run_id.to_string(),
                step_id: r.step_id,
                kind: kind.into(),
                status: map_step_status(&r.status).into(),
                config: vec![],  // Config not stored in DB yet
                context: vec![], // Context not stored in DB yet
                output: serde_json::to_vec(&r.output).unwrap_or_default(),
                error: r.error.map(|e| e.into_bytes()).unwrap_or_default(),
                started_at: r.started_at.map(|t| prost_types::Timestamp {
                    seconds: t.timestamp(),
                    nanos: t.timestamp_subsec_nanos() as i32,
                }),
                finished_at: r.finished_at.map(|t| prost_types::Timestamp {
                    seconds: t.timestamp(),
                    nanos: t.timestamp_subsec_nanos() as i32,
                }),
                created_at: r.created_at.map(|t| prost_types::Timestamp {
                    seconds: t.timestamp(),
                    nanos: t.timestamp_subsec_nanos() as i32,
                }),
                updated_at: r
                    .finished_at
                    .or(r.created_at)
                    .map(|t| prost_types::Timestamp {
                        seconds: t.timestamp(),
                        nanos: t.timestamp_subsec_nanos() as i32,
                    }),
                child_workflow_run_id: r
                    .child_workflow_run_id
                    .map(|u| u.to_string())
                    .unwrap_or_default(),
                namespace_id: r.namespace_id,
            };
            Ok(Response::new(GetStepAttemptResponse {
                step_attempt: Some(attempt),
            }))
        } else {
            Err(Status::not_found("Step attempt not found"))
        }
    }

    async fn list_step_attempts(
        &self,
        request: Request<ListStepAttemptsRequest>,
    ) -> Result<Response<ListStepAttemptsResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.workflow_run_id)
            .map_err(|_| Status::invalid_argument("Invalid workflow_run_id"))?;

        // Default page size
        let page_size = if req.page_size <= 0 {
            50
        } else if req.page_size > 100 {
            100
        } else {
            req.page_size
        };

        // Define a struct to hold the query result
        #[derive(sqlx::FromRow)]
        struct StepRunRow {
            attempt_id: uuid::Uuid,
            workflow_run_id: uuid::Uuid,
            step_id: String,
            #[allow(dead_code)]
            attempt_number: i32,
            status: String,
            #[allow(dead_code)]
            input: Option<serde_json::Value>,
            output: Option<serde_json::Value>,
            error: Option<String>,
            child_workflow_run_id: Option<uuid::Uuid>,
            started_at: Option<chrono::DateTime<chrono::Utc>>,
            finished_at: Option<chrono::DateTime<chrono::Utc>>,
            created_at: Option<chrono::DateTime<chrono::Utc>>,
            namespace_id: String,
        }

        // Query with optional step_id filter
        let rows: Vec<StepRunRow> = if req.step_id.is_empty() {
            sqlx::query_as(
                r#"
                SELECT attempt_id, run_id as workflow_run_id, step_id, attempt_number, status,
                       input, output, error, child_workflow_run_id,
                       started_at, finished_at, created_at, namespace_id
                FROM kagzi.step_runs
                WHERE run_id = $1
                ORDER BY created_at ASC, attempt_number ASC
                LIMIT $2
                "#,
            )
            .bind(run_id)
            .bind(page_size as i64)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as(
                r#"
                SELECT attempt_id, run_id as workflow_run_id, step_id, attempt_number, status,
                       input, output, error, child_workflow_run_id,
                       started_at, finished_at, created_at, namespace_id
                FROM kagzi.step_runs
                WHERE run_id = $1 AND step_id = $2
                ORDER BY attempt_number ASC
                LIMIT $3
                "#,
            )
            .bind(run_id)
            .bind(&req.step_id)
            .bind(page_size as i64)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| {
            tracing::error!("Failed to list step attempts: {:?}", e);
            Status::internal("Failed to list step attempts")
        })?;

        let attempts =
            rows.into_iter()
                .map(|r| {
                    // Determine step kind from step_id patterns or other logic
                    let kind = if r.step_id.contains("sleep") || r.step_id.contains("wait") {
                        StepKind::Sleep
                    } else if r.step_id.contains("function") || r.step_id.contains("task") {
                        StepKind::Function
                    } else {
                        StepKind::Unspecified
                    };

                    StepAttempt {
                        step_attempt_id: r.attempt_id.to_string(),
                        workflow_run_id: r.workflow_run_id.to_string(),
                        step_id: r.step_id,
                        kind: kind.into(),
                        status: map_step_status(&r.status).into(),
                        config: vec![],  // Config not stored in DB yet
                        context: vec![], // Context not stored in DB yet
                        output: serde_json::to_vec(&r.output).unwrap_or_default(),
                        error: r.error.map(|e| e.into_bytes()).unwrap_or_default(),
                        started_at: r.started_at.map(|t| prost_types::Timestamp {
                            seconds: t.timestamp(),
                            nanos: t.timestamp_subsec_nanos() as i32,
                        }),
                        finished_at: r.finished_at.map(|t| prost_types::Timestamp {
                            seconds: t.timestamp(),
                            nanos: t.timestamp_subsec_nanos() as i32,
                        }),
                        created_at: r.created_at.map(|t| prost_types::Timestamp {
                            seconds: t.timestamp(),
                            nanos: t.timestamp_subsec_nanos() as i32,
                        }),
                        updated_at: r.finished_at.or(r.created_at).map(|t| {
                            prost_types::Timestamp {
                                seconds: t.timestamp(),
                                nanos: t.timestamp_subsec_nanos() as i32,
                            }
                        }),
                        child_workflow_run_id: r
                            .child_workflow_run_id
                            .map(|u| u.to_string())
                            .unwrap_or_default(),
                        namespace_id: r.namespace_id,
                    }
                })
                .collect();

        Ok(Response::new(ListStepAttemptsResponse {
            step_attempts: attempts,
            next_page_token: String::new(), // Pagination not implemented yet
        }))
    }

    async fn begin_step(
        &self,
        request: Request<BeginStepRequest>,
    ) -> Result<Response<BeginStepResponse>, Status> {
        let req = request.into_inner();
        info!("BeginStep request: {:?}", req);

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let step = sqlx::query!(
            r#"
            SELECT status, output
            FROM kagzi.step_runs
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true
            "#,
            run_id,
            req.step_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to check step status: {:?}", e);
            Status::internal("Failed to check step status")
        })?;

        if let Some(s) = step
            && s.status == "COMPLETED"
        {
            let cached_bytes = serde_json::to_vec(&s.output).unwrap_or_default();
            return Ok(Response::new(BeginStepResponse {
                should_execute: false,
                cached_result: cached_bytes,
            }));
        }

        Ok(Response::new(BeginStepResponse {
            should_execute: true,
            cached_result: vec![],
        }))
    }

    async fn complete_step(
        &self,
        request: Request<CompleteStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("CompleteStep request: {:?}", req);

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let output_json: serde_json::Value = serde_json::from_slice(&req.output)
            .map_err(|e| Status::invalid_argument(format!("Output must be valid JSON: {}", e)))?;

        // Mark previous attempts as not latest
        sqlx::query!(
            "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
            run_id,
            req.step_id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update previous attempts: {:?}", e);
            Status::internal("Failed to update previous attempts")
        })?;

        // Then insert new attempt
        sqlx::query!(
            r#"
            INSERT INTO kagzi.step_runs (run_id, step_id, status, output, finished_at, is_latest, attempt_number)
            VALUES ($1, $2, 'COMPLETED', $3, NOW(), true, 
                    COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2), 0) + 1)
            "#,
            run_id,
            req.step_id,
            output_json
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to complete step: {:?}", e);
            Status::internal("Failed to complete step")
        })?;

        Ok(Response::new(Empty {}))
    }

    async fn fail_step(
        &self,
        request: Request<FailStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("FailStep request: {:?}", req);

        // Validate UUID format
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        if req.step_id.is_empty() {
            return Err(Status::invalid_argument("step_id is required"));
        }

        // Mark previous attempts as not latest
        sqlx::query!(
            "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
            run_id,
            req.step_id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update previous attempts: {:?}", e);
            Status::internal("Failed to update previous attempts")
        })?;

        // Insert new failed attempt
        sqlx::query!(
            r#"
            INSERT INTO kagzi.step_runs (run_id, step_id, status, error, finished_at, is_latest, attempt_number, namespace_id)
            VALUES ($1, $2, 'FAILED', $3, NOW(), true, 
                    COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2), 0) + 1,
                    COALESCE((SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $1), 'default'))
            "#,
            run_id,
            req.step_id,
            req.error
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to record step failure: {:?}", e);
            Status::internal("Failed to record step failure")
        })?;

        info!(
            "Step {} failed for workflow {}: {}",
            req.step_id, run_id, req.error
        );
        Ok(Response::new(Empty {}))
    }

    async fn complete_workflow(
        &self,
        request: Request<CompleteWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("CompleteWorkflow request: {:?}", req);

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let output_json: serde_json::Value = serde_json::from_slice(&req.output)
            .map_err(|e| Status::invalid_argument(format!("Output must be valid JSON: {}", e)))?;

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
            output_json
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to complete workflow: {:?}", e);
            Status::internal("Failed to complete workflow")
        })?;

        Ok(Response::new(Empty {}))
    }

    async fn fail_workflow(
        &self,
        request: Request<FailWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("FailWorkflow request: {:?}", req);

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

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
            req.error
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fail workflow: {:?}", e);
            Status::internal("Failed to fail workflow")
        })?;

        Ok(Response::new(Empty {}))
    }

    async fn schedule_sleep(
        &self,
        request: Request<ScheduleSleepRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        info!("ScheduleSleep request: {:?}", req);

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let duration = req.duration_seconds as f64;

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
        .await
        .map_err(|e| {
            tracing::error!("Failed to schedule sleep: {:?}", e);
            Status::internal("Failed to schedule sleep")
        })?;

        Ok(Response::new(Empty {}))
    }
}
