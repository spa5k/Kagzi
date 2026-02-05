use std::collections::HashMap;

use kagzi_proto::kagzi::queue_service_server::QueueService;
use kagzi_proto::kagzi::{
    CreateQueueRequest, CreateQueueResponse, GetQueueRequest, GetQueueResponse, ListQueuesRequest,
    ListQueuesResponse, PageInfo, TaskQueue, UpdateQueueRequest, UpdateQueueResponse,
};
use kagzi_store::PgStore;
use tonic::{Request, Response, Status};
use tracing::instrument;

use crate::constants::DEFAULT_TASK_QUEUE;
use crate::helpers::{invalid_argument_error, normalize_page_size, require_non_empty};
use crate::proto_convert::timestamp_from;
use crate::queue_store;

#[derive(Clone)]
pub struct QueueServiceImpl {
    store: PgStore,
}

impl QueueServiceImpl {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }
}

fn validate_task_queue_name(task_queue: &str) -> Result<(), Status> {
    if task_queue.is_empty() {
        return Err(invalid_argument_error("task_queue is required"));
    }
    if task_queue.len() > 64 {
        return Err(invalid_argument_error(
            "task_queue must be <= 64 characters",
        ));
    }
    if !task_queue
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid_argument_error(
            "task_queue must contain only letters, numbers, '_' or '-'",
        ));
    }
    Ok(())
}

fn normalize_optional_string(v: Option<String>) -> Option<Option<String>> {
    v.map(|s| if s.trim().is_empty() { None } else { Some(s) })
}

fn extra_value_to_bytes(extra: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(extra).unwrap_or_default()
}

fn row_to_proto(row: queue_store::TaskQueueRow) -> TaskQueue {
    TaskQueue {
        namespace: row.namespace,
        task_queue: row.task_queue,
        display_name: row.display_name,
        description: row.description,
        labels: row.labels,
        extra_json: extra_value_to_bytes(&row.extra),
        enabled: row.enabled,
        created_at: Some(timestamp_from(row.created_at)),
        updated_at: Some(timestamp_from(row.updated_at)),
    }
}

#[tonic::async_trait]
impl QueueService for QueueServiceImpl {
    #[instrument(skip(self, request))]
    async fn create_queue(
        &self,
        request: Request<CreateQueueRequest>,
    ) -> Result<Response<CreateQueueResponse>, Status> {
        let req = request.into_inner();

        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;
        validate_task_queue_name(&task_queue)?;

        // Reserve "default" as a first-class per-namespace queue; allow creating it explicitly.
        let enabled = req.enabled.unwrap_or(true);
        let display_name = req.display_name.filter(|s| !s.trim().is_empty());
        let description = req.description.filter(|s| !s.trim().is_empty());
        let labels: HashMap<String, String> = req.labels;
        let extra_json = req.extra_json;

        let row = queue_store::create_task_queue(
            &self.store,
            &namespace,
            &task_queue,
            queue_store::CreateTaskQueueInput {
                display_name,
                description,
                labels,
                extra_json,
                enabled,
            },
        )
        .await?;

        Ok(Response::new(CreateQueueResponse {
            queue: Some(row_to_proto(row)),
        }))
    }

    #[instrument(skip(self, request))]
    async fn get_queue(
        &self,
        request: Request<GetQueueRequest>,
    ) -> Result<Response<GetQueueResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;

        let row = queue_store::get_task_queue(&self.store, &namespace, &task_queue).await?;

        Ok(Response::new(GetQueueResponse {
            queue: Some(row_to_proto(row)),
        }))
    }

    #[instrument(skip(self, request))]
    async fn list_queues(
        &self,
        request: Request<ListQueuesRequest>,
    ) -> Result<Response<ListQueuesResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let page = req
            .page
            .ok_or_else(|| invalid_argument_error("page is required"))?;
        let page_size = normalize_page_size(page.page_size, 50, 200);
        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(page.page_token)
        };

        let (rows, next_cursor, has_more) =
            queue_store::list_task_queues(&self.store, &namespace, page_size, cursor).await?;

        Ok(Response::new(ListQueuesResponse {
            queues: rows.into_iter().map(row_to_proto).collect(),
            page: Some(PageInfo {
                next_page_token: next_cursor.unwrap_or_default(),
                has_more,
                total_count: 0,
            }),
        }))
    }

    #[instrument(skip(self, request))]
    async fn update_queue(
        &self,
        request: Request<UpdateQueueRequest>,
    ) -> Result<Response<UpdateQueueResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;

        if task_queue == DEFAULT_TASK_QUEUE && req.enabled == Some(false) {
            return Err(invalid_argument_error("default queue cannot be disabled"));
        }

        let display_name = normalize_optional_string(req.display_name);
        let description = normalize_optional_string(req.description);
        let enabled = req.enabled;

        // Proto map cannot be optional; treat empty map as "leave unchanged".
        let labels = (!req.labels.is_empty()).then_some(req.labels as HashMap<String, String>);

        let row = queue_store::update_task_queue(
            &self.store,
            &namespace,
            &task_queue,
            queue_store::UpdateTaskQueueInput {
                display_name,
                description,
                enabled,
                labels,
                extra_json: req.extra_json,
            },
        )
        .await?;

        Ok(Response::new(UpdateQueueResponse {
            queue: Some(row_to_proto(row)),
        }))
    }
}
