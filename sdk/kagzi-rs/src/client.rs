use std::collections::HashMap;
use std::future::{Future, IntoFuture};
use std::pin::Pin;

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
use crate::retry::RetryPolicy;

pub struct Client {
    workflow_client: WorkflowServiceClient<Channel>,
    schedule_client: WorkflowScheduleServiceClient<Channel>,
}

impl Client {
    pub async fn connect(addr: &str) -> anyhow::Result<Self> {
        let channel = Channel::from_shared(addr.to_string())?.connect().await?;

        Ok(Self {
            workflow_client: WorkflowServiceClient::new(channel.clone()),
            schedule_client: WorkflowScheduleServiceClient::new(channel),
        })
    }

    pub fn workflow<I: Serialize>(
        &mut self,
        workflow_type: &str,
        task_queue: &str,
        input: I,
    ) -> WorkflowBuilder<'_, I> {
        WorkflowBuilder::new(self, workflow_type, task_queue, input)
    }

    pub fn workflow_schedule<I: Serialize>(
        &mut self,
        workflow_type: &str,
        task_queue: &str,
        cron_expr: &str,
        input: I,
    ) -> WorkflowScheduleBuilder<'_, I> {
        WorkflowScheduleBuilder::new(self, workflow_type, task_queue, cron_expr, input)
    }

    pub async fn get_workflow_schedule(
        &mut self,
        schedule_id: &str,
        namespace_id: Option<&str>,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        let request = Request::new(GetWorkflowScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.unwrap_or("default").to_string(),
        });

        let resp = self
            .schedule_client
            .get_workflow_schedule(request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(resp.schedule)
    }

    pub async fn list_workflow_schedules(
        &mut self,
        namespace_id: &str,
        page: Option<PageRequest>,
    ) -> anyhow::Result<Vec<WorkflowSchedule>> {
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

        let resp = self
            .schedule_client
            .list_workflow_schedules(request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(resp.schedules)
    }

    pub async fn delete_workflow_schedule(
        &mut self,
        schedule_id: &str,
        namespace_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let request = Request::new(DeleteWorkflowScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.unwrap_or("default").to_string(),
        });

        self.schedule_client
            .delete_workflow_schedule(request)
            .await
            .map_err(map_grpc_error)?;

        Ok(())
    }
}

pub struct WorkflowBuilder<'a, I> {
    client: &'a mut Client,
    workflow_type: String,
    task_queue: String,
    input: I,
    namespace_id: String,
    external_id: Option<String>,
    version: Option<String>,
    retry_policy: Option<RetryPolicy>,
}

impl<'a, I: Serialize> WorkflowBuilder<'a, I> {
    fn new(client: &'a mut Client, workflow_type: &str, task_queue: &str, input: I) -> Self {
        Self {
            client,
            workflow_type: workflow_type.to_string(),
            task_queue: task_queue.to_string(),
            input,
            namespace_id: "default".to_string(),
            external_id: None,
            version: None,
            retry_policy: None,
        }
    }

    pub fn id(mut self, external_id: impl Into<String>) -> Self {
        self.external_id = Some(external_id.into());
        self
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace_id = ns.into();
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    pub fn retries(mut self, max_attempts: i32) -> Self {
        self.retry_policy
            .get_or_insert_with(Default::default)
            .maximum_attempts = Some(max_attempts);
        self
    }

    async fn execute(self) -> anyhow::Result<String> {
        let input_bytes = serde_json::to_vec(&self.input)?;

        let resp = self
            .client
            .workflow_client
            .start_workflow(Request::new(StartWorkflowRequest {
                external_id: self
                    .external_id
                    .unwrap_or_else(|| Uuid::now_v7().to_string()),
                task_queue: self.task_queue,
                workflow_type: self.workflow_type,
                input: Some(ProtoPayload {
                    data: input_bytes,
                    metadata: HashMap::new(),
                }),
                namespace_id: self.namespace_id,
                version: self.version.unwrap_or_default(),
                retry_policy: self.retry_policy.map(Into::into),
            }))
            .await
            .map_err(map_grpc_error)?;

        Ok(resp.into_inner().run_id)
    }
}

impl<'a, I: Serialize + Send + 'a> IntoFuture for WorkflowBuilder<'a, I> {
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;
    type Output = anyhow::Result<String>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.execute())
    }
}

pub struct WorkflowScheduleBuilder<'a, I> {
    client: &'a mut Client,
    workflow_type: String,
    task_queue: String,
    cron_expr: String,
    input: I,
    namespace_id: String,
    enabled: Option<bool>,
    max_catchup: Option<i32>,
    version: Option<String>,
}

impl<'a, I: Serialize> WorkflowScheduleBuilder<'a, I> {
    fn new(
        client: &'a mut Client,
        workflow_type: &str,
        task_queue: &str,
        cron_expr: &str,
        input: I,
    ) -> Self {
        Self {
            client,
            workflow_type: workflow_type.to_string(),
            task_queue: task_queue.to_string(),
            cron_expr: cron_expr.to_string(),
            input,
            namespace_id: "default".to_string(),
            enabled: None,
            max_catchup: None,
            version: None,
        }
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace_id = ns.into();
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    pub fn max_catchup(mut self, max_catchup: i32) -> Self {
        self.max_catchup = Some(max_catchup);
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    async fn create(self) -> anyhow::Result<WorkflowSchedule> {
        let input_bytes = serde_json::to_vec(&self.input)?;

        let request = CreateWorkflowScheduleRequest {
            namespace_id: self.namespace_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            cron_expr: self.cron_expr,
            input: Some(ProtoPayload {
                data: input_bytes,
                metadata: HashMap::new(),
            }),
            enabled: self.enabled,
            max_catchup: self.max_catchup,
            version: self.version,
        };

        let resp = self
            .client
            .schedule_client
            .create_workflow_schedule(Request::new(request))
            .await
            .map_err(map_grpc_error)?
            .into_inner()
            .schedule
            .ok_or_else(|| anyhow!("Workflow schedule not returned by server"))?;

        Ok(resp)
    }
}

impl<'a, I: Serialize + Send + 'a> IntoFuture for WorkflowScheduleBuilder<'a, I> {
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;
    type Output = anyhow::Result<WorkflowSchedule>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.create())
    }
}
