use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CancelWorkflowRunRequest, CompleteStepRequest,
    CompleteWorkflowRequest, Empty, FailStepRequest, FailWorkflowRequest, GetWorkflowRunRequest,
    GetWorkflowRunResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse, PollActivityRequest,
    PollActivityResponse, RecordHeartbeatRequest, ScheduleSleepRequest, StartWorkflowRequest,
    StartWorkflowResponse,
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

        let input_json: serde_json::Value = serde_json::from_slice(&req.input)
            .map_err(|e| Status::invalid_argument(format!("Input must be valid JSON: {}", e)))?;

        let run_id = sqlx::query!(
            r#"
            INSERT INTO kagzi.workflow_runs (
                business_id, task_queue, workflow_type, status, input
            )
            VALUES ($1, $2, $3, 'PENDING', $4)
            RETURNING run_id
            "#,
            req.workflow_id,
            req.task_queue,
            req.workflow_type,
            input_json
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
                      AND status = 'PENDING'
                      AND (wake_up_at IS NULL OR wake_up_at <= NOW())
                    ORDER BY created_at ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                )
                RETURNING run_id, workflow_type, input
                "#,
                req.worker_id,
                req.task_queue
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
            WHERE run_id = $1 AND step_id = $2
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

        sqlx::query!(
            r#"
            INSERT INTO kagzi.step_runs (run_id, step_id, status, output, finished_at)
            VALUES ($1, $2, 'COMPLETED', $3, NOW())
            ON CONFLICT (run_id, step_id)
            DO UPDATE SET status = 'COMPLETED', output = $3, finished_at = NOW()
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
