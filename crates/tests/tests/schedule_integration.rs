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
    async fn register_worker(
        &self,
        _request: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        Err(Status::unimplemented("register_worker"))
    }

    async fn worker_heartbeat(
        &self,
        _request: Request<kagzi_proto::kagzi::WorkerHeartbeatRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::WorkerHeartbeatResponse>, Status> {
        Err(Status::unimplemented("worker_heartbeat"))
    }

    async fn deregister_worker(
        &self,
        _request: Request<kagzi_proto::kagzi::DeregisterWorkerRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("deregister_worker"))
    }

    async fn list_workers(
        &self,
        _request: Request<kagzi_proto::kagzi::ListWorkersRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::ListWorkersResponse>, Status> {
        Err(Status::unimplemented("list_workers"))
    }

    async fn get_worker(
        &self,
        _request: Request<kagzi_proto::kagzi::GetWorkerRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::GetWorkerResponse>, Status> {
        Err(Status::unimplemented("get_worker"))
    }

    async fn start_workflow(
        &self,
        _request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        Err(Status::unimplemented("start_workflow"))
    }

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

    async fn get_schedule(
        &self,
        _request: Request<GetScheduleRequest>,
    ) -> Result<Response<GetScheduleResponse>, Status> {
        Err(Status::unimplemented("get_schedule"))
    }

    async fn list_schedules(
        &self,
        _request: Request<ListSchedulesRequest>,
    ) -> Result<Response<ListSchedulesResponse>, Status> {
        Err(Status::unimplemented("list_schedules"))
    }

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
            next_fire_at: Some(req.next_fire_at.unwrap_or_else(now_ts)),
            last_fired_at: None,
            version: req.version.clone().unwrap_or_default(),
        };

        Ok(Response::new(UpdateScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn delete_schedule(
        &self,
        _request: Request<DeleteScheduleRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("delete_schedule"))
    }

    async fn get_workflow_run(
        &self,
        _request: Request<kagzi_proto::kagzi::GetWorkflowRunRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::GetWorkflowRunResponse>, Status> {
        Err(Status::unimplemented("get_workflow_run"))
    }

    async fn list_workflow_runs(
        &self,
        _request: Request<kagzi_proto::kagzi::ListWorkflowRunsRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::ListWorkflowRunsResponse>, Status> {
        Err(Status::unimplemented("list_workflow_runs"))
    }

    async fn cancel_workflow_run(
        &self,
        _request: Request<kagzi_proto::kagzi::CancelWorkflowRunRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("cancel_workflow_run"))
    }

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

    async fn list_step_attempts(
        &self,
        _request: Request<kagzi_proto::kagzi::ListStepAttemptsRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::ListStepAttemptsResponse>, Status> {
        Err(Status::unimplemented("list_step_attempts"))
    }

    async fn begin_step(
        &self,
        _request: Request<kagzi_proto::kagzi::BeginStepRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::BeginStepResponse>, Status> {
        Err(Status::unimplemented("begin_step"))
    }

    async fn complete_step(
        &self,
        _request: Request<kagzi_proto::kagzi::CompleteStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("complete_step"))
    }

    async fn fail_step(
        &self,
        _request: Request<kagzi_proto::kagzi::FailStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("fail_step"))
    }

    async fn complete_workflow(
        &self,
        _request: Request<kagzi_proto::kagzi::CompleteWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("complete_workflow"))
    }

    async fn fail_workflow(
        &self,
        _request: Request<kagzi_proto::kagzi::FailWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("fail_workflow"))
    }

    async fn schedule_sleep(
        &self,
        _request: Request<ScheduleSleepRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("schedule_sleep"))
    }

    async fn health_check(
        &self,
        _request: Request<kagzi_proto::kagzi::HealthCheckRequest>,
    ) -> Result<Response<kagzi_proto::kagzi::HealthCheckResponse>, Status> {
        Err(Status::unimplemented("health_check"))
    }
}

fn now_ts() -> prost_types::Timestamp {
    let now = Utc::now();
    prost_types::Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

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
