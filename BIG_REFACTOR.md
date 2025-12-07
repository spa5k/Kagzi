# Kagzi Proto/API/SDK Production Refactoring Plan

> **Status**: APPROVED  
> **Breaking Changes**: Allowed (pre-release)  
> **Target**: Production-ready API design

---

## PR Overview

This refactoring is split into **8 independent PRs** that can be reviewed and merged sequentially:

| PR  | Title                              | Scope                        | Risk   |
| --- | ---------------------------------- | ---------------------------- | ------ |
| 1   | Proto Foundation & Common Types    | Proto only                   | Low    |
| 2   | WorkflowService Extraction         | Proto + Server               | Medium |
| 3   | WorkerService Extraction           | Proto + Server + SDK         | Medium |
| 4   | WorkflowScheduleService Extraction | Proto + Server + SDK         | Medium |
| 5   | AdminService Creation              | Proto + Server               | Low    |
| 6   | ID & Idempotency Refactor          | Proto + Store + DB Migration | High   |
| 7   | SDK Refactor                       | SDK only                     | Medium |
| 8   | Cleanup & Testing                  | All                          | Low    |

---

## Summary of Issues

### Naming Inconsistencies

| Issue                   | Current State                              | Impact                                                           |
| ----------------------- | ------------------------------------------ | ---------------------------------------------------------------- |
| `PollActivity` naming   | Polls for workflow tasks, not activities   | Semantic confusion - Kagzi has Workflows → Steps, not activities |
| ID terminology          | `workflow_id` vs `business_id` vs `run_id` | Same concept with different names in different places            |
| Redundant idempotency   | Separate `idempotency_key` field           | `workflow_id` should serve as idempotency key (Temporal pattern) |
| Generic schedule naming | `Schedule`, `ScheduleService`              | Should be `WorkflowSchedule` to clarify relationship             |

### Service Structure

- **Monolithic Service**: Single `WorkflowService` with 20+ RPCs mixing client, worker, admin, and schedule operations
- **No API Versioning**: No strategy for evolving API

### Other Issues

- Wrapper types instead of proto3 `optional`
- Custom `Empty` instead of `google.protobuf.Empty`
- Inconsistent pagination across List operations
- Raw `bytes input` without metadata
- Hacky `StepKind` inference from step_id string

---

## PR 1: Proto Foundation & Common Types

**Goal**: Establish versioning and shared types without changing service structure.

### Changes

#### 1.1 Add API Versioning

All proto files get package version:

```protobuf
syntax = "proto3";
package kagzi.v1;
```

#### 1.2 Create/Update `common.proto`

```protobuf
syntax = "proto3";
package kagzi.v1;

import "google/protobuf/timestamp.proto";

// ============================================
// PAYLOAD - Future-proof data container
// ============================================

message Payload {
  bytes data = 1;
  map<string, string> metadata = 2;  // encoding, content-type, etc.
}

// ============================================
// PAGINATION - Standardized for all List ops
// ============================================

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

// ============================================
// ERRORS - Unified error handling
// ============================================

enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  ERROR_CODE_NOT_FOUND = 1;
  ERROR_CODE_INVALID_ARGUMENT = 2;
  ERROR_CODE_PRECONDITION_FAILED = 3;
  ERROR_CODE_CONFLICT = 4;
  ERROR_CODE_UNAUTHORIZED = 5;
  ERROR_CODE_UNAVAILABLE = 6;
  ERROR_CODE_INTERNAL = 7;
  ERROR_CODE_TIMEOUT = 8;
}

message ErrorDetail {
  ErrorCode code = 1;
  string message = 2;
  bool non_retryable = 3;
  int64 retry_after_ms = 4;
  string subject = 5;
  string subject_id = 6;
  map<string, string> metadata = 7;
}

// ============================================
// RETRY POLICY - Shared across workflows/steps
// ============================================

message RetryPolicy {
  int32 maximum_attempts = 1;
  int64 initial_interval_ms = 2;
  double backoff_coefficient = 3;
  int64 maximum_interval_ms = 4;
  repeated string non_retryable_errors = 5;
}
```

#### 1.3 Delete `errors.proto`

Merged into `common.proto`.

#### 1.4 Delete `kagzi.proto`

Empty umbrella file serves no purpose.

### Files Changed

```
proto/
├── common.proto          # MODIFIED: Add Payload, PageInfo, merge errors
├── errors.proto          # DELETED
├── kagzi.proto           # DELETED
├── workflow.proto        # MODIFIED: Add package kagzi.v1
├── worker.proto          # MODIFIED: Add package kagzi.v1
├── schedule.proto        # MODIFIED: Add package kagzi.v1
└── health.proto          # MODIFIED: Add package kagzi.v1
```

### Checklist

- [x] Add `package kagzi.v1` to all proto files
- [x] Add `Payload` message to `common.proto`
- [x] Add `PageRequest`/`PageInfo` to `common.proto`
- [x] Enhance `ErrorDetail` with metadata field
- [x] Delete `errors.proto` (merge into common)
- [x] Delete `kagzi.proto` (empty umbrella)
- [x] Update `build.rs` if needed
- [x] Verify proto compilation (cargo build -p kagzi-proto, cargo test --all)

---

## PR 2: WorkflowService Extraction

**Goal**: Extract client-facing workflow operations into clean `WorkflowService`.

### Changes

#### 2.1 Update `workflow.proto`

```protobuf
syntax = "proto3";
package kagzi.v1;

import "common.proto";
import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";

// ============================================
// SERVICE
// ============================================

service WorkflowService {
  rpc StartWorkflow(StartWorkflowRequest) returns (StartWorkflowResponse);
  rpc GetWorkflow(GetWorkflowRequest) returns (GetWorkflowResponse);
  rpc ListWorkflows(ListWorkflowsRequest) returns (ListWorkflowsResponse);
  rpc CancelWorkflow(CancelWorkflowRequest) returns (google.protobuf.Empty);
}

// ============================================
// ENUMS
// ============================================

enum WorkflowStatus {
  WORKFLOW_STATUS_UNSPECIFIED = 0;
  WORKFLOW_STATUS_PENDING = 1;
  WORKFLOW_STATUS_RUNNING = 2;
  WORKFLOW_STATUS_SLEEPING = 3;
  WORKFLOW_STATUS_COMPLETED = 4;
  WORKFLOW_STATUS_FAILED = 5;
  WORKFLOW_STATUS_CANCELLED = 6;
}

// ============================================
// MESSAGES
// ============================================

message Workflow {
  string run_id = 1;
  string workflow_id = 2;  // Renamed from business_id
  string namespace_id = 3;
  string task_queue = 4;
  string workflow_type = 5;
  WorkflowStatus status = 6;
  Payload input = 7;
  Payload output = 8;
  Payload context = 9;
  ErrorDetail error = 10;
  int32 attempts = 11;
  google.protobuf.Timestamp created_at = 12;
  google.protobuf.Timestamp started_at = 13;
  google.protobuf.Timestamp finished_at = 14;
  google.protobuf.Timestamp wake_up_at = 15;
  google.protobuf.Timestamp deadline_at = 16;
  string worker_id = 17;
  string version = 18;
  string parent_step_id = 19;
}

// --- Requests/Responses ---

message StartWorkflowRequest {
  string workflow_id = 1;  // User-provided, acts as idempotency key
  string namespace_id = 2;
  string task_queue = 3;
  string workflow_type = 4;
  Payload input = 5;
  Payload context = 6;
  google.protobuf.Timestamp deadline_at = 7;
  string version = 8;
  RetryPolicy retry_policy = 9;
}

message StartWorkflowResponse {
  string run_id = 1;
  bool already_exists = 2;  // True if returned existing workflow
}

message GetWorkflowRequest {
  string run_id = 1;
  string namespace_id = 2;
}

message GetWorkflowResponse {
  Workflow workflow = 1;
}

message ListWorkflowsRequest {
  string namespace_id = 1;
  optional WorkflowStatus status_filter = 2;
  PageRequest page = 3;
}

message ListWorkflowsResponse {
  repeated Workflow workflows = 1;
  PageInfo page = 2;
}

message CancelWorkflowRequest {
  string run_id = 1;
  string namespace_id = 2;
}
```

#### 2.2 Create `workflow_service.rs`

Extract workflow-related handlers from `service.rs`.

### Renames in This PR

| Old                       | New                    |
| ------------------------- | ---------------------- |
| `WorkflowRun`             | `Workflow`             |
| `GetWorkflowRun`          | `GetWorkflow`          |
| `ListWorkflowRuns`        | `ListWorkflows`        |
| `CancelWorkflowRun`       | `CancelWorkflow`       |
| `GetWorkflowRunRequest`   | `GetWorkflowRequest`   |
| `ListWorkflowRunsRequest` | `ListWorkflowsRequest` |

### Files Changed

```
proto/workflow.proto                           # MODIFIED
crates/kagzi-server/src/workflow_service.rs    # NEW
crates/kagzi-server/src/service.rs             # MODIFIED: Workflow RPCs removed, legacy service gated behind feature
crates/kagzi-server/src/main.rs                # MODIFIED: Register WorkflowServiceImpl
crates/kagzi-server/Cargo.toml                 # MODIFIED: Add legacy-service feature flag
```

### Checklist

- [x] Update `workflow.proto` with new structure
- [x] Create `workflow_service.rs` with extracted handlers
- [x] Remove workflow RPCs from old `service.rs`
- [x] Register `WorkflowService` in `main.rs`
- [x] Update proto imports
- [ ] Test workflow operations (grpcurl/manual)

**Progress (Dec 2025):** PR2 implemented in codebase. Workflow RPCs extracted to dedicated `WorkflowServiceImpl`; monolithic service is gated by `legacy-service` feature. Builds pass (`cargo build -p kagzi-proto -p kagzi-server`). Non-workflow RPCs are temporarily unavailable until PR3-5 land.

---

## PR 3: WorkerService Extraction

**Goal**: Extract worker lifecycle and execution operations.

### Changes

#### 3.1 Update `worker.proto`

```protobuf
syntax = "proto3";
package kagzi.v1;

import "common.proto";
import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";

// ============================================
// SERVICE
// ============================================

service WorkerService {
  // Lifecycle
  rpc Register(RegisterRequest) returns (RegisterResponse);
  rpc Heartbeat(HeartbeatRequest) returns (HeartbeatResponse);
  rpc Deregister(DeregisterRequest) returns (google.protobuf.Empty);

  // Execution
  rpc PollTask(PollTaskRequest) returns (PollTaskResponse);  // Renamed from PollActivity
  rpc BeginStep(BeginStepRequest) returns (BeginStepResponse);
  rpc CompleteStep(CompleteStepRequest) returns (CompleteStepResponse);
  rpc FailStep(FailStepRequest) returns (FailStepResponse);
  rpc CompleteWorkflow(CompleteWorkflowRequest) returns (CompleteWorkflowResponse);
  rpc FailWorkflow(FailWorkflowRequest) returns (FailWorkflowResponse);
  rpc Sleep(SleepRequest) returns (google.protobuf.Empty);
}

// ============================================
// ENUMS
// ============================================

enum WorkerStatus {
  WORKER_STATUS_UNSPECIFIED = 0;
  WORKER_STATUS_ONLINE = 1;
  WORKER_STATUS_DRAINING = 2;
  WORKER_STATUS_OFFLINE = 3;
}

enum StepKind {
  STEP_KIND_UNSPECIFIED = 0;
  STEP_KIND_FUNCTION = 1;
  STEP_KIND_SLEEP = 2;
}

enum StepStatus {
  STEP_STATUS_UNSPECIFIED = 0;
  STEP_STATUS_PENDING = 1;
  STEP_STATUS_RUNNING = 2;
  STEP_STATUS_COMPLETED = 3;
  STEP_STATUS_FAILED = 4;
}

// ============================================
// MESSAGES
// ============================================

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
  optional int32 queue_concurrency_limit = 16;
  repeated WorkflowTypeConcurrency workflow_type_concurrency = 17;
}

message WorkflowTypeConcurrency {
  string workflow_type = 1;
  int32 max_concurrent = 2;
}

message Step {
  string step_id = 1;
  string run_id = 2;
  string namespace_id = 3;
  string name = 4;
  StepKind kind = 5;
  StepStatus status = 6;
  int32 attempt_number = 7;
  Payload input = 8;
  Payload output = 9;
  ErrorDetail error = 10;
  google.protobuf.Timestamp created_at = 11;
  google.protobuf.Timestamp started_at = 12;
  google.protobuf.Timestamp finished_at = 13;
  optional string child_run_id = 14;
}

// --- Lifecycle Requests ---

message RegisterRequest {
  string namespace_id = 1;
  string task_queue = 2;
  repeated string workflow_types = 3;
  string hostname = 4;
  int32 pid = 5;
  string version = 6;
  int32 max_concurrent = 7;
  map<string, string> labels = 8;
  optional int32 queue_concurrency_limit = 9;
  repeated WorkflowTypeConcurrency workflow_type_concurrency = 10;
}

message RegisterResponse {
  string worker_id = 1;
  int32 heartbeat_interval_secs = 2;
}

message HeartbeatRequest {
  string worker_id = 1;
  int32 active_count = 2;
  int32 completed_delta = 3;
  int32 failed_delta = 4;
}

message HeartbeatResponse {
  bool accepted = 1;
  bool should_drain = 2;
}

message DeregisterRequest {
  string worker_id = 1;
  bool drain = 2;
}

// --- Execution Requests ---

message PollTaskRequest {
  string worker_id = 1;
  string namespace_id = 2;
  string task_queue = 3;
  repeated string workflow_types = 4;
}

message PollTaskResponse {
  string run_id = 1;
  string workflow_type = 2;
  Payload input = 3;
}

message BeginStepRequest {
  string run_id = 1;
  string step_name = 2;
  StepKind kind = 3;  // NEW: Explicit, no more inference
  Payload input = 4;
  RetryPolicy retry_policy = 5;
}

message BeginStepResponse {
  string step_id = 1;
  bool should_execute = 2;
  Payload cached_output = 3;
}

message CompleteStepRequest {
  string run_id = 1;
  string step_id = 2;
  Payload output = 3;
}

message CompleteStepResponse {
  Step step = 1;
}

message FailStepRequest {
  string run_id = 1;
  string step_id = 2;
  ErrorDetail error = 3;
}

message FailStepResponse {
  bool scheduled_retry = 1;
  google.protobuf.Timestamp retry_at = 2;
}

message CompleteWorkflowRequest {
  string run_id = 1;
  Payload output = 2;
}

message CompleteWorkflowResponse {
  WorkflowStatus status = 1;
}

message FailWorkflowRequest {
  string run_id = 1;
  ErrorDetail error = 2;
}

message FailWorkflowResponse {
  WorkflowStatus status = 1;
}

message SleepRequest {
  string run_id = 1;
  string step_id = 2;
  uint64 duration_seconds = 3;
}
```

#### 3.2 Create `worker_service.rs`

### Key Renames

| Old                    | New                    |
| ---------------------- | ---------------------- |
| `PollActivity`         | `PollTask`             |
| `PollActivityRequest`  | `PollTaskRequest`      |
| `PollActivityResponse` | `PollTaskResponse`     |
| `RegisterWorker`       | `Register`             |
| `WorkerHeartbeat`      | `Heartbeat`            |
| `DeregisterWorker`     | `Deregister`           |
| `ScheduleSleep`        | `Sleep`                |
| `StepAttempt`          | `Step`                 |
| `workflow_input`       | `input` (Payload type) |

### Files Changed

```
proto/worker.proto                           # MODIFIED
crates/kagzi-server/src/worker_service.rs    # NEW
crates/kagzi-server/src/service.rs           # MODIFIED: Remove worker RPCs
crates/kagzi-server/src/main.rs              # MODIFIED: Register WorkerService
crates/kagzi/src/lib.rs                      # MODIFIED: Update SDK for PollTask
```

### Checklist

- [x] Update `worker.proto` with new structure
- [x] Create `worker_service.rs`
- [x] Add explicit `StepKind` to `BeginStepRequest`
- [x] Update SDK to set `StepKind` explicitly
- [x] Remove hacky step_id string inference
- [ ] Test worker operations

**Progress (Dec 2025):** PR3 implemented. `WorkerServiceImpl` is registered in the server; SDK now targets `WorkerService` (PollTask/BeginStep/CompleteStep/FailStep/Sleep) with explicit `StepKind` and payloads. Legacy schedule client helpers were removed from the SDK until PR4 introduces `WorkflowScheduleService`.

---

## PR 4: WorkflowScheduleService Extraction

**Goal**: Create dedicated service for scheduled workflows with explicit naming.

### Changes

#### 4.1 Create `workflow_schedule.proto`

```protobuf
syntax = "proto3";
package kagzi.v1;

import "common.proto";
import "google/protobuf/timestamp.proto";

// ============================================
// SERVICE
// ============================================

service WorkflowScheduleService {
  rpc CreateWorkflowSchedule(CreateWorkflowScheduleRequest) returns (CreateWorkflowScheduleResponse);
  rpc GetWorkflowSchedule(GetWorkflowScheduleRequest) returns (GetWorkflowScheduleResponse);
  rpc ListWorkflowSchedules(ListWorkflowSchedulesRequest) returns (ListWorkflowSchedulesResponse);
  rpc UpdateWorkflowSchedule(UpdateWorkflowScheduleRequest) returns (UpdateWorkflowScheduleResponse);
  rpc DeleteWorkflowSchedule(DeleteWorkflowScheduleRequest) returns (DeleteWorkflowScheduleResponse);
}

// ============================================
// MESSAGES
// ============================================

message WorkflowSchedule {
  string schedule_id = 1;
  string namespace_id = 2;
  string task_queue = 3;
  string workflow_type = 4;
  string cron_expr = 5;
  Payload input = 6;
  Payload context = 7;
  bool enabled = 8;
  int32 max_catchup = 9;
  google.protobuf.Timestamp next_fire_at = 10;
  google.protobuf.Timestamp last_fired_at = 11;
  string version = 12;
  google.protobuf.Timestamp created_at = 13;
  google.protobuf.Timestamp updated_at = 14;
}

// --- Requests/Responses ---

message CreateWorkflowScheduleRequest {
  string namespace_id = 1;
  string task_queue = 2;
  string workflow_type = 3;
  string cron_expr = 4;
  Payload input = 5;
  Payload context = 6;
  optional bool enabled = 7;
  optional int32 max_catchup = 8;
  optional string version = 9;
}

message CreateWorkflowScheduleResponse {
  WorkflowSchedule schedule = 1;
}

message GetWorkflowScheduleRequest {
  string schedule_id = 1;
  string namespace_id = 2;
}

message GetWorkflowScheduleResponse {
  WorkflowSchedule schedule = 1;
}

message ListWorkflowSchedulesRequest {
  string namespace_id = 1;
  optional string task_queue = 2;
  PageRequest page = 3;
}

message ListWorkflowSchedulesResponse {
  repeated WorkflowSchedule schedules = 1;
  PageInfo page = 2;
}

message UpdateWorkflowScheduleRequest {
  string schedule_id = 1;
  string namespace_id = 2;
  optional string task_queue = 3;
  optional string workflow_type = 4;
  optional string cron_expr = 5;
  optional Payload input = 6;
  optional Payload context = 7;
  optional bool enabled = 8;
  optional int32 max_catchup = 9;
  optional google.protobuf.Timestamp next_fire_at = 10;
  optional string version = 11;
}

message UpdateWorkflowScheduleResponse {
  WorkflowSchedule schedule = 1;
}

message DeleteWorkflowScheduleRequest {
  string schedule_id = 1;
  string namespace_id = 2;
}

message DeleteWorkflowScheduleResponse {
  bool deleted = 1;
}
```

#### 4.2 Delete `schedule.proto`

Replaced by `workflow_schedule.proto`.

### Key Renames

| Old               | New                       |
| ----------------- | ------------------------- |
| `Schedule`        | `WorkflowSchedule`        |
| `ScheduleService` | `WorkflowScheduleService` |
| `CreateSchedule`  | `CreateWorkflowSchedule`  |
| All schedule ops  | `*WorkflowSchedule*`      |

### Files Changed

```
proto/workflow_schedule.proto                          # NEW
proto/schedule.proto                                   # DELETED
crates/kagzi-server/src/workflow_schedule_service.rs   # NEW
crates/kagzi-server/src/lib.rs                         # MODIFIED: export service impl
crates/kagzi-server/src/main.rs                        # MODIFIED: register service
crates/kagzi-proto/build.rs                            # MODIFIED: compile new proto
crates/kagzi-store/src/models/workflow_schedule.rs     # RENAMED module
crates/kagzi-store/src/repository/workflow_schedule.rs # RENAMED module
crates/kagzi-store/src/postgres/workflow_schedule.rs   # RENAMED module
crates/kagzi-server/src/scheduler.rs                   # MODIFIED: aliases WorkflowSchedule
crates/kagzi/src/lib.rs                                # MODIFIED: workflow_schedule client helpers
```

### Checklist

- [x] Create `workflow_schedule.proto`
- [x] Create `workflow_schedule_service.rs`
- [x] Delete old `schedule.proto`
- [x] Rename store models and repository
- [x] Update scheduler to use new naming
- [x] Update SDK `schedule()` → `workflow_schedule()`

**Progress (Dec 2025):** PR4 implemented. `WorkflowScheduleServiceImpl` is registered and builds against `workflow_schedule.proto`; store modules renamed to `workflow_schedule`, scheduler uses `WorkflowSchedule` alias, and SDK exposes `workflow_schedule()` builder plus get/list/delete helpers. Legacy monolithic schedule RPCs remain gated behind the legacy feature.

---

## PR 5: AdminService Creation

**Goal**: Create dedicated service for discovery, health, and server info.

### Changes

#### 5.1 Create `admin.proto`

```protobuf
syntax = "proto3";
package kagzi.v1;

import "common.proto";
import "worker.proto";
import "google/protobuf/timestamp.proto";

// ============================================
// SERVICE
// ============================================

service AdminService {
  // Discovery
  rpc ListWorkers(ListWorkersRequest) returns (ListWorkersResponse);
  rpc GetWorker(GetWorkerRequest) returns (GetWorkerResponse);
  rpc GetStep(GetStepRequest) returns (GetStepResponse);
  rpc ListSteps(ListStepsRequest) returns (ListStepsResponse);

  // Health & Info
  rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);
  rpc GetServerInfo(GetServerInfoRequest) returns (GetServerInfoResponse);
}

// ============================================
// DISCOVERY MESSAGES
// ============================================

message ListWorkersRequest {
  string namespace_id = 1;
  optional string task_queue = 2;
  optional WorkerStatus status_filter = 3;
  PageRequest page = 4;
}

message ListWorkersResponse {
  repeated Worker workers = 1;
  PageInfo page = 2;
}

message GetWorkerRequest {
  string worker_id = 1;
}

message GetWorkerResponse {
  Worker worker = 1;
}

message GetStepRequest {
  string step_id = 1;
  string namespace_id = 2;
}

message GetStepResponse {
  Step step = 1;
}

message ListStepsRequest {
  string run_id = 1;
  optional string step_name = 2;
  PageRequest page = 3;
}

message ListStepsResponse {
  repeated Step steps = 1;
  PageInfo page = 2;
}

// ============================================
// HEALTH & INFO MESSAGES
// ============================================

message HealthCheckRequest {
  optional string service = 1;
}

message HealthCheckResponse {
  enum ServingStatus {
    SERVING_STATUS_UNSPECIFIED = 0;
    SERVING = 1;
    NOT_SERVING = 2;
  }
  ServingStatus status = 1;
  string message = 2;
  google.protobuf.Timestamp timestamp = 3;
}

message GetServerInfoRequest {}

message GetServerInfoResponse {
  string version = 1;
  string api_version = 2;
  repeated string supported_features = 3;
  string min_sdk_version = 4;
}
```

#### 5.2 Delete `health.proto`

Merged into `admin.proto`.

### Key Renames

| Old                | New         |
| ------------------ | ----------- |
| `GetStepAttempt`   | `GetStep`   |
| `ListStepAttempts` | `ListSteps` |

### Files Changed

```
proto/admin.proto                           # NEW
proto/health.proto                          # DELETED
crates/kagzi-server/src/admin_service.rs    # NEW
crates/kagzi-server/src/main.rs             # MODIFIED: Register AdminService
```

### Checklist

- [ ] Create `admin.proto`
- [ ] Create `admin_service.rs`
- [ ] Implement `GetServerInfo` RPC
- [ ] Delete `health.proto`
- [ ] Register `AdminService` in `main.rs`

---

## PR 6: ID & Idempotency Refactor

**Goal**: Unify workflow identification and make `workflow_id` the idempotency key.

### Changes

#### 6.1 Database Migration

```sql
-- Migration: 20251207_workflow_id_refactor.sql

-- Step 1: Rename column
ALTER TABLE workflow_runs RENAME COLUMN business_id TO workflow_id;

-- Step 2: Drop old indexes
DROP INDEX IF EXISTS idx_workflow_runs_business_id;
DROP INDEX IF EXISTS idx_workflow_runs_idempotency;

-- Step 3: Create new index
CREATE INDEX idx_workflow_runs_workflow_id ON workflow_runs(workflow_id);

-- Step 4: Add idempotency constraint (only one active workflow per workflow_id per namespace)
CREATE UNIQUE INDEX uq_active_workflow_per_namespace
ON workflow_runs(workflow_id, namespace_id)
WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED');

-- Step 5: Remove idempotency_key column
ALTER TABLE workflow_runs DROP COLUMN IF EXISTS idempotency_key;
```

#### 6.2 Update Store Models

```rust
// crates/kagzi-store/src/models/workflow.rs

pub struct WorkflowRun {
    pub run_id: Uuid,
    pub workflow_id: String,  // Renamed from business_id
    pub namespace_id: String,
    // ... rest unchanged
    // REMOVED: idempotency_key
}

pub struct CreateWorkflow {
    pub workflow_id: String,  // Renamed from business_id
    // REMOVED: idempotency_key
    // ...
}
```

#### 6.3 Update Proto

Already done in PR 2, but ensure `StartWorkflowRequest` uses `workflow_id` as idempotency key:

```protobuf
message StartWorkflowRequest {
  // User-provided business identifier. Acts as idempotency key.
  // If a workflow with this ID is already running, returns existing run_id.
  // If empty, server generates UUID.
  string workflow_id = 1;
  // ...
}

message StartWorkflowResponse {
  string run_id = 1;
  bool already_exists = 2;  // True if returned existing workflow due to idempotency
}
```

### Files Changed

```
migrations/20251207_workflow_id_refactor.sql           # NEW
crates/kagzi-store/src/models/workflow.rs              # MODIFIED
crates/kagzi-store/src/postgres/workflow.rs            # MODIFIED
crates/kagzi-server/src/workflow_service.rs            # MODIFIED: Idempotency logic
crates/kagzi-server/src/scheduler.rs                   # MODIFIED: Use workflow_id
```

### Checklist

- [ ] Create database migration
- [ ] Rename `business_id` → `workflow_id` in store models
- [ ] Remove `idempotency_key` field
- [ ] Update queries to use `workflow_id` for idempotency
- [ ] Update `StartWorkflow` to return `already_exists` flag
- [ ] Update scheduler to use `workflow_id`
- [ ] Test idempotency behavior

---

## PR 7: SDK Refactor

**Goal**: Update SDK to match new API, improve ergonomics.

### Changes

#### 7.1 Fix Hardcoded Namespace

```rust
// Before
impl<'a, I: Serialize> WorkflowBuilder<'a, I> {
    async fn execute(self) -> anyhow::Result<String> {
        // namespace_id: "default".to_string(),  // HARDCODED!
    }
}

// After
pub struct WorkflowBuilder<'a, I> {
    namespace_id: String,
    // ...
}

impl<'a, I: Serialize> WorkflowBuilder<'a, I> {
    pub fn new(...) -> Self {
        Self {
            namespace_id: "default".to_string(),  // Default, but configurable
            // ...
        }
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace_id = ns.into();
        self
    }
}
```

#### 7.2 Remove `.idempotent()` Method

```rust
// Before
client.workflow("type", "queue", input)
    .id("order-123")
    .idempotent("order-123-key")  // REMOVED
    .await?;

// After
client.workflow("type", "queue", input)
    .id("order-123")  // This IS the idempotency key now
    .await?;
```

#### 7.3 Explicit StepKind

```rust
impl WorkflowContext {
    pub async fn run<R, Fut>(&mut self, step_name: &str, fut: Fut) -> anyhow::Result<R> {
        let request = BeginStepRequest {
            run_id: self.run_id.clone(),
            step_name: step_name.to_string(),
            kind: StepKind::Function as i32,  // Explicit
            // ...
        };
        // ...
    }

    pub async fn sleep(&mut self, duration: Duration) -> anyhow::Result<()> {
        let request = BeginStepRequest {
            // ...
            kind: StepKind::Sleep as i32,  // Explicit
            // ...
        };
        // ...
    }
}
```

#### 7.4 Rename Schedule Methods

```rust
// Before
client.schedule("type", "queue", "0 * * * *")

// After
client.workflow_schedule("type", "queue", "0 * * * *")
```

#### 7.5 Update Error Type

```rust
#[derive(Debug, Clone)]
pub struct KagziError {
    pub code: ErrorCode,
    pub message: String,
    pub non_retryable: bool,
    pub retry_after: Option<Duration>,
    pub subject: Option<String>,
    pub subject_id: Option<String>,
    pub metadata: HashMap<String, String>,  // NEW
}
```

### Files Changed

```
crates/kagzi/src/lib.rs              # MODIFIED: All SDK changes
crates/kagzi/src/tracing_utils.rs    # MODIFIED if needed
```

### Checklist

- [ ] Add `.namespace()` to `WorkflowBuilder`
- [ ] Remove `.idempotent()` method
- [ ] Update `WorkflowContext.run()` to set `StepKind::Function`
- [ ] Update `WorkflowContext.sleep()` to set `StepKind::Sleep`
- [ ] Rename `schedule()` → `workflow_schedule()`
- [ ] Add `metadata` field to `KagziError`
- [ ] Update all proto type imports

---

## PR 8: Cleanup & Testing

**Goal**: Final cleanup, delete old code, update all tests and examples.

### Changes

#### 8.1 Delete Old Service File

Remove the now-empty `service.rs` (all RPCs moved to dedicated services).

#### 8.2 Update Examples

```rust
// crates/examples/examples/simple.rs

// Before
let mut worker = Worker::builder("http://localhost:50051", "default")
    // ...

// After
let mut worker = Worker::builder("http://localhost:50051", "default")
    .namespace("production")  // Now configurable
    // ...
```

#### 8.3 Update Integration Tests

Update all tests in `crates/tests/` for new API.

### Files Changed

```
crates/kagzi-server/src/service.rs          # DELETED (empty after extraction)
crates/examples/examples/*.rs               # MODIFIED: Update all examples
crates/tests/tests/*.rs                     # MODIFIED: Update all tests
```

### Checklist

- [ ] Delete empty `service.rs`
- [ ] Update `simple.rs` example
- [ ] Update `comprehensive_demo.rs` example
- [ ] Update `traced_workflow.rs` example
- [ ] Update all integration tests
- [ ] Run full test suite
- [ ] Update README if needed

---

## Final Proto File Structure

After all PRs are merged:

```
proto/
├── common.proto              # Payload, PageInfo, ErrorDetail, RetryPolicy
├── workflow.proto            # WorkflowService
├── worker.proto              # WorkerService, Step, Worker
├── workflow_schedule.proto   # WorkflowScheduleService
└── admin.proto               # AdminService, HealthCheck, ServerInfo
```

## Final Service Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                        kagzi.v1                               │
├───────────────┬───────────────┬───────────────┬──────────────┤
│WorkflowService│ WorkerService │WorkflowSchedule│ AdminService │
│               │               │    Service     │              │
│• Start        │• Register     │• Create        │• ListWorkers │
│• Get          │• Heartbeat    │• Get           │• GetWorker   │
│• List         │• Deregister   │• List          │• GetStep     │
│• Cancel       │• PollTask     │• Update        │• ListSteps   │
│               │• BeginStep    │• Delete        │• HealthCheck │
│               │• CompleteStep │                │• ServerInfo  │
│               │• FailStep     │                │              │
│               │• Complete/Fail│                │              │
│               │• Sleep        │                │              │
└───────────────┴───────────────┴───────────────┴──────────────┘
```

---

_Last Updated: December 2024_
