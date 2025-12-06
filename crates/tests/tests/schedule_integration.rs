use std::net::SocketAddr;
use std::sync::Arc;

use chrono::Utc;
use kagzi::ScheduleUpdate;
use kagzi_proto::kagzi::workflow_service_server::{WorkflowService, WorkflowServiceServer};
use kagzi_proto::kagzi::{
    CreateScheduleRequest, CreateScheduleResponse, DeleteScheduleRequest, Empty,
    GetScheduleRequest, GetScheduleResponse, ListSchedulesRequest, ListSchedulesResponse,
    RegisterWorkerRequest, RegisterWorkerResponse, Schedule, ScheduleSleepRequest,
    StartWorkflowRequest, StartWorkflowResponse, UpdateScheduleRequest, UpdateScheduleResponse,
};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

#[derive(Clone, Default)]
struct MockWorkflowService {
    last_create: Arc<Mutex<Option<CreateScheduleRequest>>>,
    last_update: Arc<Mutex<Option<UpdateScheduleRequest>>>,
}

#[tonic::async_trait]
impl WorkflowService for MockWorkflowService {
    /// gRPC handler for registering a worker in the mock service.
    ///
    /// Always responds with an unimplemented gRPC `Status` using the message `"register_worker"`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Assuming `svc` is an instance of `MockWorkflowService` and `req` is a `Request<RegisterWorkerRequest>`.
    /// let res = svc.register_worker(req).await;
    /// assert!(res.is_err());
    /// ```
    async fn register_worker(
        &self,
        _request: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        Err(Status::unimplemented("register_worker"))
    }

    /// Handles heartbeat messages from a worker, reporting its liveness and returning any follow-up instructions.
    ///
    /// Processes a `WorkerHeartbeatRequest` from a worker and returns a `WorkerHeartbeatResponse` that acknowledges receipt and may include updated directives or assignment changes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kagzi_proto::kagzi::WorkerHeartbeatRequest;
    /// use tonic::Request;
    ///
    /// // Construct a basic heartbeat request (fields depend on proto definition)
    /// let req = WorkerHeartbeatRequest { ..Default::default() };
    /// // Calling this method requires an async runtime and a service instance that implements it:
    /// // let resp = tokio::runtime::Runtime::new().unwrap().block_on(service.worker_heartbeat(Request::new(req)));
    /// ```
    async fn worker_heartbeat(
        &self,
        _request: Request<kagzi_proto::kagzi::WorkerHeartbeatRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::WorkerHeartbeatResponse>, Status> {
        Err(Status::unimplemented("worker_heartbeat"))
    }

    /// Deregisters a worker from the service.
    ///
    /// Removes the specified worker from the registry so it will no longer receive tasks or heartbeats.
    ///
    /// # Returns
    ///
    /// `Empty` on success.
    async fn deregister_worker(
        &self,
        _request: Request<kagzi_proto::kagzi::DeregisterWorkerRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("deregister_worker"))
    }

    /// Handles a request to list workers for the workflow service.
    ///
    /// Currently this implementation always returns a gRPC `Unimplemented` status.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tonic::Request;
    /// # use kagzi_proto::kagzi::ListWorkersRequest;
    /// # let svc = crate::MockWorkflowService::default();
    /// # let _ = tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let req = Request::new(ListWorkersRequest {});
    /// let res = svc.list_workers(req).await;
    /// assert!(res.is_err());
    /// # });
    /// ```
    async fn list_workers(
        &self,
        _request: Request<kagzi_proto::kagzi::ListWorkersRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::ListWorkersResponse>, Status> {
        Err(Status::unimplemented("list_workers"))
    }

    /// Retrieves information about a worker identified by the request.
    ///
    /// Returns a `GetWorkerResponse` containing the worker's details.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tonic::Request;
    /// # async fn example(svc: &crate::MockWorkflowService) {
    /// let req = kagzi_proto::kagzi::GetWorkerRequest { /* fields */ };
    /// let _resp = svc.get_worker(Request::new(req)).await;
    /// # }
    /// ```
    async fn get_worker(
        &self,
        _request: Request<kagzi_proto::kagzi::GetWorkerRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::GetWorkerResponse>, Status> {
        Err(Status::unimplemented("get_worker"))
    }

    /// Starts a workflow run described by the provided `StartWorkflowRequest`.
    ///
    /// # Returns
    ///
    /// A `StartWorkflowResponse` containing details about the started workflow.
    ///
    /// # Examples
    ///
    /// ```
    /// # tokio_test::block_on(async {
    /// # use tonic::Request;
    /// # use crate::MockWorkflowService;
    /// # use crate::StartWorkflowRequest;
    /// let svc = MockWorkflowService::default();
    /// let req = StartWorkflowRequest::default();
    /// let _ = svc.start_workflow(Request::new(req)).await;
    /// # });
    /// ```
    async fn start_workflow(
        &self,
        _request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        Err(Status::unimplemented("start_workflow"))
    }

    /// Creates a Schedule from the given CreateScheduleRequest and records the request in the mock's `last_create`.
    ///
    /// The constructed Schedule receives a new UUID for `schedule_id`, copies relevant fields from the request,
    /// sets `next_fire_at` to the current time, and uses `true` for `enabled` when that field is not provided.
    ///
    /// # Returns
    /// A `CreateScheduleResponse` containing the created `Schedule`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tonic::Request;
    /// # use crate::mocks::MockWorkflowService;
    /// # use crate::proto::CreateScheduleRequest;
    /// # tokio_test::block_on(async {
    /// let svc = MockWorkflowService::default();
    /// let req = CreateScheduleRequest { namespace_id: "default".into(), ..Default::default() };
    /// let resp = svc.create_schedule(Request::new(req)).await.unwrap();
    /// assert!(resp.get_ref().schedule.is_some());
    /// # });
    /// ```
    async fn create_schedule(
        &self,
        request: Request<CreateScheduleRequest>,
    ) -> Result<Response<CreateScheduleResponse>, Status> {
        let req = request.into_inner();
        *self.last_create.lock().await = Some(req.clone());

        let schedule = Schedule {
            schedule_id: Uuid::new_v4().to_string(),
            namespace_id: req.namespace_id.clone(),
            task_queue: req.task_queue.clone(),
            workflow_type: req.workflow_type.clone(),
            cron_expr: req.cron_expr.clone(),
            input: req.input.clone(),
            context: req.context.clone(),
            enabled: req.enabled.unwrap_or(true),
            max_catchup: req.max_catchup,
            next_fire_at: Some(now_ts()),
            last_fired_at: None,
            version: req.version.clone(),
        };

        Ok(Response::new(CreateScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    /// Retrieves a schedule specified by the request.
    ///
    /// # Examples
    ///
    /// ```
    /// # tokio_test::block_on(async {
    /// let req = GetScheduleRequest { schedule_id: "id".into(), namespace_id: "ns".into(), ..Default::default() };
    /// let _ = service.get_schedule(tonic::Request::new(req)).await;
    /// # });
    /// ```
    async fn get_schedule(
        &self,
        _request: Request<GetScheduleRequest>,
    ) -> Result<Response<GetScheduleResponse>, Status> {
        Err(Status::unimplemented("get_schedule"))
    }

    /// Lists schedules that match the provided request filters.
    ///
    /// The response contains a `ListSchedulesResponse` with the schedules and any pagination token.
    /// On failure this method returns a gRPC `Status` error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tonic::Request;
    /// use my_crate::workflow::ListSchedulesRequest;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let svc = /* obtain service instance implementing this method */ unimplemented!();
    ///     let req = Request::new(ListSchedulesRequest { ..Default::default() });
    ///     let _resp = svc.list_schedules(req).await;
    /// }
    /// ```
    async fn list_schedules(
        &self,
        _request: Request<ListSchedulesRequest>,
    ) -> Result<Response<ListSchedulesResponse>, Status> {
        Err(Status::unimplemented("list_schedules"))
    }

    /// Updates a schedule from an `UpdateScheduleRequest` and returns the resulting schedule in the response.
    ///
    /// The request is recorded into the service's last_update field and a `Schedule` is constructed
    /// using values from the request with sensible defaults for any missing fields.
    ///
    /// # Returns
    ///
    /// An `UpdateScheduleResponse` containing the constructed `Schedule`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tonic::Request;
    /// # use tokio::runtime::Runtime;
    /// # use std::sync::Arc;
    /// # use tokio::sync::Mutex;
    /// # // Types `MockWorkflowService` and `UpdateScheduleRequest` are defined in the same crate.
    /// # use crate::{MockWorkflowService, UpdateScheduleRequest};
    /// let rt = Runtime::new().unwrap();
    /// let svc = MockWorkflowService {
    ///     last_create: Arc::new(Mutex::new(None)),
    ///     last_update: Arc::new(Mutex::new(None)),
    /// };
    ///
    /// let req = UpdateScheduleRequest::default();
    /// let resp = rt.block_on(async { svc.update_schedule(Request::new(req)).await.unwrap() });
    /// assert!(resp.into_inner().schedule.is_some());
    /// ```
    async fn update_schedule(
        &self,
        request: Request<UpdateScheduleRequest>,
    ) -> Result<Response<UpdateScheduleResponse>, Status> {
        let req = request.into_inner();
        *self.last_update.lock().await = Some(req.clone());

        let schedule = Schedule {
            schedule_id: req.schedule_id.clone(),
            namespace_id: req.namespace_id.clone(),
            task_queue: req
                .task_queue
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            workflow_type: req
                .workflow_type
                .clone()
                .unwrap_or_else(|| "wf".to_string()),
            cron_expr: req
                .cron_expr
                .clone()
                .unwrap_or_else(|| "0 * * * * *".to_string()),
            input: req.input.clone().unwrap_or_default(),
            context: req.context.clone().unwrap_or_default(),
            enabled: req.enabled.unwrap_or(true),
            max_catchup: req.max_catchup.unwrap_or(100),
            next_fire_at: Some(req.next_fire_at.clone().unwrap_or_else(now_ts)),
            last_fired_at: None,
            version: req.version.clone().unwrap_or_default(),
        };

        Ok(Response::new(UpdateScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    /// Signals that deleting a schedule is not implemented by this mock service.
    ///
    /// This method always returns a gRPC `UNIMPLEMENTED` status to indicate the
    /// operation is not supported by the mock.
    ///
    /// # Examples
    ///
    /// ```
    /// // The mock always responds with an UNIMPLEMENTED tonic::Status.
    /// // let svc = MockWorkflowService::default();
    /// // let req = tonic::Request::new(DeleteScheduleRequest { /* fields */ });
    /// // let res = futures::executor::block_on(svc.delete_schedule(req));
    /// // assert!(res.is_err());
    /// ```
    async fn delete_schedule(
        &self,
        _request: Request<DeleteScheduleRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("delete_schedule"))
    }

    /// Retrieves a workflow run by its identifier.
    ///
    /// Accepts a gRPC `GetWorkflowRunRequest` and returns the corresponding `GetWorkflowRunResponse` on success.
    ///
    /// # Examples
    ///
    /// ```
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use tonic::Request;
    /// use kagzi_proto::kagzi::GetWorkflowRunRequest;
    ///
    /// let svc = crate::MockWorkflowService::default();
    /// let req = Request::new(GetWorkflowRunRequest::default());
    /// let res = svc.get_workflow_run(req).await;
    /// // This mock implementation returns an unimplemented status.
    /// assert!(res.is_err());
    /// # });
    /// ```
    async fn get_workflow_run(
        &self,
        _request: Request<kagzi_proto::kagzi::GetWorkflowRunRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::GetWorkflowRunResponse>, Status> {
        Err(Status::unimplemented("get_workflow_run"))
    }

    /// Lists workflow runs that match the provided request filters.
    ///
    /// On success returns a `ListWorkflowRunsResponse` containing matching workflow runs and any
    /// pagination tokens; on failure returns a gRPC `Status` describing the error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tonic::Request;
    /// let req = kagzi_proto::kagzi::ListWorkflowRunsRequest { /* set filters */ };
    /// let resp = service.list_workflow_runs(Request::new(req)).await;
    /// match resp {
    ///     Ok(resp) => println!("got runs: {:?}", resp.into_inner().runs),
    ///     Err(status) => eprintln!("rpc error: {}", status),
    /// }
    /// ```
    async fn list_workflow_runs(
        &self,
        _request: Request<kagzi_proto::kagzi::ListWorkflowRunsRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::ListWorkflowRunsResponse>, Status> {
        Err(Status::unimplemented("list_workflow_runs"))
    }

    /// Handles a request to cancel a running workflow.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tonic::Request;
    /// use kagzi_proto::kagzi::CancelWorkflowRunRequest;
    ///
    /// // Construct the request (fields omitted for brevity)
    /// let req = Request::new(CancelWorkflowRunRequest { ..Default::default() });
    /// // `svc` is an instance of the service implementing this method
    /// // let resp = svc.cancel_workflow_run(req).await;
    /// ```
    —
    async fn cancel_workflow_run(
        &self,
        _request: Request<kagzi_proto::kagzi::CancelWorkflowRunRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("cancel_workflow_run"))
    }

    /// Polls for an available activity task for a worker to execute.
    ///
    /// Returns a `PollActivityResponse` containing an activity task when one is available.
    /// The call may fail with a gRPC `Status` on error.
    async fn poll_activity(
        &self,
        _request: Request<kagzi_proto::kagzi::PollActivityRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::PollActivityResponse>, Status> {
        Err(Status::unimplemented("poll_activity"))
    }

    async fn get_step_attempt(
        &self,
        _request: Request<kagzi_proto::kagzi::GetStepAttemptRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::GetStepAttemptResponse>, Status> {
        Err(Status::unimplemented("get_step_attempt"))
    }

    /// Lists execution attempts for a workflow step.
    ///
    /// The response contains the matching step attempts and any pagination token needed to fetch additional pages.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Construct a request with the step identifier and optional pagination params.
    /// let req = kagzi_proto::kagzi::ListStepAttemptsRequest {
    ///     // fill required fields, e.g. workflow_id, run_id, step_id, page_size, page_token
    ///     ..Default::default()
    /// };
    /// let resp = service.list_step_attempts(tonic::Request::new(req)).await;
    /// ```
    async fn list_step_attempts(
        &self,
        _request: Request<kagzi_proto::kagzi::ListStepAttemptsRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::ListStepAttemptsResponse>, Status> {
        Err(Status::unimplemented("list_step_attempts"))
    }

    /// Begins execution of a workflow step and returns metadata required to perform that step.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use kagzi_proto::kagzi::BeginStepRequest;
    /// # use tonic::Request;
    /// # async fn example(svc: &impl crate::workflow::WorkflowService) {
    /// let req = Request::new(BeginStepRequest { /* fill fields */ });
    /// let _resp = svc.begin_step(req).await.unwrap();
    /// # }
    /// ```
    async fn begin_step(
        &self,
        _request: Request<kagzi_proto::kagzi::BeginStepRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::BeginStepResponse>, Status> {
        Err(Status::unimplemented("begin_step"))
    }

    /// Marks a workflow step as completed and records its outcome.
    ///
    /// This RPC completes the step identified by the request and captures any provided
    /// output or completion metadata.
    ///
    /// # Returns
    ///
    /// `Ok(Response<Empty>)` if the step was completed successfully, or a gRPC `Status` error otherwise.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use kagzi_proto::kagzi::CompleteStepRequest;
    /// # async fn demo(svc: &impl crate::WorkflowService) {
    /// let req = CompleteStepRequest { /* populate fields */ };
    /// let res = svc.complete_step(tonic::Request::new(req)).await;
    /// match res {
    ///     Ok(_) => println!("step completed"),
    ///     Err(e) => eprintln!("completion failed: {}", e),
    /// }
    /// # }
    /// ```
    async fn complete_step(
        &self,
        _request: Request<kagzi_proto::kagzi::CompleteStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("complete_step"))
    }

    /// Marks a step attempt as failed for a workflow execution.
    ///
    /// # Returns
    ///
    /// `Empty` on success.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tonic::Request;
    /// # use kagzi_proto::kagzi::FailStepRequest;
    /// # async fn doc_example(svc: &crate::MockWorkflowService) {
    /// let req = Request::new(FailStepRequest {
    ///     // fill required fields
    ///     ..Default::default()
    /// });
    /// let _ = svc.fail_step(req).await;
    /// # }
    /// ```
    async fn fail_step(
        &self,
        _request: Request<kagzi_proto::kagzi::FailStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("fail_step"))
    }

    /// Completes a running workflow, marking it as finished and recording its final state.
    ///
    /// Currently this implementation always returns an `Unimplemented` gRPC `Status`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tonic::Code;
    /// # async fn example(svc: &crate::MockWorkflowService) {
    /// let req = kagzi_proto::kagzi::CompleteWorkflowRequest::default();
    /// let res = svc.complete_workflow(tonic::Request::new(req)).await;
    /// let err = res.expect_err("expected unimplemented error");
    /// assert_eq!(err.code(), Code::Unimplemented);
    /// # }
    /// ```
    async fn complete_workflow(
        &self,
        _request: Request<kagzi_proto::kagzi::CompleteWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("complete_workflow"))
    }

    /// Marks a workflow run as failed and records the failure details.
    ///
    /// Transitions the targeted workflow run into a failed terminal state.
    ///
    /// # Returns
    ///
    /// An empty response on success.
    async fn fail_workflow(
        &self,
        _request: Request<kagzi_proto::kagzi::FailWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("fail_workflow"))
    }

    /// Responds to schedule sleep requests with an unimplemented gRPC status.
    ///
    /// This RPC is not supported by the mock implementation and always returns
    /// `Status::unimplemented("schedule_sleep")`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tonic::Request;
    /// # use tonic::Code;
    /// # async fn doc() {
    /// let svc = crate::MockWorkflowService {
    ///     last_create: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
    ///     last_update: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
    /// };
    /// let result = svc.schedule_sleep(Request::new(tonic::transport::Body::empty())).await;
    /// let status = result.unwrap_err();
    /// assert_eq!(status.code(), Code::Unimplemented);
    /// # }
    /// ```
    async fn schedule_sleep(
        &self,
        _request: Request<ScheduleSleepRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("schedule_sleep"))
    }

    /// Reports the service's health status.
    ///
    /// # Returns
    ///
    /// `Ok(Response<HealthCheckResponse>)` containing the current health status when available, `Err(Status)` if the health check is not implemented or the service cannot report health.
    ///
    /// # Examples
    ///
    /// ```
    /// # use kagzi_proto::kagzi::HealthCheckRequest;
    /// # use tonic::Request;
    /// # use crate::mocks::MockWorkflowService;
    /// # let svc = MockWorkflowService::default();
    /// # let _ = tokio_test::block_on(svc.health_check(Request::new(HealthCheckRequest {})));
    /// ```
    async fn health_check(
        &self,
        _request: Request<kagzi_proto::kagzi::HealthCheckRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::HealthCheckResponse>, Status> {
        Err(Status::unimplemented("health_check"))
    }
}

/// Provides the current UTC time as a `prost_types::Timestamp`.
///
/// # Examples
///
/// ```
/// let ts = now_ts();
/// // seconds should be a positive Unix timestamp and nanos within 0..1_000_000_000
/// assert!(ts.seconds > 0);
/// assert!(ts.nanos >= 0 && ts.nanos < 1_000_000_000);
/// ```
fn now_ts() -> prost_types::Timestamp {
    let now = Utc::now();
    prost_types::Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

/// Starts a local gRPC server that serves the provided MockWorkflowService on an ephemeral localhost port.
///
/// Returns the bound SocketAddr and the spawned Tokio JoinHandle for the server task.
///
/// # Examples
///
/// ```
/// #[tokio::test]
/// async fn start_mock_server_example() {
///     let service = MockWorkflowService::default();
///     let (addr, handle) = start_mock_server(service).await.unwrap();
///     assert!(addr.port() != 0);
///     handle.abort();
/// }
/// ```
async fn start_mock_server(
    service: MockWorkflowService,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let incoming = TcpListenerStream::new(listener);

    let svc = service.clone();
    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(WorkflowServiceServer::new(svc))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    Ok((addr, handle))
}

/// Verifies that creating a schedule preserves the provided cron expression, task queue, workflow type, and JSON input payload.
///
/// # Examples
///
/// ```
/// # async fn run() -> anyhow::Result<()> {
/// let mock = MockWorkflowService::default();
/// let (addr, handle) = start_mock_server(mock.clone()).await?;
/// let mut client = kagzi::Client::connect(&format!("http://{}", addr)).await?;
/// let cron = "0 */5 * * * *";
/// let _ = client
///     .schedule("wf", "queue", cron)
///     .input(serde_json::json!({"hello": "world"}))
///     .create()
///     .await?;
/// handle.abort();
/// # Ok::<(), anyhow::Error>(())
/// # }
/// ```
#[tokio::test]
async fn create_schedule_uses_native_cron_format() -> anyhow::Result<()> {
    let mock = MockWorkflowService::default();
    let (addr, handle) = start_mock_server(mock.clone()).await?;

    let mut client = kagzi::Client::connect(&format!("http://{}", addr)).await?;

    let cron = "0 */5 * * * *";
    let _schedule = client
        .schedule("wf", "queue", cron)
        .input(serde_json::json!({"hello": "world"}))
        .create()
        .await?;

    handle.abort();

    let recorded = mock
        .last_create
        .lock()
        .await
        .clone()
        .expect("request captured");
    assert_eq!(recorded.cron_expr, cron);
    assert_eq!(recorded.task_queue, "queue");
    assert_eq!(recorded.workflow_type, "wf");

    let input_json: serde_json::Value = serde_json::from_slice(&recorded.input)?;
    assert_eq!(input_json, serde_json::json!({"hello": "world"}));
    Ok(())
}

#[tokio::test]
async fn update_schedule_passes_cron_and_version() -> anyhow::Result<()> {
    let mock = MockWorkflowService::default();
    let (addr, handle) = start_mock_server(mock.clone()).await?;

    let mut client = kagzi::Client::connect(&format!("http://{}", addr)).await?;

    let schedule_id = Uuid::new_v4().to_string();
    let cron = "15 0 8 * * *"; // 08:00:15 UTC daily
    let _ = client
        .update_schedule(
            &schedule_id,
            "default",
            ScheduleUpdate::default()
                .cron_expr(cron)
                .version("v2")
                .max_catchup(50),
        )
        .await?;

    handle.abort();

    let recorded = mock
        .last_update
        .lock()
        .await
        .clone()
        .expect("request captured");
    assert_eq!(recorded.schedule_id, schedule_id);
    assert_eq!(recorded.namespace_id, "default");
    assert_eq!(recorded.cron_expr.as_deref(), Some(cron));
    assert_eq!(recorded.version.as_deref(), Some("v2"));
    assert_eq!(recorded.max_catchup, Some(50));

    Ok(())
}