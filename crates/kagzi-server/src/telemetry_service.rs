use chrono::TimeZone;
use kagzi_proto::kagzi::telemetry_service_server::TelemetryService;
use kagzi_proto::kagzi::{
    GetQueueTelemetryStateRequest, GetQueueTelemetryStateResponse, GetWorkerTelemetryStateRequest,
    GetWorkerTelemetryStateResponse, ListQueueTelemetryStatesRequest,
    ListQueueTelemetryStatesResponse, ListServerTelemetryEventsRequest,
    ListServerTelemetryEventsResponse, ListWorkerTelemetryEventsRequest,
    ListWorkerTelemetryEventsResponse, ListWorkerTelemetryStatesRequest,
    ListWorkerTelemetryStatesResponse, PageInfo, QueueTelemetryState, ReportWorkerEventsRequest,
    ReportWorkerEventsResponse, ReportWorkerSnapshotRequest, ReportWorkerSnapshotResponse,
    ServerTelemetryEvent, TelemetryLevel, WorkerTelemetryEvent, WorkerTelemetrySnapshot,
};
use kagzi_store::{PgStore, WorkerRepository, WorkerStatus as StoreWorkerStatus};
use sqlx::{QueryBuilder, Row};
use tonic::{Request, Response, Status};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::helpers::{
    decode_cursor, decode_cursor_str, encode_cursor, encode_cursor_str, invalid_argument_error,
    map_store_error, normalize_page_size, not_found_error, precondition_failed_error,
    require_non_empty,
};
use crate::proto_convert::timestamp_from;

#[derive(Clone)]
pub struct TelemetryServiceImpl {
    pub store: PgStore,
    pub enabled: bool,
    pub max_events_per_report: usize,
}

impl TelemetryServiceImpl {
    pub fn new(store: PgStore, enabled: bool, max_events_per_report: usize) -> Self {
        Self {
            store,
            enabled,
            max_events_per_report,
        }
    }

    fn require_enabled(&self) -> Result<(), Status> {
        if !self.enabled {
            return Err(precondition_failed_error("Worker telemetry is disabled"));
        }
        Ok(())
    }

    fn parse_extra_json(extra: &[u8]) -> Result<serde_json::Value, Status> {
        if extra.is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_slice(extra)
            .map_err(|_| invalid_argument_error("extra_json must be valid JSON"))
    }

    async fn validate_worker_identity(
        &self,
        worker_id: Uuid,
        namespace: &str,
        task_queue: &str,
    ) -> Result<(), Status> {
        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Worker not found", "worker", worker_id.to_string()))?;

        if worker.status == StoreWorkerStatus::Offline {
            return Err(precondition_failed_error(
                "Worker is offline; telemetry is only accepted for online/draining workers",
            ));
        }

        if worker.namespace != namespace || worker.task_queue != task_queue {
            return Err(precondition_failed_error(
                "Worker not registered for the requested namespace/task_queue",
            ));
        }

        Ok(())
    }

    fn normalize_level(level: i32) -> &'static str {
        match TelemetryLevel::try_from(level).unwrap_or(TelemetryLevel::Unspecified) {
            TelemetryLevel::Debug => "debug",
            TelemetryLevel::Info => "info",
            TelemetryLevel::Warn => "warn",
            TelemetryLevel::Error => "error",
            TelemetryLevel::Unspecified => "info",
        }
    }

    async fn upsert_state(&self, snapshot: &WorkerTelemetrySnapshot) -> Result<(), Status> {
        let worker_id = Uuid::parse_str(&snapshot.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let namespace = require_non_empty(snapshot.namespace.clone(), "namespace")?;
        let task_queue = require_non_empty(snapshot.task_queue.clone(), "task_queue")?;

        self.validate_worker_identity(worker_id, &namespace, &task_queue)
            .await?;

        let extra = Self::parse_extra_json(&snapshot.extra_json)?;

        let last_subscribe_ok_at = snapshot.last_subscribe_ok_at.as_ref().map(|t| {
            chrono::Utc
                .timestamp_opt(t.seconds, t.nanos as u32)
                .single()
        });
        let last_wakeup_at = snapshot.last_wakeup_at.as_ref().map(|t| {
            chrono::Utc
                .timestamp_opt(t.seconds, t.nanos as u32)
                .single()
        });
        let last_error_at = snapshot.last_error_at.as_ref().map(|t| {
            chrono::Utc
                .timestamp_opt(t.seconds, t.nanos as u32)
                .single()
        });

        // NOTE: We set updated_at server-side for consistent "last seen" semantics.
        sqlx::query(
            r#"
            INSERT INTO kagzi.worker_telemetry_state (
                worker_id, namespace, task_queue,
                signal_backend, subscribed, subscription_state,
                last_subscribe_ok_at, last_wakeup_at,
                max_concurrent, in_flight,
                last_error, last_error_at,
                updated_at, extra
            )
            VALUES (
                $1, $2, $3,
                $4, $5, $6,
                $7, $8,
                $9, $10,
                $11, $12,
                NOW(), $13
            )
            ON CONFLICT (worker_id) DO UPDATE SET
                namespace = EXCLUDED.namespace,
                task_queue = EXCLUDED.task_queue,
                signal_backend = EXCLUDED.signal_backend,
                subscribed = EXCLUDED.subscribed,
                subscription_state = EXCLUDED.subscription_state,
                last_subscribe_ok_at = EXCLUDED.last_subscribe_ok_at,
                last_wakeup_at = EXCLUDED.last_wakeup_at,
                max_concurrent = EXCLUDED.max_concurrent,
                in_flight = EXCLUDED.in_flight,
                last_error = EXCLUDED.last_error,
                last_error_at = EXCLUDED.last_error_at,
                updated_at = NOW(),
                extra = EXCLUDED.extra
            "#,
        )
        .bind(worker_id)
        .bind(namespace)
        .bind(task_queue)
        .bind(&snapshot.signal_backend)
        .bind(snapshot.subscribed)
        .bind(&snapshot.subscription_state)
        .bind(last_subscribe_ok_at.flatten())
        .bind(last_wakeup_at.flatten())
        .bind(snapshot.max_concurrent as i32)
        .bind(snapshot.in_flight as i32)
        .bind(&snapshot.last_error)
        .bind(last_error_at.flatten())
        .bind(extra)
        .execute(self.store.pool())
        .await
        .map_err(|e| map_store_error(e.into()))?;

        Ok(())
    }

    fn row_to_snapshot(row: &sqlx::postgres::PgRow) -> Result<WorkerTelemetrySnapshot, Status> {
        let worker_id: Uuid = row
            .try_get("worker_id")
            .map_err(|e| map_store_error(e.into()))?;
        let namespace: String = row
            .try_get("namespace")
            .map_err(|e| map_store_error(e.into()))?;
        let task_queue: String = row
            .try_get("task_queue")
            .map_err(|e| map_store_error(e.into()))?;
        let signal_backend: String = row
            .try_get("signal_backend")
            .map_err(|e| map_store_error(e.into()))?;
        let subscribed: bool = row
            .try_get("subscribed")
            .map_err(|e| map_store_error(e.into()))?;
        let subscription_state: String = row
            .try_get("subscription_state")
            .map_err(|e| map_store_error(e.into()))?;
        let last_subscribe_ok_at: Option<chrono::DateTime<chrono::Utc>> = row
            .try_get("last_subscribe_ok_at")
            .map_err(|e| map_store_error(e.into()))?;
        let last_wakeup_at: Option<chrono::DateTime<chrono::Utc>> =
            row.try_get("last_wakeup_at")
                .map_err(|e| map_store_error(e.into()))?;
        let max_concurrent: i32 = row
            .try_get("max_concurrent")
            .map_err(|e| map_store_error(e.into()))?;
        let in_flight: i32 = row
            .try_get("in_flight")
            .map_err(|e| map_store_error(e.into()))?;
        let last_error: String = row
            .try_get("last_error")
            .map_err(|e| map_store_error(e.into()))?;
        let last_error_at: Option<chrono::DateTime<chrono::Utc>> = row
            .try_get("last_error_at")
            .map_err(|e| map_store_error(e.into()))?;
        let extra: serde_json::Value = row
            .try_get("extra")
            .map_err(|e| map_store_error(e.into()))?;
        let active_workflows_authoritative: i32 = row
            .try_get("active_workflows_authoritative")
            .map_err(|e| map_store_error(e.into()))?;
        let last_claim_at: Option<chrono::DateTime<chrono::Utc>> = row
            .try_get("last_claim_at")
            .map_err(|e| map_store_error(e.into()))?;
        let last_claim_result: String = row
            .try_get("last_claim_result")
            .map_err(|e| map_store_error(e.into()))?;
        let last_claim_error: String = row
            .try_get("last_claim_error")
            .map_err(|e| map_store_error(e.into()))?;

        Ok(WorkerTelemetrySnapshot {
            worker_id: worker_id.to_string(),
            namespace,
            task_queue,
            observed_at: None,
            signal_backend,
            subscribed,
            subscription_state,
            last_subscribe_ok_at: last_subscribe_ok_at.map(timestamp_from),
            last_wakeup_at: last_wakeup_at.map(timestamp_from),
            last_error,
            last_error_at: last_error_at.map(timestamp_from),
            max_concurrent: max_concurrent.max(0) as u32,
            in_flight: in_flight.max(0) as u32,
            active_workflows_authoritative: active_workflows_authoritative.max(0) as u32,
            last_claim_at: last_claim_at.map(timestamp_from),
            last_claim_result,
            last_claim_error,
            claim_attempts_delta: 0,
            claim_success_delta: 0,
            claim_no_task_delta: 0,
            claim_error_delta: 0,
            claim_rtt_ms_avg: 0.0,
            claim_rtt_ms_p95: 0.0,
            claim_rtt_ms_max: 0.0,
            extra_json: serde_json::to_vec(&extra).unwrap_or_default(),
        })
    }

    fn row_to_queue_state(row: &sqlx::postgres::PgRow) -> Result<QueueTelemetryState, Status> {
        let namespace: String = row
            .try_get("namespace")
            .map_err(|e| map_store_error(e.into()))?;
        let task_queue: String = row
            .try_get("task_queue")
            .map_err(|e| map_store_error(e.into()))?;
        let updated_at: chrono::DateTime<chrono::Utc> = row
            .try_get("updated_at")
            .map_err(|e| map_store_error(e.into()))?;

        let pending_count: i64 = row
            .try_get("pending_count")
            .map_err(|e| map_store_error(e.into()))?;
        let sleeping_count: i64 = row
            .try_get("sleeping_count")
            .map_err(|e| map_store_error(e.into()))?;
        let running_count: i64 = row
            .try_get("running_count")
            .map_err(|e| map_store_error(e.into()))?;
        let due_count: i64 = row
            .try_get("due_count")
            .map_err(|e| map_store_error(e.into()))?;

        let publish_attempts: i64 = row
            .try_get("publish_attempts")
            .map_err(|e| map_store_error(e.into()))?;
        let publish_errors: i64 = row
            .try_get("publish_errors")
            .map_err(|e| map_store_error(e.into()))?;
        let last_publish_ok_at: Option<chrono::DateTime<chrono::Utc>> = row
            .try_get("last_publish_ok_at")
            .map_err(|e| map_store_error(e.into()))?;
        let last_publish_error_at: Option<chrono::DateTime<chrono::Utc>> = row
            .try_get("last_publish_error_at")
            .map_err(|e| map_store_error(e.into()))?;
        let last_publish_error: String = row
            .try_get("last_publish_error")
            .map_err(|e| map_store_error(e.into()))?;

        let last_due_work_notified_at: Option<chrono::DateTime<chrono::Utc>> = row
            .try_get("last_due_work_notified_at")
            .map_err(|e| map_store_error(e.into()))?;

        let extra: serde_json::Value = row
            .try_get("extra")
            .map_err(|e| map_store_error(e.into()))?;

        Ok(QueueTelemetryState {
            namespace,
            task_queue,
            updated_at: Some(timestamp_from(updated_at)),
            pending_count,
            sleeping_count,
            running_count,
            due_count,
            publish_attempts,
            publish_errors,
            last_publish_ok_at: last_publish_ok_at.map(timestamp_from),
            last_publish_error_at: last_publish_error_at.map(timestamp_from),
            last_publish_error,
            last_due_work_notified_at: last_due_work_notified_at.map(timestamp_from),
            extra_json: serde_json::to_vec(&extra).unwrap_or_default(),
        })
    }

    fn level_from_str(level: &str) -> i32 {
        match level {
            "debug" => TelemetryLevel::Debug as i32,
            "warn" => TelemetryLevel::Warn as i32,
            "error" => TelemetryLevel::Error as i32,
            _ => TelemetryLevel::Info as i32,
        }
    }
}

#[tonic::async_trait]
impl TelemetryService for TelemetryServiceImpl {
    #[instrument(skip(self, request), fields(worker_id = %request.get_ref().snapshot.as_ref().map(|s| s.worker_id.clone()).unwrap_or_default()))]
    async fn report_worker_snapshot(
        &self,
        request: Request<ReportWorkerSnapshotRequest>,
    ) -> Result<Response<ReportWorkerSnapshotResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let snapshot = req
            .snapshot
            .ok_or_else(|| invalid_argument_error("snapshot is required"))?;

        self.upsert_state(&snapshot).await?;

        Ok(Response::new(ReportWorkerSnapshotResponse {
            accepted: true,
            server_time: Some(timestamp_from(chrono::Utc::now())),
        }))
    }

    #[instrument(skip(self, request), fields(count = request.get_ref().events.len()))]
    async fn report_worker_events(
        &self,
        request: Request<ReportWorkerEventsRequest>,
    ) -> Result<Response<ReportWorkerEventsResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        if req.events.is_empty() {
            return Ok(Response::new(ReportWorkerEventsResponse {
                accepted: 0,
                dropped: 0,
                server_time: Some(timestamp_from(chrono::Utc::now())),
            }));
        }

        // Enforce single worker_id per batch for simplicity and to avoid partial writes.
        let first_worker_id = req.events[0].worker_id.clone();
        if req.events.iter().any(|e| e.worker_id != first_worker_id) {
            return Err(invalid_argument_error(
                "ReportWorkerEvents must contain events for a single worker_id",
            ));
        }

        let worker_id = Uuid::parse_str(&first_worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let namespace = require_non_empty(req.events[0].namespace.clone(), "namespace")?;
        let task_queue = require_non_empty(req.events[0].task_queue.clone(), "task_queue")?;
        self.validate_worker_identity(worker_id, &namespace, &task_queue)
            .await?;

        let max = self.max_events_per_report.max(1);
        let (accepted_events, dropped) = if req.events.len() > max {
            (req.events[..max].to_vec(), (req.events.len() - max) as u32)
        } else {
            (req.events, 0)
        };

        let mut builder = QueryBuilder::new(
            "INSERT INTO kagzi.worker_telemetry_events (worker_id, namespace, task_queue, occurred_at, level, event_type, message, extra) ",
        );
        builder.push_values(accepted_events.iter(), |mut b, e| {
            let extra = match Self::parse_extra_json(&e.extra_json) {
                Ok(v) => v,
                Err(err) => {
                    warn!(error = %err, "Invalid extra_json in worker event; storing empty object");
                    serde_json::json!({})
                }
            };

            let occurred_at = e.occurred_at.as_ref().and_then(|t| {
                chrono::Utc
                    .timestamp_opt(t.seconds, t.nanos as u32)
                    .single()
            });

            b.push_bind(worker_id)
                .push_bind(&namespace)
                .push_bind(&task_queue)
                .push_bind(occurred_at.unwrap_or_else(chrono::Utc::now))
                .push_bind(Self::normalize_level(e.level))
                .push_bind(&e.event_type)
                .push_bind(&e.message)
                .push_bind(extra);
        });

        builder
            .build()
            .execute(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;

        Ok(Response::new(ReportWorkerEventsResponse {
            accepted: accepted_events.len() as u32,
            dropped,
            server_time: Some(timestamp_from(chrono::Utc::now())),
        }))
    }

    async fn get_worker_telemetry_state(
        &self,
        request: Request<GetWorkerTelemetryStateRequest>,
    ) -> Result<Response<GetWorkerTelemetryStateResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let row = sqlx::query(
            r#"
            SELECT
                worker_id, namespace, task_queue,
                signal_backend, subscribed, subscription_state,
                last_subscribe_ok_at, last_wakeup_at,
                max_concurrent, in_flight,
                last_error, last_error_at,
                active_workflows_authoritative,
                last_claim_at, last_claim_result, last_claim_error,
                updated_at, extra
            FROM kagzi.worker_telemetry_state
            WHERE worker_id = $1
            "#,
        )
        .bind(worker_id)
        .fetch_optional(self.store.pool())
        .await
        .map_err(|e| map_store_error(e.into()))?;

        let row = row.ok_or_else(|| {
            not_found_error("Worker telemetry not found", "worker", req.worker_id)
        })?;
        let updated_at: chrono::DateTime<chrono::Utc> = row
            .try_get("updated_at")
            .map_err(|e| map_store_error(e.into()))?;

        let snapshot = Self::row_to_snapshot(&row)?;

        Ok(Response::new(GetWorkerTelemetryStateResponse {
            snapshot: Some(snapshot),
            updated_at: Some(timestamp_from(updated_at)),
        }))
    }

    async fn list_worker_telemetry_states(
        &self,
        request: Request<ListWorkerTelemetryStatesRequest>,
    ) -> Result<Response<ListWorkerTelemetryStatesResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let page = req.page.unwrap_or_default();

        let namespace = require_non_empty(req.namespace, "namespace")?;
        let page_size = normalize_page_size(page.page_size, 50, 200) as i64;
        let limit = page_size + 1;

        let task_queue = req.task_queue.filter(|t| !t.is_empty());

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(decode_cursor(&page.page_token)?)
        };

        let mut builder = QueryBuilder::new(
            r#"
            SELECT
                worker_id, namespace, task_queue,
                signal_backend, subscribed, subscription_state,
                last_subscribe_ok_at, last_wakeup_at,
                max_concurrent, in_flight,
                last_error, last_error_at,
                active_workflows_authoritative,
                last_claim_at, last_claim_result, last_claim_error,
                updated_at, extra
            FROM kagzi.worker_telemetry_state
            WHERE namespace = 
            "#,
        );
        builder.push_bind(&namespace);
        if let Some(tq) = &task_queue {
            builder.push(" AND task_queue = ").push_bind(tq);
        }
        if let Some((ts, id)) = &cursor {
            builder
                .push(" AND (updated_at, worker_id) < (")
                .push_bind(ts)
                .push(", ")
                .push_bind(id)
                .push(")");
        }
        builder.push(" ORDER BY updated_at DESC, worker_id DESC LIMIT ");
        builder.push_bind(limit);

        let rows = builder
            .build()
            .fetch_all(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;

        if rows.is_empty() {
            return Ok(Response::new(ListWorkerTelemetryStatesResponse {
                snapshots: Vec::new(),
                page: Some(PageInfo {
                    next_page_token: "".to_string(),
                    has_more: false,
                    total_count: 0,
                }),
            }));
        }

        let has_more = rows.len() as i64 > page_size;
        let mut snapshots = Vec::with_capacity(rows.len().min(page_size as usize));
        let mut next_cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = None;
        for row in rows.into_iter().take(page_size as usize) {
            let updated_at: chrono::DateTime<chrono::Utc> = row
                .try_get("updated_at")
                .map_err(|e| map_store_error(e.into()))?;
            let worker_id: Uuid = row
                .try_get("worker_id")
                .map_err(|e| map_store_error(e.into()))?;
            let snapshot = Self::row_to_snapshot(&row)?;
            snapshots.push(snapshot);
            next_cursor = Some((updated_at, worker_id));
        }

        let next_page_token = if has_more {
            let (ts, id) = next_cursor.expect("cursor set when snapshots non-empty");
            encode_cursor(ts.timestamp_millis(), &id)
        } else {
            "".to_string()
        };

        let total_count = if page.include_total_count {
            let row = sqlx::query(
                r#"
                SELECT COUNT(*) as count
                FROM kagzi.worker_telemetry_state
                WHERE namespace = $1
                  AND ($2::TEXT IS NULL OR task_queue = $2)
                "#,
            )
            .bind(&namespace)
            .bind(&task_queue)
            .fetch_one(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;
            let count: i64 = row
                .try_get::<i64, _>("count")
                .map_err(|e| map_store_error(e.into()))?;
            count
        } else {
            0
        };

        Ok(Response::new(ListWorkerTelemetryStatesResponse {
            snapshots,
            page: Some(PageInfo {
                next_page_token,
                has_more,
                total_count,
            }),
        }))
    }

    async fn list_worker_telemetry_events(
        &self,
        request: Request<ListWorkerTelemetryEventsRequest>,
    ) -> Result<Response<ListWorkerTelemetryEventsResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let page = req.page.unwrap_or_default();

        let namespace = require_non_empty(req.namespace, "namespace")?;
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let page_size = normalize_page_size(page.page_size, 50, 200) as i64;
        let limit = page_size + 1;

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(decode_cursor(&page.page_token)?)
        };

        let mut builder = QueryBuilder::new(
            r#"
            SELECT event_id, worker_id, namespace, task_queue, occurred_at, level, event_type, message, extra
            FROM kagzi.worker_telemetry_events
            WHERE namespace =
            "#,
        );
        builder.push_bind(&namespace);
        builder.push(" AND worker_id = ").push_bind(worker_id);
        if let Some((ts, id)) = &cursor {
            builder
                .push(" AND (occurred_at, event_id) < (")
                .push_bind(ts)
                .push(", ")
                .push_bind(id)
                .push(")");
        }
        builder.push(" ORDER BY occurred_at DESC, event_id DESC LIMIT ");
        builder.push_bind(limit);

        let rows = builder
            .build()
            .fetch_all(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;

        let has_more = rows.len() as i64 > page_size;
        let mut events = Vec::with_capacity(rows.len().min(page_size as usize));
        let mut next_cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = None;
        for row in rows.into_iter().take(page_size as usize) {
            let event_id: Uuid = row
                .try_get("event_id")
                .map_err(|e| map_store_error(e.into()))?;
            let task_queue: String = row
                .try_get("task_queue")
                .map_err(|e| map_store_error(e.into()))?;
            let occurred_at: chrono::DateTime<chrono::Utc> = row
                .try_get("occurred_at")
                .map_err(|e| map_store_error(e.into()))?;
            let level: String = row
                .try_get("level")
                .map_err(|e| map_store_error(e.into()))?;
            let event_type: String = row
                .try_get("event_type")
                .map_err(|e| map_store_error(e.into()))?;
            let message: String = row
                .try_get("message")
                .map_err(|e| map_store_error(e.into()))?;
            let extra: serde_json::Value = row
                .try_get("extra")
                .map_err(|e| map_store_error(e.into()))?;

            let level_i32 = match level.as_str() {
                "debug" => TelemetryLevel::Debug as i32,
                "warn" => TelemetryLevel::Warn as i32,
                "error" => TelemetryLevel::Error as i32,
                _ => TelemetryLevel::Info as i32,
            };

            events.push(WorkerTelemetryEvent {
                worker_id: worker_id.to_string(),
                namespace: namespace.clone(),
                task_queue,
                occurred_at: Some(timestamp_from(occurred_at)),
                level: level_i32,
                event_type,
                message,
                extra_json: serde_json::to_vec(&extra).unwrap_or_default(),
            });

            next_cursor = Some((occurred_at, event_id));
        }

        let next_page_token = if has_more {
            let (ts, id) = next_cursor.expect("cursor set when events non-empty");
            encode_cursor(ts.timestamp_millis(), &id)
        } else {
            "".to_string()
        };

        let total_count = if page.include_total_count {
            let row = sqlx::query(
                r#"
                SELECT COUNT(*) as count
                FROM kagzi.worker_telemetry_events
                WHERE namespace = $1 AND worker_id = $2
                "#,
            )
            .bind(&namespace)
            .bind(worker_id)
            .fetch_one(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;
            let count: i64 = row
                .try_get::<i64, _>("count")
                .map_err(|e| map_store_error(e.into()))?;
            count
        } else {
            0
        };

        Ok(Response::new(ListWorkerTelemetryEventsResponse {
            events,
            page: Some(PageInfo {
                next_page_token,
                has_more,
                total_count,
            }),
        }))
    }

    async fn get_queue_telemetry_state(
        &self,
        request: Request<GetQueueTelemetryStateRequest>,
    ) -> Result<Response<GetQueueTelemetryStateResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;

        let row = sqlx::query(
            r#"
            SELECT
                namespace, task_queue, updated_at,
                pending_count, sleeping_count, running_count, due_count,
                publish_attempts, publish_errors,
                last_publish_ok_at, last_publish_error_at, last_publish_error,
                last_due_work_notified_at,
                extra
            FROM kagzi.queue_telemetry_state
            WHERE namespace = $1 AND task_queue = $2
            "#,
        )
        .bind(&namespace)
        .bind(&task_queue)
        .fetch_optional(self.store.pool())
        .await
        .map_err(|e| map_store_error(e.into()))?;

        let row = row.ok_or_else(|| {
            not_found_error(
                "Queue telemetry not found",
                "task_queue",
                format!("{namespace}:{task_queue}"),
            )
        })?;

        let state = Self::row_to_queue_state(&row)?;
        Ok(Response::new(GetQueueTelemetryStateResponse {
            state: Some(state),
        }))
    }

    async fn list_queue_telemetry_states(
        &self,
        request: Request<ListQueueTelemetryStatesRequest>,
    ) -> Result<Response<ListQueueTelemetryStatesResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let page = req.page.unwrap_or_default();
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let page_size = normalize_page_size(page.page_size, 50, 200) as i64;
        let limit = page_size + 1;

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(decode_cursor_str(&page.page_token)?)
        };

        let mut builder = QueryBuilder::new(
            r#"
            SELECT
                namespace, task_queue, updated_at,
                pending_count, sleeping_count, running_count, due_count,
                publish_attempts, publish_errors,
                last_publish_ok_at, last_publish_error_at, last_publish_error,
                last_due_work_notified_at,
                extra
            FROM kagzi.queue_telemetry_state
            WHERE namespace =
            "#,
        );
        builder.push_bind(&namespace);
        if let Some((ts, tq)) = &cursor {
            builder
                .push(" AND (updated_at, task_queue) < (")
                .push_bind(ts)
                .push(", ")
                .push_bind(tq)
                .push(")");
        }
        builder.push(" ORDER BY updated_at DESC, task_queue DESC LIMIT ");
        builder.push_bind(limit);

        let rows = builder
            .build()
            .fetch_all(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;

        let has_more = rows.len() as i64 > page_size;
        let mut states = Vec::with_capacity(rows.len().min(page_size as usize));
        let mut next_cursor: Option<(chrono::DateTime<chrono::Utc>, String)> = None;
        for row in rows.into_iter().take(page_size as usize) {
            let updated_at: chrono::DateTime<chrono::Utc> = row
                .try_get("updated_at")
                .map_err(|e| map_store_error(e.into()))?;
            let task_queue: String = row
                .try_get("task_queue")
                .map_err(|e| map_store_error(e.into()))?;
            states.push(Self::row_to_queue_state(&row)?);
            next_cursor = Some((updated_at, task_queue));
        }

        let next_page_token = if has_more {
            let (ts, tq) = next_cursor.expect("cursor set when states non-empty");
            encode_cursor_str(ts.timestamp_millis(), &tq)
        } else {
            "".to_string()
        };

        let total_count = if page.include_total_count {
            let row = sqlx::query(
                r#"
                SELECT COUNT(*) as count
                FROM kagzi.queue_telemetry_state
                WHERE namespace = $1
                "#,
            )
            .bind(&namespace)
            .fetch_one(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;
            row.try_get::<i64, _>("count")
                .map_err(|e| map_store_error(e.into()))?
        } else {
            0
        };

        Ok(Response::new(ListQueueTelemetryStatesResponse {
            states,
            page: Some(PageInfo {
                next_page_token,
                has_more,
                total_count,
            }),
        }))
    }

    async fn list_server_telemetry_events(
        &self,
        request: Request<ListServerTelemetryEventsRequest>,
    ) -> Result<Response<ListServerTelemetryEventsResponse>, Status> {
        self.require_enabled()?;

        let req = request.into_inner();
        let page = req.page.unwrap_or_default();
        let page_size = normalize_page_size(page.page_size, 100, 500) as i64;
        let limit = page_size + 1;

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(decode_cursor(&page.page_token)?)
        };

        let namespace = req.namespace.filter(|n| !n.is_empty());

        let mut builder = QueryBuilder::new(
            r#"
            SELECT event_id, namespace, occurred_at, level, event_type, message, extra
            FROM kagzi.server_telemetry_events
            WHERE 1=1
            "#,
        );
        if let Some(ns) = &namespace {
            builder.push(" AND namespace = ").push_bind(ns);
        }
        if let Some((ts, id)) = &cursor {
            builder
                .push(" AND (occurred_at, event_id) < (")
                .push_bind(ts)
                .push(", ")
                .push_bind(id)
                .push(")");
        }
        builder.push(" ORDER BY occurred_at DESC, event_id DESC LIMIT ");
        builder.push_bind(limit);

        let rows = builder
            .build()
            .fetch_all(self.store.pool())
            .await
            .map_err(|e| map_store_error(e.into()))?;

        let has_more = rows.len() as i64 > page_size;
        let mut events = Vec::with_capacity(rows.len().min(page_size as usize));
        let mut next_cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = None;
        for row in rows.into_iter().take(page_size as usize) {
            let event_id: Uuid = row
                .try_get("event_id")
                .map_err(|e| map_store_error(e.into()))?;
            let ns: String = row
                .try_get("namespace")
                .map_err(|e| map_store_error(e.into()))?;
            let occurred_at: chrono::DateTime<chrono::Utc> = row
                .try_get("occurred_at")
                .map_err(|e| map_store_error(e.into()))?;
            let level: String = row
                .try_get("level")
                .map_err(|e| map_store_error(e.into()))?;
            let event_type: String = row
                .try_get("event_type")
                .map_err(|e| map_store_error(e.into()))?;
            let message: String = row
                .try_get("message")
                .map_err(|e| map_store_error(e.into()))?;
            let extra: serde_json::Value = row
                .try_get("extra")
                .map_err(|e| map_store_error(e.into()))?;

            events.push(ServerTelemetryEvent {
                namespace: ns,
                occurred_at: Some(timestamp_from(occurred_at)),
                level: Self::level_from_str(&level),
                event_type,
                message,
                extra_json: serde_json::to_vec(&extra).unwrap_or_default(),
            });

            next_cursor = Some((occurred_at, event_id));
        }

        let next_page_token = if has_more {
            let (ts, id) = next_cursor.expect("cursor set when events non-empty");
            encode_cursor(ts.timestamp_millis(), &id)
        } else {
            "".to_string()
        };

        let total_count = if page.include_total_count {
            let mut q = QueryBuilder::new(
                "SELECT COUNT(*) as count FROM kagzi.server_telemetry_events WHERE 1=1",
            );
            if let Some(ns) = &namespace {
                q.push(" AND namespace = ").push_bind(ns);
            }
            let row = q
                .build()
                .fetch_one(self.store.pool())
                .await
                .map_err(|e| map_store_error(e.into()))?;
            row.try_get::<i64, _>("count")
                .map_err(|e| map_store_error(e.into()))?
        } else {
            0
        };

        Ok(Response::new(ListServerTelemetryEventsResponse {
            events,
            page: Some(PageInfo {
                next_page_token,
                has_more,
                total_count,
            }),
        }))
    }
}
