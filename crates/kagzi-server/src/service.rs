use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CancelWorkflowRunRequest, CompleteStepRequest,
    CompleteWorkflowRequest, Empty, FailStepRequest, FailWorkflowRequest, GetStepAttemptRequest,
    GetStepAttemptResponse, GetWorkflowRunRequest, GetWorkflowRunResponse, ListStepAttemptsRequest,
    ListStepAttemptsResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
    PollActivityRequest, PollActivityResponse, RecordHeartbeatRequest, ScheduleSleepRequest,
    StartWorkflowRequest, StartWorkflowResponse, StepAttempt,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::info;

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
        info!("GetWorkflowRun request: {:?}", request);
        Err(Status::unimplemented("Not implemented yet"))
    }

    async fn list_workflow_runs(
        &self,
        request: Request<ListWorkflowRunsRequest>,
    ) -> Result<Response<ListWorkflowRunsResponse>, Status> {
        info!("ListWorkflowRuns request: {:?}", request);
        Ok(Response::new(ListWorkflowRunsResponse {
            workflow_runs: vec![],
            next_page_token: "".to_string(),
            prev_page_token: "".to_string(),
            has_more: false,
        }))
    }

    async fn cancel_workflow_run(
        &self,
        request: Request<CancelWorkflowRunRequest>,
    ) -> Result<Response<Empty>, Status> {
        info!("CancelWorkflowRun request: {:?}", request);
        Ok(Response::new(Empty {}))
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
        info!("RecordHeartbeat request: {:?}", request);
        Ok(Response::new(Empty {}))
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
            let attempt = StepAttempt {
                step_attempt_id: r.attempt_id.to_string(),
                workflow_run_id: r.workflow_run_id.to_string(),
                step_id: r.step_id,
                kind: 0,   // TODO: Store kind
                status: 0, // TODO: Map status string to enum
                config: vec![], // TODO: Store config if needed
                context: vec![], // TODO: Store context if needed
                output: serde_json::to_vec(&r.output).unwrap_or_default(),
                error: serde_json::to_vec(&r.error).unwrap_or_default(),
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
                updated_at: r.created_at.map(|t| prost_types::Timestamp { // Use created_at as updated_at for now
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

        let rows = sqlx::query!(
            r#"
            SELECT attempt_id, run_id as workflow_run_id, step_id, attempt_number, status,
                   input, output, error, child_workflow_run_id,
                   started_at, finished_at, created_at, namespace_id
            FROM kagzi.step_runs
            WHERE run_id = $1
            ORDER BY attempt_number ASC
            LIMIT $2
            "#,
            run_id,
            req.page_size as i64
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list step attempts: {:?}", e);
            Status::internal("Failed to list step attempts")
        })?;

        let attempts = rows
            .into_iter()
            .map(|r| StepAttempt {
                step_attempt_id: r.attempt_id.to_string(),
                workflow_run_id: r.workflow_run_id.to_string(),
                step_id: r.step_id,
                kind: 0,
                status: 0,
                config: vec![], // TODO: Store config if needed
                context: vec![], // TODO: Store context if needed
                output: serde_json::to_vec(&r.output).unwrap_or_default(),
                error: serde_json::to_vec(&r.error).unwrap_or_default(),
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
                updated_at: r.created_at.map(|t| prost_types::Timestamp { // Use created_at as updated_at for now
                    seconds: t.timestamp(),
                    nanos: t.timestamp_subsec_nanos() as i32,
                }),
                child_workflow_run_id: r
                    .child_workflow_run_id
                    .map(|u| u.to_string())
                    .unwrap_or_default(),
                namespace_id: r.namespace_id,
            })
            .collect();

        Ok(Response::new(ListStepAttemptsResponse {
            step_attempts: attempts,
            next_page_token: "".to_string(),
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

        if let Some(s) = step {
            if s.status == "COMPLETED" {
                let cached_bytes = serde_json::to_vec(&s.output).unwrap_or_default();
                return Ok(Response::new(BeginStepResponse {
                    should_execute: false,
                    cached_result: cached_bytes,
                }));
            }
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

        // For new schema, we need to create a new attempt and mark previous as not latest
        // First, mark previous attempts as not latest
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
        info!("FailStep request: {:?}", request);
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

        // Convert duration to i64 for interval calculation (Postgres interval takes double precision for seconds but let's be safe)
        // We'll use make_interval in SQL or just multiply
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
