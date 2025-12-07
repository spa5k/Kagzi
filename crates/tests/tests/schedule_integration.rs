use std::net::SocketAddr;
use std::sync::Arc;

use chrono::Utc;
use kagzi_proto::kagzi::workflow_schedule_service_server::{
    WorkflowScheduleService, WorkflowScheduleServiceServer,
};
use kagzi_proto::kagzi::{
    CreateWorkflowScheduleRequest, CreateWorkflowScheduleResponse, DeleteWorkflowScheduleRequest,
    DeleteWorkflowScheduleResponse, GetWorkflowScheduleRequest, GetWorkflowScheduleResponse,
    ListWorkflowSchedulesRequest, ListWorkflowSchedulesResponse, UpdateWorkflowScheduleRequest,
    UpdateWorkflowScheduleResponse, WorkflowSchedule,
};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

#[derive(Clone, Default)]
struct MockWorkflowScheduleService {
    last_create: Arc<Mutex<Option<CreateWorkflowScheduleRequest>>>,
    last_update: Arc<Mutex<Option<UpdateWorkflowScheduleRequest>>>,
}

#[tonic::async_trait]
impl WorkflowScheduleService for MockWorkflowScheduleService {
    async fn create_workflow_schedule(
        &self,
        request: Request<CreateWorkflowScheduleRequest>,
    ) -> Result<Response<CreateWorkflowScheduleResponse>, Status> {
        let req = request.into_inner();
        *self.last_create.lock().await = Some(req.clone());

        let schedule = WorkflowSchedule {
            schedule_id: Uuid::now_v7().to_string(),
            namespace_id: req.namespace_id.clone(),
            task_queue: req.task_queue.clone(),
            workflow_type: req.workflow_type.clone(),
            cron_expr: req.cron_expr.clone(),
            input: req.input.clone(),
            context: req.context.clone(),
            enabled: req.enabled.unwrap_or(true),
            max_catchup: req.max_catchup.unwrap_or_default(),
            next_fire_at: Some(now_ts()),
            last_fired_at: None,
            version: req.version.unwrap_or_default(),
            created_at: None,
            updated_at: None,
        };

        Ok(Response::new(CreateWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn get_workflow_schedule(
        &self,
        _request: Request<GetWorkflowScheduleRequest>,
    ) -> Result<Response<GetWorkflowScheduleResponse>, Status> {
        Err(Status::unimplemented("get_workflow_schedule"))
    }

    async fn list_workflow_schedules(
        &self,
        _request: Request<ListWorkflowSchedulesRequest>,
    ) -> Result<Response<ListWorkflowSchedulesResponse>, Status> {
        Err(Status::unimplemented("list_workflow_schedules"))
    }

    async fn update_workflow_schedule(
        &self,
        request: Request<UpdateWorkflowScheduleRequest>,
    ) -> Result<Response<UpdateWorkflowScheduleResponse>, Status> {
        let req = request.into_inner();
        *self.last_update.lock().await = Some(req.clone());

        let schedule = WorkflowSchedule {
            schedule_id: req.schedule_id.clone(),
            namespace_id: req.namespace_id.clone(),
            task_queue: req.task_queue.unwrap_or_else(|| "default".to_string()),
            workflow_type: req.workflow_type.unwrap_or_else(|| "wf".to_string()),
            cron_expr: req.cron_expr.unwrap_or_else(|| "0 * * * * *".to_string()),
            input: req.input,
            context: req.context,
            enabled: req.enabled.unwrap_or(true),
            max_catchup: req.max_catchup.unwrap_or(100),
            next_fire_at: Some(req.next_fire_at.unwrap_or_else(now_ts)),
            last_fired_at: None,
            version: req.version.unwrap_or_default(),
            created_at: None,
            updated_at: None,
        };

        Ok(Response::new(UpdateWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn delete_workflow_schedule(
        &self,
        _request: Request<DeleteWorkflowScheduleRequest>,
    ) -> Result<Response<DeleteWorkflowScheduleResponse>, Status> {
        Err(Status::unimplemented("delete_workflow_schedule"))
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
    service: MockWorkflowScheduleService,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let incoming = TcpListenerStream::new(listener);

    let svc = service.clone();
    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(WorkflowScheduleServiceServer::new(svc))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    Ok((addr, handle))
}

#[tokio::test]
async fn create_schedule_uses_native_cron_format() -> anyhow::Result<()> {
    let mock = MockWorkflowScheduleService::default();
    let (addr, handle) = start_mock_server(mock.clone()).await?;

    let mut client = kagzi::Client::connect(&format!("http://{}", addr)).await?;

    let cron = "0 */5 * * * *";
    let _schedule = client
        .workflow_schedule("wf", "queue", cron, serde_json::json!({"hello": "world"}))
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

    let input_json: serde_json::Value = serde_json::from_slice(&recorded.input.unwrap().data)?;
    assert_eq!(input_json, serde_json::json!({"hello": "world"}));
    Ok(())
}

#[tokio::test]
async fn update_schedule_passes_cron_and_version() -> anyhow::Result<()> {
    let mock = MockWorkflowScheduleService::default();
    let (addr, handle) = start_mock_server(mock.clone()).await?;

    let mut client = kagzi_proto::kagzi::workflow_schedule_service_client::WorkflowScheduleServiceClient::connect(format!("http://{}", addr)).await?;

    let schedule_id = Uuid::now_v7().to_string();
    let cron = "15 0 8 * * *"; // 08:00:15 UTC daily
    let _ = client
        .update_workflow_schedule(Request::new(UpdateWorkflowScheduleRequest {
            schedule_id: schedule_id.clone(),
            namespace_id: "default".to_string(),
            task_queue: None,
            workflow_type: None,
            cron_expr: Some(cron.to_string()),
            input: None,
            context: None,
            enabled: None,
            max_catchup: Some(50),
            next_fire_at: None,
            version: Some("v2".to_string()),
        }))
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
