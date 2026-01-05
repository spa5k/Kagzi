use std::str::FromStr;

use chrono::Utc;
use kagzi_proto::kagzi::workflow_schedule_service_server::WorkflowScheduleService;
use kagzi_proto::kagzi::{
    CreateWorkflowScheduleRequest, CreateWorkflowScheduleResponse, DeleteWorkflowScheduleRequest,
    DeleteWorkflowScheduleResponse, GetWorkflowScheduleRequest, GetWorkflowScheduleResponse,
    ListWorkflowSchedulesRequest, ListWorkflowSchedulesResponse, PageInfo,
    UpdateWorkflowScheduleRequest, UpdateWorkflowScheduleResponse, WorkflowSchedule,
};
use kagzi_store::{CreateWorkflow, ListWorkflowsParams, PgStore, WorkflowRepository, WorkflowRun};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::helpers::{
    bytes_to_payload, decode_cursor, encode_cursor, invalid_argument_error, map_store_error,
    normalize_page_size, not_found_error, payload_to_optional_bytes, require_non_empty,
};
use crate::proto_convert::timestamp_from;

fn parse_cron_expr(expr: &str) -> Result<cron::Schedule, Status> {
    if expr.trim().is_empty() {
        return Err(invalid_argument_error("cron_expr cannot be empty"));
    }

    // Validate format: must be 6 fields (second minute hour day month weekday)
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 6 {
        return Err(invalid_argument_error(format!(
            "Invalid cron format: expected 6 fields (second minute hour day month weekday), got {}. Example: '0 */5 * * * *' (every 5 minutes)",
            fields.len()
        )));
    }

    cron::Schedule::from_str(expr).map_err(|e| {
        invalid_argument_error(format!(
            "Invalid cron expression: {}. Use 6-field format: second minute hour day month weekday",
            e
        ))
    })
}

fn next_fire_from_now(
    cron_expr: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>, Status> {
    let schedule = parse_cron_expr(cron_expr)?;
    let next = schedule
        .after(&now)
        .next()
        .ok_or_else(|| invalid_argument_error("Cron expression has no future occurrences"))?;
    Ok(next.with_timezone(&chrono::Utc))
}

// to_proto_timestamp removed

fn workflow_run_to_schedule_proto(w: WorkflowRun) -> Result<WorkflowSchedule, Status> {
    let input = bytes_to_payload(Some(w.input));

    let cron_expr = w
        .cron_expr
        .ok_or_else(|| invalid_argument_error("Workflow run is not a schedule template"))?;

    let next_fire_at = w
        .available_at
        .ok_or_else(|| invalid_argument_error("Schedule template has no next_fire_at"))?;

    let created_at = w
        .created_at
        .ok_or_else(|| invalid_argument_error("Schedule template has no created_at"))?;

    Ok(WorkflowSchedule {
        schedule_id: w.run_id.to_string(),
        namespace_id: w.namespace_id,
        task_queue: w.task_queue,
        workflow_type: w.workflow_type,
        cron_expr,
        input: Some(input),
        enabled: w.status == kagzi_store::WorkflowStatus::Scheduled,
        max_catchup: w.max_catchup,
        next_fire_at: Some(timestamp_from(next_fire_at)),
        last_fired_at: w.last_fired_at.map(timestamp_from),
        version: w.version.unwrap_or_default(),
        created_at: Some(timestamp_from(created_at)),
        updated_at: None,
    })
}

#[derive(Clone)]
pub struct WorkflowScheduleServiceImpl {
    pub store: PgStore,
    pub default_max_catchup: i32,
}

impl WorkflowScheduleServiceImpl {
    pub fn new(store: PgStore, default_max_catchup: i32) -> Self {
        Self {
            store,
            default_max_catchup,
        }
    }
}

#[tonic::async_trait]
impl WorkflowScheduleService for WorkflowScheduleServiceImpl {
    async fn create_workflow_schedule(
        &self,
        request: Request<CreateWorkflowScheduleRequest>,
    ) -> Result<Response<CreateWorkflowScheduleResponse>, Status> {
        let req = request.into_inner();

        let task_queue = require_non_empty(req.task_queue, "task_queue")?;
        let workflow_type = require_non_empty(req.workflow_type, "workflow_type")?;
        let cron_expr = require_non_empty(req.cron_expr, "cron_expr")?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let input = payload_to_optional_bytes(req.input).unwrap_or_default();

        let first_fire = next_fire_from_now(&cron_expr, Utc::now())?;

        let run_id = Uuid::now_v7();

        self.store
            .workflows()
            .create(CreateWorkflow {
                run_id,
                external_id: format!("schedule-{}", run_id),
                task_queue,
                workflow_type,
                input,
                namespace_id: namespace_id.clone(),
                version: req.version.unwrap_or_default(),
                retry_policy: None,
                cron_expr: Some(cron_expr),
                schedule_id: None,
            })
            .await
            .map_err(map_store_error)?;

        // Update the workflow to SCHEDULED status and set first fire time
        let mut template = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| invalid_argument_error("Failed to create schedule"))?;

        template.status = kagzi_store::WorkflowStatus::Scheduled;
        template.available_at = Some(first_fire);
        template.max_catchup = req.max_catchup.unwrap_or(self.default_max_catchup);

        self.store
            .workflows()
            .update(run_id, template.clone())
            .await
            .map_err(map_store_error)?;

        let schedule = workflow_run_to_schedule_proto(template)?;

        Ok(Response::new(CreateWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn get_workflow_schedule(
        &self,
        request: Request<GetWorkflowScheduleRequest>,
    ) -> Result<Response<GetWorkflowScheduleResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument_error("Invalid schedule_id"))?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let schedule = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .map(workflow_run_to_schedule_proto)
            .transpose()?
            .ok_or_else(|| not_found_error("Schedule not found", "schedule", req.schedule_id))?;

        Ok(Response::new(GetWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn list_workflow_schedules(
        &self,
        request: Request<ListWorkflowSchedulesRequest>,
    ) -> Result<Response<ListWorkflowSchedulesResponse>, Status> {
        let req = request.into_inner();
        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let page_size = req
            .page
            .as_ref()
            .map(|p| normalize_page_size(p.page_size, 100, 500))
            .unwrap_or(100);

        let cursor = if let Some(page) = req.page.as_ref() {
            if page.page_token.is_empty() {
                None
            } else {
                let (created_at, run_id) = decode_cursor(&page.page_token)?;
                Some(kagzi_store::WorkflowCursor { created_at, run_id })
            }
        } else {
            None
        };

        let workflows_result = self
            .store
            .workflows()
            .list(ListWorkflowsParams {
                namespace_id: namespace_id.clone(),
                filter_status: Some("SCHEDULED".to_string()),
                page_size,
                cursor,
                schedule_id: None,
            })
            .await
            .map_err(map_store_error)?;

        let mut proto_schedules = Vec::with_capacity(workflows_result.items.len());
        for w in workflows_result.items {
            proto_schedules.push(workflow_run_to_schedule_proto(w)?);
        }

        let next_page_token = workflows_result
            .next_cursor
            .map(|c| encode_cursor(c.created_at.timestamp_millis(), &c.run_id))
            .unwrap_or_default();

        Ok(Response::new(ListWorkflowSchedulesResponse {
            schedules: proto_schedules,
            page: Some(PageInfo {
                next_page_token,
                has_more: workflows_result.has_more,
                total_count: 0,
            }),
        }))
    }

    async fn update_workflow_schedule(
        &self,
        request: Request<UpdateWorkflowScheduleRequest>,
    ) -> Result<Response<UpdateWorkflowScheduleResponse>, Status> {
        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument_error("Invalid schedule_id"))?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let current = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                not_found_error("Schedule not found", "schedule", req.schedule_id.clone())
            })?;

        let cron_expr = req.cron_expr.clone();
        let parsed_cron = if let Some(ref expr) = cron_expr {
            Some(parse_cron_expr(expr)?)
        } else {
            None
        };

        // Validate cron if provided but we'll recalculate next_fire when enabling
        if let Some(ref cron) = parsed_cron {
            cron.after(&Utc::now()).next().ok_or_else(|| {
                invalid_argument_error("Cron expression has no future occurrences")
            })?;
        }

        let new_status = match req.enabled {
            Some(true) => Some(kagzi_store::WorkflowStatus::Scheduled),
            Some(false) => Some(kagzi_store::WorkflowStatus::Paused),
            None => None,
        };

        let next_fire = if new_status == Some(kagzi_store::WorkflowStatus::Scheduled) {
            let cron_expr = current.cron_expr.as_ref().ok_or_else(|| {
                invalid_argument_error("Cannot enable schedule without cron expression")
            })?;
            let cron = parse_cron_expr(cron_expr)?;
            Some(cron.after(&Utc::now()).next().ok_or_else(|| {
                invalid_argument_error("Cron expression has no future occurrences")
            })?)
        } else {
            None
        };

        if let Some(status) = new_status {
            let mut wf = current.clone();
            wf.status = status;
            if let Some(fire) = next_fire {
                wf.available_at = Some(fire);
            }
            self.store
                .workflows()
                .update(run_id, wf)
                .await
                .map_err(map_store_error)?;
        }

        let schedule = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .map(workflow_run_to_schedule_proto)
            .transpose()?
            .ok_or_else(|| not_found_error("Schedule not found", "schedule", run_id.to_string()))?;

        Ok(Response::new(UpdateWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn delete_workflow_schedule(
        &self,
        request: Request<DeleteWorkflowScheduleRequest>,
    ) -> Result<Response<DeleteWorkflowScheduleResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument_error("Invalid schedule_id"))?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let result = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if result.is_none() {
            return Err(not_found_error(
                "Schedule not found",
                "schedule",
                req.schedule_id,
            ));
        }

        self.store
            .workflows()
            .delete(run_id)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(DeleteWorkflowScheduleResponse {
            deleted: true,
        }))
    }
}
