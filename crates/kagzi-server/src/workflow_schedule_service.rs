use std::str::FromStr;

use base64::Engine;
use chrono::{TimeZone, Utc};
use kagzi_proto::kagzi::workflow_schedule_service_server::WorkflowScheduleService;
use kagzi_proto::kagzi::{
    CreateWorkflowScheduleRequest, CreateWorkflowScheduleResponse, DeleteWorkflowScheduleRequest,
    DeleteWorkflowScheduleResponse, GetWorkflowScheduleRequest, GetWorkflowScheduleResponse,
    ListWorkflowSchedulesRequest, ListWorkflowSchedulesResponse, PageInfo,
    UpdateWorkflowScheduleRequest, UpdateWorkflowScheduleResponse, WorkflowSchedule,
};
use kagzi_store::{
    CreateSchedule as StoreCreateSchedule, ListSchedulesParams, PgStore,
    UpdateSchedule as StoreUpdateSchedule, WorkflowScheduleRepository, clamp_max_catchup,
};
use tonic::{Request, Response, Status};

use crate::helpers::{
    bytes_to_payload, invalid_argument, map_store_error, not_found, payload_to_optional_bytes,
};
use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};

fn parse_cron_expr(expr: &str) -> Result<cron::Schedule, Status> {
    if expr.trim().is_empty() {
        return Err(invalid_argument("cron_expr cannot be empty"));
    }
    cron::Schedule::from_str(expr).map_err(|e| invalid_argument(format!("Invalid cron: {}", e)))
}

fn next_fire_from_now(
    cron_expr: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>, Status> {
    let schedule = parse_cron_expr(cron_expr)?;
    let next = schedule
        .after(&now)
        .next()
        .ok_or_else(|| invalid_argument("Cron expression has no future occurrences"))?;
    Ok(next.with_timezone(&chrono::Utc))
}

fn to_proto_timestamp(ts: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: ts.timestamp(),
        nanos: ts.timestamp_subsec_nanos() as i32,
    }
}

fn workflow_schedule_to_proto(s: kagzi_store::Schedule) -> Result<WorkflowSchedule, Status> {
    let input = bytes_to_payload(Some(s.input));

    Ok(WorkflowSchedule {
        schedule_id: s.schedule_id.to_string(),
        namespace_id: s.namespace_id,
        task_queue: s.task_queue,
        workflow_type: s.workflow_type,
        cron_expr: s.cron_expr,
        input: Some(input),
        enabled: s.enabled,
        max_catchup: s.max_catchup,
        next_fire_at: Some(to_proto_timestamp(s.next_fire_at)),
        last_fired_at: s.last_fired_at.map(to_proto_timestamp),
        version: s.version.unwrap_or_default(),
        created_at: Some(to_proto_timestamp(s.created_at)),
        updated_at: Some(to_proto_timestamp(s.updated_at)),
    })
}

#[derive(Clone)]
pub struct WorkflowScheduleServiceImpl {
    pub store: PgStore,
}

impl WorkflowScheduleServiceImpl {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl WorkflowScheduleService for WorkflowScheduleServiceImpl {
    async fn create_workflow_schedule(
        &self,
        request: Request<CreateWorkflowScheduleRequest>,
    ) -> Result<Response<CreateWorkflowScheduleResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("CreateWorkflowSchedule", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        if req.task_queue.is_empty() {
            return Err(invalid_argument("task_queue is required"));
        }
        if req.workflow_type.is_empty() {
            return Err(invalid_argument("workflow_type is required"));
        }
        if req.cron_expr.is_empty() {
            return Err(invalid_argument("cron_expr is required"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let input = payload_to_optional_bytes(req.input).unwrap_or_default();
        // Context is not used - user payloads treated as opaque bytes

        let enabled = req.enabled.unwrap_or(true);
        let max_catchup = if let Some(m) = req.max_catchup {
            clamp_max_catchup(m)
        } else {
            100
        };

        let next_fire_at = next_fire_from_now(&req.cron_expr, Utc::now())?;

        let schedule_id = self
            .store
            .schedules()
            .create(StoreCreateSchedule {
                namespace_id: namespace_id.clone(),
                task_queue: req.task_queue,
                workflow_type: req.workflow_type,
                cron_expr: req.cron_expr,
                input,
                enabled,
                max_catchup,
                next_fire_at,
                version: req.version,
            })
            .await
            .map_err(map_store_error)?;

        let schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .map(workflow_schedule_to_proto)
            .transpose()?
            .ok_or_else(|| not_found("Schedule not found", "schedule", schedule_id.to_string()))?;

        log_grpc_response(
            "CreateWorkflowSchedule",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(CreateWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn get_workflow_schedule(
        &self,
        request: Request<GetWorkflowScheduleRequest>,
    ) -> Result<Response<GetWorkflowScheduleResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("GetWorkflowSchedule", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let schedule_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument("Invalid schedule_id"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .map(workflow_schedule_to_proto)
            .transpose()?
            .ok_or_else(|| not_found("Schedule not found", "schedule", req.schedule_id))?;

        log_grpc_response(
            "GetWorkflowSchedule",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(GetWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn list_workflow_schedules(
        &self,
        request: Request<ListWorkflowSchedulesRequest>,
    ) -> Result<Response<ListWorkflowSchedulesResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("ListWorkflowSchedules", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let page_size = req
            .page
            .as_ref()
            .and_then(|p| {
                if p.page_size <= 0 {
                    None
                } else {
                    Some(p.page_size)
                }
            })
            .unwrap_or(100)
            .min(500);

        let cursor = if let Some(page) = req.page.as_ref() {
            if page.page_token.is_empty() {
                None
            } else {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&page.page_token)
                    .map_err(|_| invalid_argument("Invalid page_token"))?;
                let token_str = std::str::from_utf8(&decoded)
                    .map_err(|_| invalid_argument("Invalid page_token"))?;

                let mut parts = token_str.splitn(2, ':');
                let created_at_ms = parts
                    .next()
                    .and_then(|p| p.parse::<i64>().ok())
                    .ok_or_else(|| invalid_argument("Invalid page_token"))?;
                let schedule_id_str = parts
                    .next()
                    .ok_or_else(|| invalid_argument("Invalid page_token"))?;

                let created_at = Utc
                    .timestamp_millis_opt(created_at_ms)
                    .single()
                    .ok_or_else(|| invalid_argument("Invalid page_token"))?;
                let schedule_id = uuid::Uuid::parse_str(schedule_id_str)
                    .map_err(|_| invalid_argument("Invalid page_token"))?;

                Some(kagzi_store::ScheduleCursor {
                    created_at,
                    schedule_id,
                })
            }
        } else {
            None
        };

        let schedules_result = self
            .store
            .schedules()
            .list(ListSchedulesParams {
                namespace_id: namespace_id.clone(),
                task_queue: req.task_queue,
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        let mut proto_schedules = Vec::with_capacity(schedules_result.items.len());
        for s in schedules_result.items {
            proto_schedules.push(workflow_schedule_to_proto(s)?);
        }

        let next_page_token = schedules_result
            .next_cursor
            .map(|c| {
                let cursor_str = format!("{}:{}", c.created_at.timestamp_millis(), c.schedule_id);
                base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
            })
            .unwrap_or_default();

        log_grpc_response(
            "ListWorkflowSchedules",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(ListWorkflowSchedulesResponse {
            schedules: proto_schedules,
            page: Some(PageInfo {
                next_page_token,
                has_more: schedules_result.has_more,
                total_count: 0, // TODO: populate total_count when supported
            }),
        }))
    }

    async fn update_workflow_schedule(
        &self,
        request: Request<UpdateWorkflowScheduleRequest>,
    ) -> Result<Response<UpdateWorkflowScheduleResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("UpdateWorkflowSchedule", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let schedule_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument("Invalid schedule_id"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let current_schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found("Schedule not found", "schedule", req.schedule_id.clone()))?;

        let cron_expr = req.cron_expr.clone();
        let parsed_cron = if let Some(ref expr) = cron_expr {
            Some(parse_cron_expr(expr)?)
        } else {
            None
        };

        let next_fire_at = if let Some(cron) = parsed_cron.as_ref() {
            let candidate = cron
                .after(&Utc::now())
                .next()
                .ok_or_else(|| invalid_argument("Cron expression has no future occurrences"))?
                .with_timezone(&Utc);

            // If the candidate matches the current scheduled time (e.g., update happens
            // at/near the scheduled fire time), advance to the next cron occurrence to
            // avoid re-triggering the same scheduled execution.
            if candidate == current_schedule.next_fire_at {
                cron.after(&candidate)
                    .next()
                    .map(|dt| dt.with_timezone(&Utc))
            } else {
                Some(candidate)
            }
        } else if let Some(ts) = req.next_fire_at {
            let dt = chrono::DateTime::<Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .ok_or_else(|| invalid_argument("next_fire_at is out of supported range"))?;
            Some(dt)
        } else {
            None
        };

        let input = payload_to_optional_bytes(req.input);
        // Context is not used - user payloads treated as opaque bytes
        let max_catchup = req.max_catchup.map(clamp_max_catchup);

        self.store
            .schedules()
            .update(
                schedule_id,
                &namespace_id,
                StoreUpdateSchedule {
                    task_queue: req.task_queue,
                    workflow_type: req.workflow_type,
                    cron_expr,
                    input,
                    enabled: req.enabled,
                    max_catchup,
                    next_fire_at,
                    version: req.version,
                },
            )
            .await
            .map_err(map_store_error)?;

        let schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .map(workflow_schedule_to_proto)
            .transpose()?
            .ok_or_else(|| not_found("Schedule not found", "schedule", schedule_id.to_string()))?;

        log_grpc_response(
            "UpdateWorkflowSchedule",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(UpdateWorkflowScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    async fn delete_workflow_schedule(
        &self,
        request: Request<DeleteWorkflowScheduleRequest>,
    ) -> Result<Response<DeleteWorkflowScheduleResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("DeleteWorkflowSchedule", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let schedule_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument("Invalid schedule_id"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let deleted = self
            .store
            .schedules()
            .delete(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if !deleted {
            return Err(not_found("Schedule not found", "schedule", req.schedule_id));
        }

        log_grpc_response(
            "DeleteWorkflowSchedule",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(DeleteWorkflowScheduleResponse { deleted }))
    }
}
