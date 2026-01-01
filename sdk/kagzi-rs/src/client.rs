use std::collections::HashMap;

use anyhow::anyhow;
use kagzi_proto::kagzi::workflow_schedule_service_client::WorkflowScheduleServiceClient;
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    CreateWorkflowScheduleRequest, DeleteWorkflowScheduleRequest, GetWorkflowScheduleRequest,
    ListWorkflowSchedulesRequest, PageRequest, Payload as ProtoPayload, StartWorkflowRequest,
    WorkflowSchedule,
};
use serde::Serialize;
use tonic::Request;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::errors::map_grpc_error;

/// Main client for interacting with Kagzi server
pub struct Kagzi {
    workflow_client: WorkflowServiceClient<Channel>,
    schedule_client: WorkflowScheduleServiceClient<Channel>,
}

impl Kagzi {
    pub async fn connect(addr: &str) -> anyhow::Result<Self> {
        let channel = Channel::from_shared(addr.to_string())?.connect().await?;

        Ok(Self {
            workflow_client: WorkflowServiceClient::new(channel.clone()),
            schedule_client: WorkflowScheduleServiceClient::new(channel),
        })
    }

    pub fn start(&self, workflow_type: &str) -> StartWorkflowBuilder<'_> {
        StartWorkflowBuilder {
            client: &self.workflow_client,
            workflow_type: workflow_type.to_string(),
            namespace: "default".to_string(),
            input: None,
            idempotency_key: None,
        }
    }

    pub fn schedule(&self, schedule_id: &str) -> ScheduleBuilder<'_> {
        ScheduleBuilder {
            client: &self.schedule_client,
            _schedule_id: schedule_id.to_string(),
            namespace: "default".to_string(),
            workflow_type: None,
            cron: None,
            input: None,
            max_catchup: 100,
            enabled: true,
        }
    }

    // Schedule query methods
    pub async fn get_workflow_schedule(
        &self,
        schedule_id: &str,
        namespace_id: Option<&str>,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        let mut client = self.schedule_client.clone();
        let request = Request::new(GetWorkflowScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.unwrap_or("default").to_string(),
        });

        let resp = client
            .get_workflow_schedule(request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(resp.schedule)
    }

    pub async fn list_workflow_schedules(
        &self,
        namespace_id: &str,
        page: Option<PageRequest>,
    ) -> anyhow::Result<Vec<WorkflowSchedule>> {
        let mut client = self.schedule_client.clone();
        let page_request = page.unwrap_or(PageRequest {
            page_size: 100,
            page_token: "".to_string(),
            include_total_count: false,
        });

        let request = Request::new(ListWorkflowSchedulesRequest {
            namespace_id: namespace_id.to_string(),
            task_queue: None,
            page: Some(page_request),
        });

        let resp = client
            .list_workflow_schedules(request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(resp.schedules)
    }

    pub async fn delete_workflow_schedule(
        &self,
        schedule_id: &str,
        namespace_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut client = self.schedule_client.clone();
        let request = Request::new(DeleteWorkflowScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.unwrap_or("default").to_string(),
        });

        client
            .delete_workflow_schedule(request)
            .await
            .map_err(map_grpc_error)?;

        Ok(())
    }
}

pub struct StartWorkflowBuilder<'a> {
    client: &'a WorkflowServiceClient<Channel>,
    workflow_type: String,
    namespace: String,
    input: Option<Vec<u8>>,
    idempotency_key: Option<String>,
}

impl<'a> StartWorkflowBuilder<'a> {
    pub fn namespace(mut self, ns: &str) -> Self {
        self.namespace = ns.to_string();
        self
    }

    pub fn input<T: Serialize>(mut self, input: T) -> Self {
        self.input = Some(serde_json::to_vec(&input).expect("Failed to serialize input"));
        self
    }

    /// Set idempotency key to prevent duplicate workflows
    pub fn id(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    /// Send the workflow start request and wait for the run to be created
    pub async fn send(self) -> anyhow::Result<WorkflowRun> {
        let mut client = self.client.clone();
        let resp = client
            .start_workflow(Request::new(StartWorkflowRequest {
                external_id: self
                    .idempotency_key
                    .unwrap_or_else(|| Uuid::now_v7().to_string()),
                task_queue: self.workflow_type.clone(), // Queue = workflow type
                workflow_type: self.workflow_type,
                input: self.input.map(|data| ProtoPayload {
                    data,
                    metadata: HashMap::new(),
                }),
                namespace_id: self.namespace,
                version: String::default(),
                retry_policy: None,
            }))
            .await
            .map_err(map_grpc_error)?;

        Ok(WorkflowRun {
            id: resp.into_inner().run_id,
        })
    }
}

#[derive(Debug)]
pub struct WorkflowRun {
    pub id: String,
}

pub struct ScheduleBuilder<'a> {
    client: &'a WorkflowScheduleServiceClient<Channel>,
    _schedule_id: String,
    namespace: String,
    workflow_type: Option<String>,
    cron: Option<String>,
    input: Option<Vec<u8>>,
    max_catchup: i32,
    enabled: bool,
}

impl<'a> ScheduleBuilder<'a> {
    pub fn namespace(mut self, ns: &str) -> Self {
        self.namespace = ns.to_string();
        self
    }

    pub fn workflow(mut self, workflow_type: &str) -> Self {
        self.workflow_type = Some(workflow_type.to_string());
        self
    }

    pub fn cron(mut self, cron_expr: &str) -> Self {
        self.cron = Some(cron_expr.to_string());
        self
    }

    pub fn input<T: Serialize>(mut self, input: T) -> Self {
        self.input = Some(serde_json::to_vec(&input).expect("Failed to serialize input"));
        self
    }

    pub fn catchup(mut self, max: i32) -> Self {
        self.max_catchup = max;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Send the schedule creation request and wait for the schedule to be created
    pub async fn send(self) -> anyhow::Result<WorkflowSchedule> {
        let workflow_type = self
            .workflow_type
            .ok_or_else(|| anyhow!("workflow_type is required"))?;
        let cron = self
            .cron
            .ok_or_else(|| anyhow!("cron expression is required"))?;

        let request = CreateWorkflowScheduleRequest {
            namespace_id: self.namespace,
            task_queue: workflow_type.clone(), // Queue = workflow type
            workflow_type,
            cron_expr: cron,
            input: self.input.map(|data| ProtoPayload {
                data,
                metadata: HashMap::new(),
            }),
            enabled: Some(self.enabled),
            max_catchup: Some(self.max_catchup),
            version: None,
        };

        let mut client = self.client.clone();
        let resp = client
            .create_workflow_schedule(Request::new(request))
            .await
            .map_err(map_grpc_error)?
            .into_inner()
            .schedule
            .ok_or_else(|| anyhow!("Workflow schedule not returned by server"))?;

        Ok(resp)
    }
}
