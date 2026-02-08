# Happyflow Architecture Plan (v1)

## 1. Goals

### 1.1 Primary goals
- Keep the system operationally simple (single backend service, PostgreSQL as source of truth).
- Guarantee durable execution with deterministic replay semantics.
- Keep scheduling, retry, sleep, and worker recovery first-class.
- Ensure correctness under concurrency before optimizing throughput.
- Keep codebase modular with small, focused files and clear ownership boundaries.

### 1.2 Non-goals (v1)
- No multi-broker/event-bus architecture.
- No push-stream task delivery as the primary model.
- No cross-region replication strategy in v1.
- No dynamic policy language for routing.

## 2. High-Level System

## 2.1 Components
1. `happyflow-server`
- Stateless gRPC server.
- Performs validation, orchestration, task claiming, and state transitions.

2. `PostgreSQL`
- Single source of truth.
- Stores workflow state, queue state, step history, worker leases, and limits.

3. `happyflow-rs` SDK
- Client API for starting/querying workflows.
- Worker runtime for register/poll/heartbeat/step execution.

4. `Coordinator loop` (inside server)
- Fires schedules.
- Marks stale workers offline.
- Reclaims expired leases.

## 2.2 Delivery model
- Primary model: `pull` (workers long-poll for tasks).
- Secondary model: `notify-as-hint` (Postgres `NOTIFY` used only to wake pollers early).
- Correctness must never depend on notifications.

## 3. Repository Layout

```text
happyflow/
  Cargo.toml
  happyflow.md
  migrations/
  proto/
    common.proto
    workflow.proto
    worker.proto
    schedule.proto
    admin.proto
  crates/
    happyflow-proto/
    happyflow-store/
    happyflow-server/
    happyflow-tests/
  sdk/
    happyflow-rs/
```

## 4. Runtime Architecture

## 4.1 Process model
- Single server binary can run N replicas.
- No in-memory workflow ownership assumptions.
- All claim/lease state in DB.

## 4.2 Internal modules (`happyflow-server`)
- `api/`:
  - `workflow_service.rs`
  - `worker_service.rs`
  - `schedule_service.rs`
  - `admin_service.rs`
- `app/` (business logic, no transport types):
  - `workflow_app.rs`
  - `worker_app.rs`
  - `schedule_app.rs`
  - `limits_app.rs`
- `coordinator/`:
  - `scheduler.rs`
  - `worker_reaper.rs`
  - `lease_reaper.rs`
- `security/`:
  - `auth_interceptor.rs`
  - `principal.rs`
- `telemetry/`:
  - `tracing.rs`
  - `metrics.rs`

Rule: API module maps request/response only. All orchestration belongs in `app/`.

## 5. Data Model & State Machine

## 5.1 Workflow run states
- `PENDING`: created, waiting to be claimed.
- `RUNNING`: leased by a worker.
- `SLEEPING`: paused until `available_at`.
- `COMPLETED`: terminal success.
- `FAILED`: terminal failure.
- `CANCELLED`: terminal cancellation.
- `SCHEDULED`: schedule template run.
- `PAUSED`: paused schedule template.

## 5.2 Step states
- `PENDING`
- `RUNNING`
- `COMPLETED`
- `FAILED`

## 5.3 Lease semantics
- Claim sets:
  - `locked_by = worker_id`
  - `leased_until = now() + visibility_timeout`
- Heartbeat extends `leased_until`.
- Expired lease enables reclaim.

## 6. PostgreSQL Schema (v1)

## 6.1 Core tables

```sql
CREATE SCHEMA IF NOT EXISTS happyflow;

CREATE TYPE happyflow.workflow_status AS ENUM (
  'PENDING', 'RUNNING', 'SLEEPING', 'COMPLETED', 'FAILED', 'CANCELLED', 'SCHEDULED', 'PAUSED'
);

CREATE TYPE happyflow.step_status AS ENUM (
  'PENDING', 'RUNNING', 'COMPLETED', 'FAILED'
);

CREATE TYPE happyflow.worker_status AS ENUM (
  'ONLINE', 'DRAINING', 'OFFLINE'
);

CREATE TABLE happyflow.workflow_runs (
  run_id UUID PRIMARY KEY,
  namespace TEXT NOT NULL,
  external_id TEXT NOT NULL,
  task_queue TEXT NOT NULL,
  workflow_type TEXT NOT NULL,
  status happyflow.workflow_status NOT NULL,
  version TEXT NOT NULL DEFAULT '1',
  attempts INT NOT NULL DEFAULT 0,
  max_attempts INT NOT NULL DEFAULT 5,
  error TEXT,
  retry_policy JSONB,
  cron_expr TEXT,
  schedule_id UUID REFERENCES happyflow.workflow_runs(run_id) ON DELETE SET NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  available_at TIMESTAMPTZ,
  leased_until TIMESTAMPTZ,
  locked_by UUID,
  last_fired_at TIMESTAMPTZ,
  max_catchup INT NOT NULL DEFAULT 50,
  CONSTRAINT uq_workflow_external_active UNIQUE (namespace, external_id, status)
    DEFERRABLE INITIALLY IMMEDIATE
);

CREATE TABLE happyflow.workflow_payloads (
  run_id UUID PRIMARY KEY REFERENCES happyflow.workflow_runs(run_id) ON DELETE CASCADE,
  input BYTEA NOT NULL,
  output BYTEA
);

CREATE TABLE happyflow.step_runs (
  attempt_id UUID PRIMARY KEY,
  run_id UUID NOT NULL REFERENCES happyflow.workflow_runs(run_id) ON DELETE CASCADE,
  namespace TEXT NOT NULL,
  step_id TEXT NOT NULL,
  step_kind TEXT NOT NULL,
  attempt_number INT NOT NULL,
  status happyflow.step_status NOT NULL,
  input BYTEA,
  output BYTEA,
  error TEXT,
  is_latest BOOLEAN NOT NULL DEFAULT true,
  retry_policy JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  retry_at TIMESTAMPTZ,
  UNIQUE (run_id, step_id, attempt_number)
);

CREATE TABLE happyflow.workers (
  worker_id UUID PRIMARY KEY,
  namespace TEXT NOT NULL,
  task_queue TEXT NOT NULL,
  status happyflow.worker_status NOT NULL,
  workflow_types TEXT[] NOT NULL,
  max_concurrent INT NOT NULL DEFAULT 10,
  active_count INT NOT NULL DEFAULT 0,
  queue_concurrency_limit INT,
  workflow_type_concurrency JSONB,
  labels JSONB NOT NULL DEFAULT '{}'::jsonb,
  version TEXT,
  hostname TEXT,
  pid INT,
  registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  drained_at TIMESTAMPTZ
);
```

## 6.2 Queue table (hot path)

```sql
CREATE TABLE happyflow.run_queue (
  run_id UUID PRIMARY KEY REFERENCES happyflow.workflow_runs(run_id) ON DELETE CASCADE,
  namespace TEXT NOT NULL,
  task_queue TEXT NOT NULL,
  workflow_type TEXT NOT NULL,
  priority SMALLINT NOT NULL DEFAULT 100,
  available_at TIMESTAMPTZ NOT NULL,
  leased_until TIMESTAMPTZ,
  locked_by UUID,
  enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## 6.3 Concurrency tracking

```sql
CREATE TABLE happyflow.concurrency_limits (
  namespace TEXT NOT NULL,
  task_queue TEXT,
  workflow_type TEXT,
  max_inflight INT NOT NULL,
  PRIMARY KEY (namespace, task_queue, workflow_type)
);

CREATE TABLE happyflow.concurrency_counters (
  namespace TEXT NOT NULL,
  task_queue TEXT,
  workflow_type TEXT,
  inflight INT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (namespace, task_queue, workflow_type)
);
```

## 6.4 Indexes

```sql
CREATE INDEX idx_run_queue_claim
  ON happyflow.run_queue (namespace, task_queue, available_at, priority, enqueued_at)
  WHERE leased_until IS NULL OR leased_until < now();

CREATE INDEX idx_workflow_status_created
  ON happyflow.workflow_runs (namespace, status, created_at DESC);

CREATE INDEX idx_workflow_schedule_due
  ON happyflow.workflow_runs (namespace, available_at)
  WHERE status = 'SCHEDULED';

CREATE INDEX idx_workers_heartbeat
  ON happyflow.workers (status, last_heartbeat_at);

CREATE INDEX idx_step_latest
  ON happyflow.step_runs (run_id, step_id)
  WHERE is_latest = true;
```

Note: remove the invalid "unique by status" active-id constraint in actual implementation; use a partial unique index instead:

```sql
CREATE UNIQUE INDEX uq_workflow_external_active
  ON happyflow.workflow_runs(namespace, external_id)
  WHERE status IN ('PENDING', 'RUNNING', 'SLEEPING');
```

## 7. gRPC API Schema (v1)

## 7.1 Common proto (`common.proto`)

```proto
syntax = "proto3";
package happyflow.v1;

message Payload {
  bytes data = 1;
  string encoding = 2; // "json", "raw"
}

message PageRequest {
  int32 page_size = 1;
  string page_token = 2;
  bool include_total_count = 3;
}

message PageInfo {
  string next_page_token = 1;
  bool has_more = 2;
  int64 total_count = 3;
}

message RetryPolicy {
  int32 maximum_attempts = 1;
  int64 initial_interval_ms = 2;
  double backoff_coefficient = 3;
  int64 maximum_interval_ms = 4;
  repeated string non_retryable_errors = 5;
}
```

## 7.2 Workflow service (`workflow.proto`)

```proto
syntax = "proto3";
package happyflow.v1;

import "common.proto";

enum WorkflowStatus {
  WORKFLOW_STATUS_UNSPECIFIED = 0;
  WORKFLOW_STATUS_PENDING = 1;
  WORKFLOW_STATUS_RUNNING = 2;
  WORKFLOW_STATUS_SLEEPING = 3;
  WORKFLOW_STATUS_COMPLETED = 4;
  WORKFLOW_STATUS_FAILED = 5;
  WORKFLOW_STATUS_CANCELLED = 6;
}

message WorkflowRun {
  string run_id = 1;
  string namespace = 2;
  string external_id = 3;
  string task_queue = 4;
  string workflow_type = 5;
  WorkflowStatus status = 6;
  Payload input = 7;
  Payload output = 8;
  int32 attempts = 9;
  string error = 10;
  string created_at = 11;
  string started_at = 12;
  string finished_at = 13;
}

message StartWorkflowRequest {
  string namespace = 1;
  string external_id = 2;
  string task_queue = 3;
  string workflow_type = 4;
  string version = 5;
  Payload input = 6;
  RetryPolicy retry_policy = 7;
}

message StartWorkflowResponse {
  string run_id = 1;
  bool already_exists = 2;
}

message GetWorkflowRequest { string namespace = 1; string run_id = 2; }
message GetWorkflowResponse { WorkflowRun workflow = 1; }

message ListWorkflowsRequest {
  string namespace = 1;
  WorkflowStatus status_filter = 2;
  PageRequest page = 3;
}

message ListWorkflowsResponse {
  repeated WorkflowRun workflows = 1;
  PageInfo page = 2;
}

message CancelWorkflowRequest { string namespace = 1; string run_id = 2; }
message CancelWorkflowResponse { bool cancelled = 1; }

service WorkflowService {
  rpc StartWorkflow(StartWorkflowRequest) returns (StartWorkflowResponse);
  rpc GetWorkflow(GetWorkflowRequest) returns (GetWorkflowResponse);
  rpc ListWorkflows(ListWorkflowsRequest) returns (ListWorkflowsResponse);
  rpc CancelWorkflow(CancelWorkflowRequest) returns (CancelWorkflowResponse);
}
```

## 7.3 Worker service (`worker.proto`)

```proto
syntax = "proto3";
package happyflow.v1;

import "common.proto";

message WorkflowTypeLimit {
  string workflow_type = 1;
  int32 max_concurrent = 2;
}

message RegisterWorkerRequest {
  string namespace = 1;
  string task_queue = 2;
  repeated string workflow_types = 3;
  int32 max_concurrent = 4;
  int32 queue_concurrency_limit = 5;
  repeated WorkflowTypeLimit workflow_type_limits = 6;
  string hostname = 7;
  int32 pid = 8;
  string version = 9;
}

message RegisterWorkerResponse {
  string worker_id = 1;
  int32 heartbeat_interval_secs = 2;
}

message PollTasksRequest {
  string namespace = 1;
  string task_queue = 2;
  string worker_id = 3;
  repeated string workflow_types = 4;
  int32 available_slots = 5;
  int32 max_batch_size = 6;
}

message TaskItem {
  string run_id = 1;
  string workflow_type = 2;
  Payload input = 3;
  string lease_deadline = 4;
}

message PollTasksResponse {
  repeated TaskItem tasks = 1;
}

message HeartbeatRequest {
  string namespace = 1;
  string worker_id = 2;
  repeated string active_run_ids = 3;
  int32 active_count = 4;
}

message HeartbeatResponse {
  bool accepted = 1;
  bool should_drain = 2;
}

message BeginStepRequest {
  string namespace = 1;
  string run_id = 2;
  string step_name = 3;
  string step_kind = 4;
  Payload input = 5;
  RetryPolicy retry_policy = 6;
}

message BeginStepResponse {
  bool should_execute = 1;
  int32 attempt_number = 2;
  Payload replay_output = 3;
}

message CompleteStepRequest {
  string namespace = 1;
  string run_id = 2;
  string step_name = 3;
  Payload output = 4;
}

message CompleteStepResponse {}

message FailStepRequest {
  string namespace = 1;
  string run_id = 2;
  string step_name = 3;
  string error = 4;
}

message FailStepResponse {
  bool should_retry = 1;
  int64 retry_delay_ms = 2;
}

message SleepWorkflowRequest {
  string namespace = 1;
  string run_id = 2;
  int64 sleep_ms = 3;
}

message SleepWorkflowResponse {}

message CompleteWorkflowRequest {
  string namespace = 1;
  string worker_id = 2;
  string run_id = 3;
  Payload output = 4;
}

message CompleteWorkflowResponse {}

message FailWorkflowRequest {
  string namespace = 1;
  string worker_id = 2;
  string run_id = 3;
  string error = 4;
}

message FailWorkflowResponse {
  bool should_retry = 1;
  int64 retry_delay_ms = 2;
}

message DeregisterWorkerRequest {
  string namespace = 1;
  string worker_id = 2;
  bool drain = 3;
}

message DeregisterWorkerResponse { bool drained = 1; }

service WorkerService {
  rpc RegisterWorker(RegisterWorkerRequest) returns (RegisterWorkerResponse);
  rpc PollTasks(PollTasksRequest) returns (PollTasksResponse);
  rpc Heartbeat(HeartbeatRequest) returns (HeartbeatResponse);
  rpc BeginStep(BeginStepRequest) returns (BeginStepResponse);
  rpc CompleteStep(CompleteStepRequest) returns (CompleteStepResponse);
  rpc FailStep(FailStepRequest) returns (FailStepResponse);
  rpc SleepWorkflow(SleepWorkflowRequest) returns (SleepWorkflowResponse);
  rpc CompleteWorkflow(CompleteWorkflowRequest) returns (CompleteWorkflowResponse);
  rpc FailWorkflow(FailWorkflowRequest) returns (FailWorkflowResponse);
  rpc DeregisterWorker(DeregisterWorkerRequest) returns (DeregisterWorkerResponse);
}
```

## 7.4 Schedule service (`schedule.proto`)

```proto
syntax = "proto3";
package happyflow.v1;

import "common.proto";

message CreateScheduleRequest {
  string namespace = 1;
  string schedule_id = 2;
  string task_queue = 3;
  string workflow_type = 4;
  string cron_expr = 5;
  Payload input = 6;
  bool enabled = 7;
  int32 max_catchup = 8;
  string version = 9;
}

message Schedule {
  string schedule_id = 1;
  string namespace = 2;
  string task_queue = 3;
  string workflow_type = 4;
  string cron_expr = 5;
  bool enabled = 6;
  int32 max_catchup = 7;
  string next_fire_at = 8;
  string last_fired_at = 9;
  Payload input = 10;
}

message CreateScheduleResponse { Schedule schedule = 1; }
message GetScheduleRequest { string namespace = 1; string schedule_id = 2; }
message GetScheduleResponse { Schedule schedule = 1; }
message PauseScheduleRequest { string namespace = 1; string schedule_id = 2; }
message PauseScheduleResponse {}
message ResumeScheduleRequest { string namespace = 1; string schedule_id = 2; }
message ResumeScheduleResponse {}
message DeleteScheduleRequest { string namespace = 1; string schedule_id = 2; }
message DeleteScheduleResponse {}

service ScheduleService {
  rpc CreateSchedule(CreateScheduleRequest) returns (CreateScheduleResponse);
  rpc GetSchedule(GetScheduleRequest) returns (GetScheduleResponse);
  rpc PauseSchedule(PauseScheduleRequest) returns (PauseScheduleResponse);
  rpc ResumeSchedule(ResumeScheduleRequest) returns (ResumeScheduleResponse);
  rpc DeleteSchedule(DeleteScheduleRequest) returns (DeleteScheduleResponse);
}
```

## 7.5 Admin service (`admin.proto`)

```proto
syntax = "proto3";
package happyflow.v1;

import "common.proto";

message HealthRequest {}
message HealthResponse { bool ok = 1; string version = 2; }

message ListWorkersRequest {
  string namespace = 1;
  PageRequest page = 2;
}

message WorkerInfo {
  string worker_id = 1;
  string namespace = 2;
  string task_queue = 3;
  string status = 4;
  int32 active_count = 5;
  string last_heartbeat_at = 6;
}

message ListWorkersResponse {
  repeated WorkerInfo workers = 1;
  PageInfo page = 2;
}

service AdminService {
  rpc Health(HealthRequest) returns (HealthResponse);
  rpc ListWorkers(ListWorkersRequest) returns (ListWorkersResponse);
}
```

## 8. Task Claiming Algorithm

## 8.1 Claim strategy
- Worker polls with `available_slots`.
- Server computes `claim_limit = min(available_slots, max_batch_size, server_batch_cap)`.
- Claim tasks in one transaction.

## 8.2 Claim transaction (conceptual SQL)

```sql
BEGIN;

-- 1) Select candidate rows that are available and not leased
WITH candidates AS (
  SELECT q.run_id, q.namespace, q.task_queue, q.workflow_type
  FROM happyflow.run_queue q
  JOIN happyflow.workflow_runs r ON r.run_id = q.run_id
  WHERE q.namespace = $1
    AND q.task_queue = $2
    AND q.workflow_type = ANY($3)
    AND q.available_at <= now()
    AND (q.leased_until IS NULL OR q.leased_until < now())
    AND r.status IN ('PENDING', 'RUNNING', 'SLEEPING')
  ORDER BY q.priority ASC, q.available_at ASC, q.enqueued_at ASC
  LIMIT $4
  FOR UPDATE SKIP LOCKED
)
SELECT * FROM candidates;

-- 2) Apply limits (namespace/queue/type + worker capacity) in app-layer logic
-- 3) Update selected rows with lease ownership
-- 4) Update workflow_runs lock columns
-- 5) Increment inflight counters

COMMIT;
```

## 8.3 Fairness
- Order by `priority`, then oldest `available_at`, then `enqueued_at`.
- Optional starvation guard: every N claims, force include oldest item globally per queue.

## 9. Concurrency Limits

## 9.1 Limit levels
1. Namespace total inflight
2. Queue inflight
3. Workflow-type inflight
4. Worker local max concurrent

## 9.2 Enforcement
- All checks + increments happen in the same transaction as claim.
- Decrements occur on terminal completion/failure/cancel OR lease expiry recovery.
- Counters are periodic-reconciled by coordinator for correctness drift.

## 10. Notifications vs Polling

## 10.1 Polling
- Long-poll timeout default: 30 seconds.
- If no tasks, return empty and repoll.

## 10.2 Notification hint
- On enqueue/requeue: `SELECT pg_notify('happyflow_work', 'namespace:queue')`.
- Poller also subscribes; on message, it wakes and polls immediately.
- Notify channel is a latency optimization only.

## 11. Scheduling & Backfill

## 11.1 Schedule templates
- Represented by rows in `workflow_runs` with `status in ('SCHEDULED', 'PAUSED')`.
- `available_at` is `next_fire_at`.

## 11.2 Coordinator behavior
- Tick every 5 seconds (configurable).
- For each due schedule:
  - Validate cron.
  - Create one schedule instance run.
  - Advance `available_at` to next fire.
  - Respect `max_catchup` and global backfill rate limit.

## 12. Retry Policy

## 12.1 Default retry
- `maximum_attempts: 5`
- `initial_interval_ms: 1000`
- `backoff_coefficient: 2.0`
- `maximum_interval_ms: 60000`

## 12.2 Behavior
- Retryable failure => transition to `SLEEPING`/`PENDING` with delayed `available_at`.
- Non-retryable or attempts exhausted => `FAILED` terminal.

## 13. Worker Protocol

## 13.1 Lifecycle
1. Register
2. Start heartbeats
3. Poll loop
4. Execute workflow deterministically
5. Complete/fail workflow
6. Deregister (drain or immediate)

## 13.2 Heartbeat
- Interval default 10s.
- Server extends leases for active run IDs.
- If heartbeats stop > stale threshold, worker marked OFFLINE.

## 14. Security Model

## 14.1 Authentication
- gRPC interceptor validates bearer JWT or API key.

## 14.2 Authorization
- Role-based checks:
  - `client`: start/get/list/cancel
  - `worker`: register/poll/step/workflow completion
  - `admin`: workers/health/config endpoints
- Namespace-scoped claims are mandatory.

## 14.3 Networking
- TLS required in production.
- CORS closed by default; explicit allow-list only when browser clients exist.

## 15. Observability

## 15.1 Logs
- JSON structured logs.
- Required fields: `run_id`, `worker_id`, `namespace`, `task_queue`, `workflow_type`.

## 15.2 Metrics
- `poll_requests_total`
- `tasks_claimed_total`
- `task_claim_latency_ms`
- `workflow_state_count{status}`
- `lease_expired_total`
- `schedule_fired_total`

## 15.3 Tracing
- OpenTelemetry spans for gRPC methods and DB transactions.

## 16. Testing Strategy

## 16.1 Required test suites
1. Unit tests: retry math, cron parsing, cursor encoding.
2. Repository integration tests (real Postgres).
3. End-to-end tests with testcontainers:
- start -> poll -> complete
- failure -> retry -> success
- sleep -> wake -> continue
- stale worker -> lease reclaim
- schedule fire + catchup limits

## 16.2 CI gates
- `cargo fmt --check`
- `cargo clippy -D warnings`
- unit + integration + e2e test jobs
- migration smoke test on empty DB

## 17. Implementation Phases

## Phase 1: Vertical slice
- StartWorkflow + RegisterWorker + PollTasks + CompleteWorkflow.
- Minimal schema + lease claim transaction.

## Phase 2: Steps + replay
- Begin/Complete/Fail step APIs.
- Deterministic replay from `step_runs`.

## Phase 3: Retry + sleep
- Retry policy evaluator.
- Sleep transitions and delayed availability.

## Phase 4: Scheduling
- Schedule CRUD + coordinator firing.

## Phase 5: Hardening
- Authz/authn, metrics, reconciliation, backfill protection.

## 18. Simplicity Constraints (must enforce)
- Keep transport types out of store layer.
- Keep SQL in repository files only.
- No module should exceed ~300 LOC without split.
- Prefer explicit code over macro-heavy abstractions.
- Add complexity only with a failing test or measured need.
