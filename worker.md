# Worker Registration & Lifecycle Management

## Executive Summary

Currently, Kagzi workers are "fire and forget" - they connect, poll for work, and the server only knows about them implicitly through workflow locks. This document outlines a comprehensive worker lifecycle management system that provides visibility, reliability, and graceful operations.

---

## Current State

### How Workers Work Today

```
┌────────────────┐                    ┌─────────────────┐
│     Worker     │───PollActivity────►│     Server      │
│  (random UUID) │◄───────────────────│  (no registry)  │
└────────────────┘                    └─────────────────┘
        │                                      │
        │ worker_id only sent                  │ No knowledge of:
        │ during polling                       │ - Active workers
        │                                      │ - Worker capabilities
        │ No registration                      │ - Worker health
        │ No deregistration                    │ - Load distribution
        └──────────────────────────────────────┘
```

## Cron Schedules

- Use `cron::Schedule` format in UTC: `sec min hour dom month dow [year]`. Missed ticks after downtime are replayed up to `max_catchup` (default 100) using `(from, to]` bounds.
- Each fire is idempotent via `schedule:{schedule_id}:{fire_at_rfc3339}`; duplicates are skipped across restarts and multiple schedulers.
- Scheduler loop is configured via `KAGZI_SCHEDULER_INTERVAL_SECS` (default 5s), `KAGZI_SCHEDULER_BATCH_SIZE` (default 100), and `KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK` (default 1000).
- Disabling a schedule stops new fires; re-enabling catches up from the last fired time within `max_catchup`.
- Cron schedules trigger workflows from outside; workflow-level sleeps/timers remain the right tool for intra-workflow delays and retries.

### Problems

| Issue                  | Impact                                                      |
| ---------------------- | ----------------------------------------------------------- |
| No worker registry     | Server cannot list active workers                           |
| No capability tracking | Server assigns work without knowing if worker can handle it |
| No graceful shutdown   | Workers vanish, orphaning workflows for 30s+                |
| No health monitoring   | Dead workers detected only when workflows timeout           |
| No load visibility     | Cannot balance work across workers                          |
| Reactive recovery      | Watchdog finds orphans after-the-fact                       |

---

## Proposed Architecture

```
┌────────────────┐                    ┌─────────────────┐       ┌──────────────┐
│     Worker     │◄──RegisterWorker───│     Server      │──────►│   workers    │
│                │───────────────────►│                 │       │    table     │
│                │                    │                 │       └──────────────┘
│                │◄──WorkerHeartbeat──│  Worker         │
│                │───────────────────►│  Registry       │
│                │                    │                 │
│                │◄──DeregisterWorker─│  Watchdog       │
│                │───────────────────►│  (stale check)  │
└────────────────┘                    └─────────────────┘
```

---

## Phase 1: Database Schema

### Migration: `add_workers_table.sql`

```sql
-- Worker registry table
CREATE TABLE kagzi.workers (
    worker_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id TEXT NOT NULL DEFAULT 'default',
    task_queue TEXT NOT NULL,

    -- Identity
    hostname TEXT,
    pid INTEGER,
    version TEXT,

    -- Capabilities
    workflow_types TEXT[] NOT NULL DEFAULT '{}',
    max_concurrent INTEGER NOT NULL DEFAULT 100,

    -- State
    status TEXT NOT NULL DEFAULT 'ONLINE',  -- ONLINE, DRAINING, OFFLINE
    active_count INTEGER NOT NULL DEFAULT 0,

    -- Metrics (cumulative)
    total_completed BIGINT NOT NULL DEFAULT 0,
    total_failed BIGINT NOT NULL DEFAULT 0,

    -- Timestamps
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deregistered_at TIMESTAMPTZ,

    -- Extensibility
    labels JSONB DEFAULT '{}'
);

-- Fast lookup by queue
CREATE INDEX idx_workers_queue
ON kagzi.workers (namespace_id, task_queue, status)
WHERE status = 'ONLINE';

-- Fast stale detection
CREATE INDEX idx_workers_heartbeat
ON kagzi.workers (status, last_heartbeat_at)
WHERE status != 'OFFLINE';

-- Enforce uniqueness for active workers
CREATE UNIQUE INDEX idx_workers_active_unique
ON kagzi.workers (namespace_id, task_queue, hostname, pid)
WHERE status != 'OFFLINE';
```

---

## Phase 2: Protocol Definition

### New Proto Messages

```protobuf
// ===== Enums =====

enum WorkerStatus {
  WORKER_STATUS_UNSPECIFIED = 0;
  WORKER_STATUS_ONLINE = 1;      // Accepting and processing work
  WORKER_STATUS_DRAINING = 2;    // Finishing existing, rejecting new
  WORKER_STATUS_OFFLINE = 3;     // Not available
}

// ===== Messages =====

message Worker {
  string worker_id = 1;
  string namespace_id = 2;
  string task_queue = 3;
  WorkerStatus status = 4;

  string hostname = 5;
  int32 pid = 6;
  string version = 7;

  repeated string workflow_types = 8;
  int32 max_concurrent = 9;
  int32 active_count = 10;

  int64 total_completed = 11;
  int64 total_failed = 12;

  google.protobuf.Timestamp registered_at = 13;
  google.protobuf.Timestamp last_heartbeat_at = 14;

  map<string, string> labels = 15;
}

// ===== Worker Lifecycle =====

message RegisterWorkerRequest {
  string namespace_id = 1;
  string task_queue = 2;
  repeated string workflow_types = 3;

  string hostname = 4;
  int32 pid = 5;
  string version = 6;
  int32 max_concurrent = 7;
  map<string, string> labels = 8;
}

message RegisterWorkerResponse {
  string worker_id = 1;
  int32 heartbeat_interval_secs = 2;  // Server-controlled interval
}

message WorkerHeartbeatRequest {
  string worker_id = 1;
  int32 active_count = 2;
  int32 completed_delta = 3;  // Since last heartbeat
  int32 failed_delta = 4;
}

message WorkerHeartbeatResponse {
  bool accepted = 1;
  bool should_drain = 2;  // Server requests graceful shutdown
}

message DeregisterWorkerRequest {
  string worker_id = 1;
  bool drain = 2;  // Wait for active workflows to complete
}

// ===== Worker Discovery =====

message ListWorkersRequest {
  string namespace_id = 1;
  string task_queue = 2;       // Optional filter
  string filter_status = 3;    // Optional: ONLINE, DRAINING, OFFLINE
  int32 page_size = 4;
  string page_token = 5;
}

message ListWorkersResponse {
  repeated Worker workers = 1;
  string next_page_token = 2;
  int32 total_count = 3;
}

message GetWorkerRequest {
  string worker_id = 1;
}

message GetWorkerResponse {
  Worker worker = 1;
}
```

### Updated RPCs

```protobuf
service WorkflowService {
  // --- Client/Trigger Facing API ---
  rpc StartWorkflow (StartWorkflowRequest) returns (StartWorkflowResponse);
  rpc GetWorkflowRun (GetWorkflowRunRequest) returns (GetWorkflowRunResponse);
  rpc ListWorkflowRuns (ListWorkflowRunsRequest) returns (ListWorkflowRunsResponse);
  rpc CancelWorkflowRun (CancelWorkflowRunRequest) returns (Empty);

  // --- Step Attempt API ---
  rpc GetStepAttempt (GetStepAttemptRequest) returns (GetStepAttemptResponse);
  rpc ListStepAttempts (ListStepAttemptsRequest) returns (ListStepAttemptsResponse);

  // --- Worker Lifecycle API (NEW) ---
  rpc RegisterWorker(RegisterWorkerRequest) returns (RegisterWorkerResponse);
  rpc WorkerHeartbeat(WorkerHeartbeatRequest) returns (WorkerHeartbeatResponse);
  rpc DeregisterWorker(DeregisterWorkerRequest) returns (Empty);

  // --- Worker Discovery API (NEW) ---
  rpc ListWorkers(ListWorkersRequest) returns (ListWorkersResponse);
  rpc GetWorker(GetWorkerRequest) returns (GetWorkerResponse);

  // --- Worker Execution API (requires registration) ---
  rpc PollActivity (PollActivityRequest) returns (PollActivityResponse);
  rpc RecordHeartbeat (RecordHeartbeatRequest) returns (Empty);
  rpc BeginStep (BeginStepRequest) returns (BeginStepResponse);
  rpc CompleteStep (CompleteStepRequest) returns (Empty);
  rpc FailStep (FailStepRequest) returns (Empty);
  rpc CompleteWorkflow (CompleteWorkflowRequest) returns (Empty);
  rpc FailWorkflow (FailWorkflowRequest) returns (Empty);
  rpc ScheduleSleep (ScheduleSleepRequest) returns (Empty);

  // --- Health & Monitoring API ---
  rpc HealthCheck (HealthCheckRequest) returns (HealthCheckResponse);
}
```

### Breaking Changes to Existing Messages

```protobuf
// PollActivity now requires a registered worker
message PollActivityRequest {
  string worker_id = 1;           // REQUIRED: Must be from RegisterWorkerResponse
  string task_queue = 2;
  string namespace_id = 3;
  repeated string supported_workflow_types = 4;  // Worker sends its capabilities for SQL filtering
}

// RecordHeartbeat removed - replaced by WorkerHeartbeat
// (workflow-level heartbeat now bulk-extends all locks for this worker)
```

> ⚠️ **Critical: Type Poisoning Prevention**
>
> The `supported_workflow_types` field is sent by the worker on every poll. This allows
> the server to filter tasks **in the SQL query** rather than claiming a task first and
> checking capability after. Without this, a worker could get stuck in an infinite loop
> claiming tasks it can't handle, blocking newer tasks it could handle.

---

## Phase 3: Store Layer

### Models

```rust
// crates/kagzi-store/src/models/worker.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerStatus {
    Online,
    Draining,
    Offline,
}

impl WorkerStatus {
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "ONLINE" => Self::Online,
            "DRAINING" => Self::Draining,
            "OFFLINE" => Self::Offline,
            _ => Self::Offline,
        }
    }

    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Online => "ONLINE",
            Self::Draining => "DRAINING",
            Self::Offline => "OFFLINE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Worker {
    pub worker_id: Uuid,
    pub namespace_id: String,
    pub task_queue: String,
    pub status: WorkerStatus,

    pub hostname: Option<String>,
    pub pid: Option<i32>,
    pub version: Option<String>,

    pub workflow_types: Vec<String>,
    pub max_concurrent: i32,
    pub active_count: i32,

    pub total_completed: i64,
    pub total_failed: i64,

    pub registered_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub deregistered_at: Option<DateTime<Utc>>,

    pub labels: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct RegisterWorkerParams {
    pub namespace_id: String,
    pub task_queue: String,
    pub workflow_types: Vec<String>,
    pub hostname: Option<String>,
    pub pid: Option<i32>,
    pub version: Option<String>,
    pub max_concurrent: i32,
    pub labels: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct WorkerHeartbeatParams {
    pub worker_id: Uuid,
    pub active_count: i32,
    pub completed_delta: i32,
    pub failed_delta: i32,
}

#[derive(Debug, Clone, Default)]
pub struct ListWorkersParams {
    pub namespace_id: String,
    pub task_queue: Option<String>,
    pub filter_status: Option<WorkerStatus>,
    pub page_size: i32,
    pub cursor: Option<Uuid>,
}
```

### Repository Traits

```rust
// crates/kagzi-store/src/repository/worker.rs

#[async_trait]
pub trait WorkerRepository: Send + Sync {
    /// Register a new worker, returns worker_id
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError>;

    /// Update heartbeat timestamp and metrics, returns false if worker not found/offline
    async fn heartbeat(&self, params: WorkerHeartbeatParams) -> Result<bool, StoreError>;

    /// Mark worker as draining (finish current, reject new)
    async fn start_drain(&self, worker_id: Uuid) -> Result<(), StoreError>;

    /// Mark worker as offline
    async fn deregister(&self, worker_id: Uuid) -> Result<(), StoreError>;

    /// Get worker by ID
    async fn find_by_id(&self, worker_id: Uuid) -> Result<Option<Worker>, StoreError>;

    /// Validate worker exists and is online
    async fn validate_online(&self, worker_id: Uuid) -> Result<bool, StoreError>;

    /// List workers with filters
    async fn list(&self, params: ListWorkersParams) -> Result<Vec<Worker>, StoreError>;

    /// Mark stale workers (missed heartbeats) as offline, returns count
    async fn mark_stale_offline(&self, threshold_secs: i64) -> Result<u64, StoreError>;

    /// Get all worker_ids that are stale (for workflow recovery)
    async fn find_stale_worker_ids(&self, threshold_secs: i64) -> Result<Vec<Uuid>, StoreError>;

    /// Get online worker count for a queue
    async fn count_online(&self, namespace_id: &str, task_queue: &str) -> Result<i64, StoreError>;

    /// Update active_count for a worker (called when claiming/releasing work)
    async fn update_active_count(&self, worker_id: Uuid, delta: i32) -> Result<(), StoreError>;
}
```

### WorkflowRepository Additions

```rust
// crates/kagzi-store/src/repository/workflow.rs - ADD these methods

#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    // ... existing methods ...

    /// Claim next workflow, filtering by supported types (prevents Type Poisoning)
    async fn claim_next_filtered(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],  // ← NEW: Filter in SQL, not after claim
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    /// Bulk extend locks for all workflows owned by a worker (heartbeat optimization)
    /// Prevents watchdog from reclaiming tasks while worker is still alive
    async fn extend_locks_for_worker(
        &self,
        worker_id: &str,
        duration_secs: i64
    ) -> Result<u64, StoreError>;
}
```

**SQL for `claim_next_filtered`:**

```sql
-- Filter by workflow_type in the query, not after claiming
SELECT run_id FROM kagzi.workflow_runs
WHERE task_queue = $1
  AND namespace_id = $2
  AND workflow_type = ANY($3)  -- ← Critical: Filter here
  AND (
    (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
    OR (status = 'SLEEPING' AND wake_up_at <= NOW())
  )
ORDER BY COALESCE(wake_up_at, created_at) ASC
FOR UPDATE SKIP LOCKED
LIMIT 1
```

**SQL for `extend_locks_for_worker`:**

```sql
-- Bulk extend all locks for a worker (called during heartbeat)
UPDATE kagzi.workflow_runs
SET locked_until = NOW() + ($2 * INTERVAL '1 second')
WHERE locked_by = $1
  AND status = 'RUNNING'
```

---

## Phase 4: Server Implementation

### Service Methods

```rust
// crates/kagzi-server/src/service.rs

#[tonic::async_trait]
impl WorkflowService for MyWorkflowService {

    // ===== NEW: Worker Lifecycle =====

    async fn register_worker(
        &self,
        request: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        let req = request.into_inner();

        if req.workflow_types.is_empty() {
            return Err(Status::invalid_argument("workflow_types cannot be empty"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let worker_id = self.store.workers().register(RegisterWorkerParams {
            namespace_id,
            task_queue: req.task_queue,
            workflow_types: req.workflow_types,
            hostname: if req.hostname.is_empty() { None } else { Some(req.hostname) },
            pid: if req.pid == 0 { None } else { Some(req.pid) },
            version: if req.version.is_empty() { None } else { Some(req.version) },
            max_concurrent: req.max_concurrent.max(1),
            labels: serde_json::to_value(&req.labels).unwrap_or_default(),
        }).await.map_err(map_store_error)?;

        info!(worker_id = %worker_id, "Worker registered");

        Ok(Response::new(RegisterWorkerResponse {
            worker_id: worker_id.to_string(),
            heartbeat_interval_secs: 10,
        }))
    }

    async fn worker_heartbeat(
        &self,
        request: Request<WorkerHeartbeatRequest>,
    ) -> Result<Response<WorkerHeartbeatResponse>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

        // 1. Update worker's last_heartbeat_at
        let accepted = self.store.workers().heartbeat(WorkerHeartbeatParams {
            worker_id,
            active_count: req.active_count,
            completed_delta: req.completed_delta,
            failed_delta: req.failed_delta,
        }).await.map_err(map_store_error)?;

        if !accepted {
            return Err(Status::not_found("Worker not found or offline"));
        }

        // 2. CRITICAL: Bulk extend locks for all workflows owned by this worker
        //    Without this, watchdog would reclaim tasks thinking worker is dead
        let extended = self.store.workflows()
            .extend_locks_for_worker(&req.worker_id, 30)
            .await
            .map_err(map_store_error)?;

        if extended > 0 {
            debug!(worker_id = %worker_id, extended = extended, "Extended workflow locks");
        }

        // 3. Check if worker should drain (could be set by admin)
        let worker = self.store.workers().find_by_id(worker_id).await.map_err(map_store_error)?;
        let should_drain = worker.map(|w| w.status == WorkerStatus::Draining).unwrap_or(false);

        Ok(Response::new(WorkerHeartbeatResponse {
            accepted: true,
            should_drain,
        }))
    }

    async fn deregister_worker(
        &self,
        request: Request<DeregisterWorkerRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

        if req.drain {
            self.store.workers().start_drain(worker_id).await.map_err(map_store_error)?;
            info!(worker_id = %worker_id, "Worker draining");
        } else {
            self.store.workers().deregister(worker_id).await.map_err(map_store_error)?;
            info!(worker_id = %worker_id, "Worker deregistered");
        }

        Ok(Response::new(Empty {}))
    }

    async fn list_workers(
        &self,
        request: Request<ListWorkersRequest>,
    ) -> Result<Response<ListWorkersResponse>, Status> {
        let req = request.into_inner();

        let filter_status = match req.filter_status.as_str() {
            "ONLINE" => Some(WorkerStatus::Online),
            "DRAINING" => Some(WorkerStatus::Draining),
            "OFFLINE" => Some(WorkerStatus::Offline),
            _ => None,
        };

        let workers = self.store.workers().list(ListWorkersParams {
            namespace_id: if req.namespace_id.is_empty() { "default".to_string() } else { req.namespace_id },
            task_queue: if req.task_queue.is_empty() { None } else { Some(req.task_queue) },
            filter_status,
            page_size: req.page_size.clamp(1, 100),
            cursor: None, // TODO: implement pagination
        }).await.map_err(map_store_error)?;

        let proto_workers = workers.into_iter().map(worker_to_proto).collect();

        Ok(Response::new(ListWorkersResponse {
            workers: proto_workers,
            next_page_token: String::new(),
            total_count: 0, // TODO: implement count
        }))
    }

    async fn get_worker(
        &self,
        request: Request<GetWorkerRequest>,
    ) -> Result<Response<GetWorkerResponse>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

        let worker = self.store.workers().find_by_id(worker_id).await.map_err(map_store_error)?;

        match worker {
            Some(w) => Ok(Response::new(GetWorkerResponse {
                worker: Some(worker_to_proto(w)),
            })),
            None => Err(Status::not_found("Worker not found")),
        }
    }

    // ===== UPDATED: PollActivity requires registered worker =====

    async fn poll_activity(
        &self,
        request: Request<PollActivityRequest>,
    ) -> Result<Response<PollActivityResponse>, Status> {
        let req = request.into_inner();

        // Validate worker is registered and online (reject draining/offline)
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

        let is_online = self.store.workers().validate_online(worker_id).await.map_err(map_store_error)?;
        if !is_online {
            return Err(Status::failed_precondition("Worker not registered or offline. Call RegisterWorker first."));
        }

        // Do not hand out new work to draining workers; they should finish in-flight tasks only
        if let Some(worker) = self.store.workers().find_by_id(worker_id).await.map_err(map_store_error)? {
            if worker.status == WorkerStatus::Draining {
                return Err(Status::failed_precondition("Worker is draining and not accepting new work"));
            }
        }

        // Validate supported types provided
        if req.supported_workflow_types.is_empty() {
            return Err(Status::invalid_argument("supported_workflow_types cannot be empty"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let timeout = Duration::from_secs(60);

        // CRITICAL: Pass supported_workflow_types to filter in SQL
        // This prevents "Type Poisoning" where worker gets stuck on tasks it can't handle
        match self.work_distributor
            .wait_for_work_filtered(
                &req.task_queue,
                &namespace_id,
                &req.worker_id,
                &req.supported_workflow_types,  // ← Filter in SQL, not after claim
                timeout
            )
            .await
        {
            Some(work_item) => {
                // Update worker's active count
                let _ = self.store.workers().update_active_count(worker_id, 1).await;

                let input_bytes = serde_json::to_vec(&work_item.input)
                    .map_err(|_| Status::internal("Failed to serialize input"))?;

                Ok(Response::new(PollActivityResponse {
                    run_id: work_item.run_id.to_string(),
                    workflow_type: work_item.workflow_type,
                    workflow_input: input_bytes,
                }))
            }
            None => Ok(Response::new(PollActivityResponse::default())),
        }
    }
}

// Helper function
fn worker_to_proto(w: Worker) -> kagzi_proto::kagzi::Worker {
    kagzi_proto::kagzi::Worker {
        worker_id: w.worker_id.to_string(),
        namespace_id: w.namespace_id,
        task_queue: w.task_queue,
        status: match w.status {
            WorkerStatus::Online => 1,
            WorkerStatus::Draining => 2,
            WorkerStatus::Offline => 3,
        },
        hostname: w.hostname.unwrap_or_default(),
        pid: w.pid.unwrap_or(0),
        version: w.version.unwrap_or_default(),
        workflow_types: w.workflow_types,
        max_concurrent: w.max_concurrent,
        active_count: w.active_count,
        total_completed: w.total_completed,
        total_failed: w.total_failed,
        registered_at: Some(prost_types::Timestamp {
            seconds: w.registered_at.timestamp(),
            nanos: w.registered_at.timestamp_subsec_nanos() as i32,
        }),
        last_heartbeat_at: Some(prost_types::Timestamp {
            seconds: w.last_heartbeat_at.timestamp(),
            nanos: w.last_heartbeat_at.timestamp_subsec_nanos() as i32,
        }),
        labels: HashMap::new(), // TODO: convert from JSON
    }
}
```

### Updated Watchdog

```rust
// crates/kagzi-server/src/watchdog.rs

pub async fn run(store: PgStore) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    info!("Watchdog started");

    loop {
        interval.tick().await;

        // 1. Wake sleeping workflows
        if let Ok(count) = store.workflows().wake_sleeping().await {
            if count > 0 {
                info!("Woke {} sleeping workflows", count);
            }
        }

        // 2. Process step retries
        if let Ok(retries) = store.steps().process_pending_retries().await {
            for r in &retries {
                info!(run_id = %r.run_id, step_id = %r.step_id, "Triggered step retry");
            }
        }

        // 3. Mark stale workers as offline (30s without heartbeat)
        match store.workers().mark_stale_offline(30).await {
            Ok(count) if count > 0 => {
                warn!("Marked {} stale workers as offline", count);
            }
            Err(e) => error!("Failed to mark stale workers: {:?}", e),
            _ => {}
        }

        // 4. Recover orphaned workflows (locked by dead workers)
        match store.workflows().find_orphaned().await {
            Ok(orphans) => {
                for orphan in orphans {
                    let policy = orphan.retry_policy.unwrap_or_default();

                    if policy.should_retry(orphan.attempts) {
                        let delay_ms = policy.calculate_delay_ms(orphan.attempts) as u64;

                        if let Err(e) = store.workflows().schedule_retry(orphan.run_id, delay_ms).await {
                            error!("Failed to schedule retry for {}: {:?}", orphan.run_id, e);
                        } else {
                            warn!(
                                run_id = %orphan.run_id,
                                worker = ?orphan.locked_by,
                                delay_ms = delay_ms,
                                "Recovered orphaned workflow"
                            );
                        }
                    } else {
                        if let Err(e) = store.workflows()
                            .mark_exhausted(orphan.run_id, "Worker died, retries exhausted")
                            .await
                        {
                            error!("Failed to mark {} as failed: {:?}", orphan.run_id, e);
                        } else {
                            error!(run_id = %orphan.run_id, "Workflow failed - worker died, no retries left");
                        }
                    }
                }
            }
            Err(e) => error!("Failed to find orphaned workflows: {:?}", e),
        }
    }
}
```

---

## Phase 5: Client Library

### New Worker API

```rust
// crates/kagzi/src/lib.rs

pub struct WorkerBuilder {
    addr: String,
    task_queue: String,
    namespace_id: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
}

impl WorkerBuilder {
    pub fn new(addr: &str, task_queue: &str) -> Self {
        Self {
            addr: addr.to_string(),
            task_queue: task_queue.to_string(),
            namespace_id: "default".to_string(),
            max_concurrent: 100,
            hostname: None,
            version: None,
            labels: HashMap::new(),
        }
    }

    pub fn namespace(mut self, ns: &str) -> Self {
        self.namespace_id = ns.to_string();
        self
    }

    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n;
        self
    }

    pub fn hostname(mut self, h: &str) -> Self {
        self.hostname = Some(h.to_string());
        self
    }

    pub fn version(mut self, v: &str) -> Self {
        self.version = Some(v.to_string());
        self
    }

    pub fn label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_string(), value.to_string());
        self
    }

    pub async fn build(self) -> anyhow::Result<Worker> {
        let client = WorkflowServiceClient::connect(self.addr.clone()).await?;

        Ok(Worker {
            client,
            addr: self.addr,
            task_queue: self.task_queue,
            namespace_id: self.namespace_id,
            max_concurrent: self.max_concurrent,
            hostname: self.hostname,
            version: self.version,
            labels: self.labels,
            workflows: HashMap::new(),
            worker_id: None,
            heartbeat_interval: Duration::from_secs(10),
            semaphore: Arc::new(Semaphore::new(self.max_concurrent)),
            shutdown: CancellationToken::new(),
        })
    }
}

pub struct Worker {
    client: WorkflowServiceClient<Channel>,
    addr: String,
    task_queue: String,
    namespace_id: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    workflows: HashMap<String, Arc<WorkflowFn>>,
    workflow_types: Vec<String>,  // Cached for poll requests (avoids rebuild every poll)
    worker_id: Option<Uuid>,
    heartbeat_interval: Duration,
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
}

impl Worker {
    /// Create a worker builder
    pub fn builder(addr: &str, task_queue: &str) -> WorkerBuilder {
        WorkerBuilder::new(addr, task_queue)
    }

    /// Register a workflow handler (call before run)
    pub fn register<F, Fut, I, O>(&mut self, name: &str, func: F)
    where
        F: Fn(WorkflowContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<O>> + Send + 'static,
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
    {
        let wrapped = move |ctx: WorkflowContext, input: serde_json::Value| -> BoxFuture<'static, anyhow::Result<serde_json::Value>> {
            let input: I = serde_json::from_value(input).unwrap();
            let fut = func(ctx, input);
            Box::pin(async move {
                let output = fut.await?;
                Ok(serde_json::to_value(output)?)
            })
        };
        self.workflows.insert(name.to_string(), Arc::new(Box::new(wrapped)));
    }

    /// Get the worker ID (None if not yet registered)
    pub fn worker_id(&self) -> Option<Uuid> {
        self.worker_id
    }

    /// Check if worker is registered
    pub fn is_registered(&self) -> bool {
        self.worker_id.is_some()
    }

    /// Get current active workflow count
    pub fn active_count(&self) -> usize {
        self.max_concurrent - self.semaphore.available_permits()
    }

    /// Signal graceful shutdown (finish current work, stop accepting new)
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Run the worker (registers, polls, handles shutdown)
    pub async fn run(&mut self) -> anyhow::Result<()> {
        if self.workflows.is_empty() {
            anyhow::bail!("No workflows registered. Call register() before run()");
        }

        // 1. Register with server
        let workflow_types: Vec<String> = self.workflows.keys().cloned().collect();
        self.workflow_types = workflow_types.clone(); // cache for poll requests

        let resp = self.client.register_worker(RegisterWorkerRequest {
            namespace_id: self.namespace_id.clone(),
            task_queue: self.task_queue.clone(),
            workflow_types,
            hostname: self.hostname.clone().unwrap_or_else(|| {
                hostname::get().ok()
                    .and_then(|h| h.into_string().ok())
                    .unwrap_or_default()
            }),
            pid: std::process::id() as i32,
            version: self.version.clone().unwrap_or_default(),
            max_concurrent: self.max_concurrent as i32,
            labels: self.labels.clone(),
        }).await?;

        let resp = resp.into_inner();
        self.worker_id = Some(Uuid::parse_str(&resp.worker_id)?);
        self.heartbeat_interval = Duration::from_secs(resp.heartbeat_interval_secs as u64);

        info!(
            worker_id = %self.worker_id.unwrap(),
            task_queue = %self.task_queue,
            workflows = ?self.workflows.keys().collect::<Vec<_>>(),
            "Worker registered"
        );

        // 2. Start heartbeat task
        let heartbeat_handle = self.spawn_heartbeat_task();

        // 3. Poll loop
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    info!("Worker shutdown signal received");
                    break;
                }
                _ = self.poll_and_execute() => {}
            }
        }

        // 4. Wait for active workflows to complete
        info!(active = self.active_count(), "Draining active workflows...");
        while self.active_count() > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // 5. Deregister
        heartbeat_handle.abort();
        let _ = self.client.deregister_worker(DeregisterWorkerRequest {
            worker_id: self.worker_id.unwrap().to_string(),
            drain: false,
        }).await;

        info!("Worker deregistered");
        Ok(())
    }

    fn spawn_heartbeat_task(&self) -> tokio::task::JoinHandle<()> {
        let mut client = self.client.clone();
        let worker_id = self.worker_id.unwrap();
        let semaphore = self.semaphore.clone();
        let max = self.max_concurrent;
        let interval = self.heartbeat_interval;
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            let mut completed: i32 = 0;
            let mut failed: i32 = 0;

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {
                        let active = (max - semaphore.available_permits()) as i32;

                        let resp = client.worker_heartbeat(WorkerHeartbeatRequest {
                            worker_id: worker_id.to_string(),
                            active_count: active,
                            completed_delta: completed,
                            failed_delta: failed,
                        }).await;

                        // Reset counters after sending
                        completed = 0;
                        failed = 0;

                        match resp {
                            Ok(r) if r.into_inner().should_drain => {
                                info!("Server requested drain");
                                shutdown.cancel();
                            }
                            Err(e) => {
                                error!("Heartbeat failed: {:?}", e);
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
    }

    async fn poll_and_execute(&mut self) {
        let permit = match self.semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Send supported types so server can filter in SQL (prevents Type Poisoning)
        let resp = self.client.poll_activity(PollActivityRequest {
            worker_id: self.worker_id.unwrap().to_string(),
            task_queue: self.task_queue.clone(),
            namespace_id: self.namespace_id.clone(),
            supported_workflow_types: self.workflow_types.clone(),  // ← Cached, sent every poll
        }).await;

        match resp {
            Ok(r) => {
                let task = r.into_inner();
                if task.run_id.is_empty() {
                    drop(permit);
                    return;
                }

                if let Some(handler) = self.workflows.get(&task.workflow_type) {
                    let handler = handler.clone();
                    let client = self.client.clone();
                    let input: serde_json::Value = serde_json::from_slice(&task.workflow_input)
                        .unwrap_or(serde_json::Value::Null);

                    tokio::spawn(async move {
                        let _permit = permit;
                        execute_workflow(client, handler, task.run_id, input).await;
                    });
                } else {
                    error!("No handler for workflow: {}", task.workflow_type);
                    drop(permit);
                }
            }
            Err(e) => {
                drop(permit);
                if e.code() != tonic::Code::DeadlineExceeded {
                    error!("Poll failed: {:?}", e);
                }
            }
        }
    }
}
```

---

## Phase 6: Implementation Checklist

### Milestone 1: Store Layer

- [ ] Create migration `YYYYMMDD_add_workers_table.sql`
- [ ] Add `worker.rs` to `kagzi-store/src/models/`
- [ ] Update `kagzi-store/src/models/mod.rs`
- [ ] Add `WorkerRepository` trait to `kagzi-store/src/repository/worker.rs`
- [ ] Update `kagzi-store/src/repository/mod.rs`
- [ ] Implement `PgWorkerRepository` in `kagzi-store/src/postgres/worker.rs`
- [ ] Update `kagzi-store/src/postgres/mod.rs`
- [ ] Export from `kagzi-store/src/lib.rs`
- [ ] **Add `claim_next_filtered()` to WorkflowRepository** (Type Poisoning fix)
- [ ] **Add `extend_locks_for_worker()` to WorkflowRepository** (Lock Extension fix)

### Milestone 2: Protocol

- [ ] Add `WorkerStatus` enum to `proto/kagzi.proto`
- [ ] Add `Worker` message
- [ ] Add `RegisterWorkerRequest/Response`
- [ ] Add `WorkerHeartbeatRequest/Response`
- [ ] Add `DeregisterWorkerRequest`
- [ ] Add `ListWorkersRequest/Response`
- [ ] Add `GetWorkerRequest/Response`
- [ ] **Add `supported_workflow_types` to `PollActivityRequest`** (Type Poisoning fix)
- [ ] Add new RPCs to `WorkflowService`
- [ ] Rebuild proto crate: `cargo build -p kagzi-proto`

### Milestone 3: Server

- [ ] Add `RegisterWorker` RPC implementation
- [ ] Add `WorkerHeartbeat` RPC implementation (with bulk lock extension)
- [ ] Add `DeregisterWorker` RPC implementation
- [ ] Add `ListWorkers` RPC implementation
- [ ] Add `GetWorker` RPC implementation
- [ ] **Update `PollActivity` to use `supported_workflow_types` filter**
- [ ] **Update `WorkDistributor` to use `claim_next_filtered`**
- [ ] Add `worker_to_proto` helper
- [ ] Update watchdog for stale worker detection
- [ ] Remove `RecordHeartbeat` RPC (replaced by `WorkerHeartbeat`)

### Milestone 4: Client

- [ ] Add `WorkerBuilder` struct
- [ ] Refactor `Worker` struct with `workflow_types` cache
- [ ] Implement `Worker::builder()`
- [ ] Update `Worker::register()` to store workflow names
- [ ] Implement `Worker::run()` with server registration
- [ ] **Send `supported_workflow_types` in every poll request**
- [ ] Add heartbeat background task
- [ ] Implement graceful shutdown with drain
- [ ] Add deregistration on exit
- [ ] **Add SIGTERM handling for Kubernetes** (optional but recommended)
- [ ] Update `WorkflowContext` (remove per-workflow heartbeat)

### Milestone 5: Testing & Cleanup

- [ ] Add worker registration integration tests
- [ ] Add worker heartbeat tests
- [ ] Add stale worker detection tests
- [ ] Update existing integration tests
- [ ] Update all examples to use new Worker API
- [ ] Update README with new Worker usage

---

## API Usage Examples

### Starting a Worker

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Build and configure worker
    let mut worker = Worker::builder("http://localhost:50051", "default")
        .namespace("production")
        .max_concurrent(50)
        .version("1.2.3")
        .label("region", "us-east-1")
        .build()
        .await?;

    // Register workflow handlers
    worker.register("send_email", send_email_workflow);
    worker.register("process_order", process_order_workflow);

    // Handle shutdown signals
    let shutdown = worker.shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        shutdown.cancel();
    });

    // Run until shutdown
    worker.run().await
}
```

### Listing Workers (Admin/Monitoring)

```rust
let resp = client.list_workers(ListWorkersRequest {
    namespace_id: "production".to_string(),
    task_queue: "default".to_string(),
    filter_status: "ONLINE".to_string(),
    page_size: 100,
    page_token: String::new(),
}).await?;

for worker in resp.workers {
    println!(
        "[{}] {} - {} active, {} completed, {} failed",
        worker.status,
        worker.worker_id,
        worker.active_count,
        worker.total_completed,
        worker.total_failed,
    );
}
```

---

## Success Metrics

| Metric                     | Before             | After                     |
| -------------------------- | ------------------ | ------------------------- |
| Time to detect dead worker | 30s (lock timeout) | 10s (missed heartbeat)    |
| Worker visibility          | None               | Full registry with status |
| Graceful shutdown          | None               | Yes, with drain           |
| Workflow type validation   | None               | Yes, at poll time         |
| Load visibility            | None               | Per-worker active count   |
| Worker metrics             | None               | Completed/failed counts   |

---

## Critical Architectural Notes

### 1. Type Poisoning Prevention

**Problem**: If we claim a task first and then check capabilities, workers can get stuck in an infinite loop claiming tasks they can't handle, blocking newer tasks they could handle.

**Solution**: Workers send `supported_workflow_types` in every `PollActivityRequest`. The server filters tasks **in the SQL query** using `workflow_type = ANY($supported_types)`. This ensures workers only receive tasks they can handle.

### 2. Lock Extension via Heartbeat

**Problem**: Removing per-workflow `RecordHeartbeat` means `workflow_runs.locked_until` won't be updated. The watchdog will reclaim tasks thinking the worker died.

**Solution**: `WorkerHeartbeat` now **bulk extends all workflow locks** owned by that worker:

```sql
UPDATE kagzi.workflow_runs
SET locked_until = NOW() + INTERVAL '30 seconds'
WHERE locked_by = $worker_id AND status = 'RUNNING'
```

### 3. Signal Handling (Kubernetes)

For production deployments, workers should handle both `SIGTERM` (Kubernetes pod termination) and `SIGINT` (Ctrl+C):

```rust
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};

// In main or worker setup
let shutdown = worker.shutdown_token();
tokio::spawn(async move {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let mut sigint = signal(SignalKind::interrupt()).unwrap();

    tokio::select! {
        _ = sigterm.recv() => info!("Received SIGTERM"),
        _ = sigint.recv() => info!("Received SIGINT"),
    }
    shutdown.cancel();
});
```

### 4. Container Identity

In Docker/Kubernetes:

- `hostname` is often the Pod ID (random string)
- `pid` is always 1

**Recommendation**: Use the `labels` field to pass meaningful identifiers like `deployment_id`, `version`, or `replica_set` for grouping in dashboards.

---

## Open Questions

1. **Heartbeat interval**: Fixed 10s or make it configurable?
2. **Stale threshold**: 30s (3x heartbeat) reasonable?
3. **Drain timeout**: Max time to wait for active workflows during drain?
4. **Offline worker cleanup**: Auto-delete offline workers after X days?
5. **Admin APIs**: Add force-drain, force-offline endpoints later?
