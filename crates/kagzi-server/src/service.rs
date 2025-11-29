use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CompleteStepRequest, CompleteWorkflowRequest, Empty,
    FailStepRequest, PollActivityRequest, PollActivityResponse, ScheduleSleepRequest,
    StartWorkflowRequest, StartWorkflowResponse,
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
        info!("StartWorkflow request: {:?}", request);
        Ok(Response::new(StartWorkflowResponse {
            run_id: "stub-run-id".to_string(),
        }))
    }

    async fn poll_activity(
        &self,
        request: Request<PollActivityRequest>,
    ) -> Result<Response<PollActivityResponse>, Status> {
        info!("PollActivity request: {:?}", request);
        // For now, just block indefinitely or return empty to simulate "no work"
        // In a real implementation, this would use long-polling
        Err(Status::deadline_exceeded("No work available (stub)"))
    }

    async fn begin_step(
        &self,
        request: Request<BeginStepRequest>,
    ) -> Result<Response<BeginStepResponse>, Status> {
        info!("BeginStep request: {:?}", request);
        Ok(Response::new(BeginStepResponse {
            should_execute: true,
            cached_result: vec![],
        }))
    }

    async fn complete_step(
        &self,
        request: Request<CompleteStepRequest>,
    ) -> Result<Response<Empty>, Status> {
        info!("CompleteStep request: {:?}", request);
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
        info!("CompleteWorkflow request: {:?}", request);
        Ok(Response::new(Empty {}))
    }

    async fn schedule_sleep(
        &self,
        request: Request<ScheduleSleepRequest>,
    ) -> Result<Response<Empty>, Status> {
        info!("ScheduleSleep request: {:?}", request);
        Ok(Response::new(Empty {}))
    }
}
