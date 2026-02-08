# kagzi-store

PostgreSQL-based data persistence layer for the Kagzi workflow orchestration system. This crate provides a clean Repository pattern abstraction over database operations with comprehensive error handling, concurrency control, and recovery mechanisms.

## Overview

`kagzi-store` is responsible for:

- **Workflow Persistence**: Storing workflow runs, states, and execution metadata
- **Step Execution**: Tracking individual step attempts, retries, and results
- **Worker Management**: Worker registration, heartbeats, and lifecycle
- **Scheduling**: Cron-based workflow scheduling with catchup support
- **Concurrency Control**: Distributed locking, queue management, and crash recovery

## Architecture

The crate follows a layered architecture:

```
┌─────────────────────────────────────────┐
│   Application Layer (kagzi-server)     │
└─────────────────────────────────────────┘
                 ↓
┌─────────────────────────────────────────┐
│      Repository Traits (abstract)      │
│  - WorkflowRepository                  │
│  - StepRepository                      │
│  - WorkerRepository                    │
│  - WorkflowScheduleRepository          │
└─────────────────────────────────────────┘
                 ↓
┌─────────────────────────────────────────┐
│   PostgreSQL Implementations            │
│  - PgWorkflowRepository                 │
│  - PgStepRepository                     │
│  - PgWorkerRepository                   │
│  - PgScheduleRepository                 │
└─────────────────────────────────────────┘
                 ↓
┌─────────────────────────────────────────┐
│         PostgreSQL Database              │
│  - workflow_runs                        │
│  - step_runs                            │
│  - workers                              │
│  - schedules                            │
│  - schedule_firings                     │
│  - concurrency_configs                  │
└─────────────────────────────────────────┘
```

## Data Model

### WorkflowRun

Represents a single workflow execution with full lifecycle tracking.

```rust
pub struct WorkflowRun {
    pub run_id: Uuid,
    pub namespace_id: String,
    pub external_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub input: Vec<u8>,
    pub output: Option<Vec<u8>>,
    pub locked_by: Option<String>,
    pub attempts: i32,
    pub error: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub wake_up_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
    pub parent_step_attempt_id: Option<String>,
    pub retry_policy: Option<RetryPolicy>,
}
```

**Statuses:**

- `Pending` - Waiting to be picked up by a worker
- `Running` - Currently executing (locked by a worker)
- `Sleeping` - Paused (e.g., awaiting child workflow completion)
- `Completed` - Finished successfully
- `Failed` - Failed after exhausting retries
- `Cancelled` - Cancelled by user request

**Key Relationships:**

- One-to-many with `StepRun` (each workflow has multiple steps)
- Optional parent-child relationship via `parent_step_attempt_id`
- Belongs to a `task_queue` and `namespace_id`

### StepRun

Represents a single step execution attempt within a workflow.

```rust
pub struct StepRun {
    pub attempt_id: Uuid,
    pub run_id: Uuid,
    pub step_id: String,
    pub namespace_id: String,
    pub step_kind: StepKind,
    pub attempt_number: i32,
    pub status: StepStatus,
    pub input: Option<Vec<u8>>,
    pub output: Option<Vec<u8>>,
    pub error: Option<String>,
    pub child_workflow_run_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub retry_at: Option<DateTime<Utc>>,
    pub retry_policy: Option<RetryPolicy>,
}
```

**Step Kinds:**

- `Function` - Regular function execution
- `Sleep` - Sleep step (no actual work, just timing)

**Statuses:**

- `Pending` - Waiting to execute
- `Running` - Currently executing
- `Completed` - Finished successfully
- `Failed` - Failed (may retry or be final)

**Key Features:**

- Multiple attempts per step tracked via `attempt_number`
- Idempotency: only the latest attempt (`is_latest = true`) is considered
- Supports child workflows (fan-out/fan-in patterns)
- Retry scheduling with `retry_at` timestamp

### Worker

Represents a worker process that can execute workflows.

```rust
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
    pub queue_concurrency_limit: Option<i32>,
    pub workflow_type_concurrency: Vec<WorkflowTypeConcurrency>,
}
```

**Statuses:**

- `Online` - Actively processing workflows
- `Draining` - Gracefully shutting down (no new workflows)
- `Offline` - Shut down or crashed

**Key Features:**

- Heartbeat-based health monitoring
- Concurrency limits per worker, per queue, and per workflow type
- Labels for flexible worker selection
- Statistics tracking (completed/failed counts)

### Schedule

Represents a cron-based workflow schedule.

```rust
pub struct Schedule {
    pub schedule_id: Uuid,
    pub namespace_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub cron_expr: String,
    pub input: Vec<u8>,
    pub enabled: bool,
    pub max_catchup: i32,
    pub next_fire_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**Key Features:**

- Cron expression support for flexible scheduling
- Catchup mechanism for missed firings (clamped to `max_catchup`)
- Enable/disable control
- Version tracking for workflow evolution

### RetryPolicy

Configures retry behavior for workflows and steps.

```rust
pub struct RetryPolicy {
    pub maximum_attempts: i32,
    pub initial_interval_ms: i64,
    pub backoff_coefficient: f64,
    pub maximum_interval_ms: i64,
    pub non_retryable_errors: Vec<String>,
}
```

**Default:**

- `maximum_attempts: 5`
- `initial_interval_ms: 1000`
- `backoff_coefficient: 2.0`
- `maximum_interval_ms: 60000`

**Behavior:**

- Exponential backoff with jitter to prevent thundering herd
- Non-retryable errors bypass retry logic
- `maximum_attempts < 0` means infinite retries

## Repository Pattern

The Repository pattern provides a clean abstraction between the domain logic and database operations. All repositories are async traits that can be swapped for alternative implementations (e.g., for testing or other databases).

### WorkflowRepository

```rust
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    // CRUD operations
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError>;
    async fn find_by_id(&self, run_id: Uuid, namespace_id: &str) -> Result<Option<WorkflowRun>, StoreError>;
    async fn find_active_by_external_id(&self, namespace_id: &str, external_id: &str, idempotency_suffix: Option<&str>) -> Result<Option<Uuid>, StoreError>;
    async fn list(&self, params: ListWorkflowsParams) -> Result<PaginatedResult<WorkflowRun, WorkflowCursor>, StoreError>;
    async fn count(&self, namespace_id: &str, filter_status: Option<&str>) -> Result<i64, StoreError>;

    // State transitions
    async fn check_exists(&self, run_id: Uuid, namespace_id: &str) -> Result<WorkflowExistsResult, StoreError>;
    async fn check_status(&self, run_id: Uuid, namespace_id: &str) -> Result<WorkflowExistsResult, StoreError>;
    async fn cancel(&self, run_id: Uuid, namespace_id: &str) -> Result<bool, StoreError>;
    async fn complete(&self, run_id: Uuid, output: Vec<u8>) -> Result<(), StoreError>;
    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;
    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError>;

    // Queue management
    async fn poll_workflow(&self, namespace_id: &str, task_queue: &str, worker_id: &str, types: &[String], lock_secs: i64) -> Result<Option<ClaimedWorkflow>, StoreError>;
    async fn extend_worker_locks(&self, worker_id: &str, duration_secs: i64) -> Result<u64, StoreError>;
    async fn extend_locks_for_runs(&self, run_ids: &[Uuid], duration_secs: i64) -> Result<u64, StoreError>;

    // Batch and recovery
    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError>;
    async fn wake_sleeping(&self, batch_size: i32) -> Result<u64, StoreError>;
    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError>;
    async fn find_and_recover_offline_worker_workflows(&self) -> Result<u64, StoreError>;

    // Retry handling
    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError>;
    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;
    async fn mark_exhausted_with_increment(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;
    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError>;
}
```

### StepRepository

```rust
#[async_trait]
pub trait StepRepository: Send + Sync {
    async fn find_by_id(&self, attempt_id: Uuid) -> Result<Option<StepRun>, StoreError>;
    async fn list(&self, params: ListStepsParams) -> Result<PaginatedResult<StepRun, StepCursor>, StoreError>;

    // Lifecycle management
    async fn begin(&self, params: BeginStepParams) -> Result<BeginStepResult, StoreError>;
    async fn complete(&self, run_id: Uuid, step_id: &str, output: Vec<u8>) -> Result<(), StoreError>;
    async fn fail(&self, params: FailStepParams) -> Result<FailStepResult, StoreError>;

    // Retry processing
    async fn process_pending_retries(&self) -> Result<Vec<RetryTriggered>, StoreError>;
}
```

**Idempotency in `begin()`:**

- Returns cached output if step already completed
- Prevents duplicate execution with idempotency support

**Retry logic in `fail()`:**

- Evaluates retry policy
- Schedules retry with exponential backoff
- Returns retry timestamp if scheduled

### WorkerRepository

```rust
#[async_trait]
pub trait WorkerRepository: Send + Sync {
    // Registration and lifecycle
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError>;
    async fn heartbeat(&self, params: WorkerHeartbeatParams) -> Result<bool, StoreError>;
    async fn start_drain(&self, worker_id: Uuid, namespace_id: &str) -> Result<(), StoreError>;
    async fn deregister(&self, worker_id: Uuid, namespace_id: &str) -> Result<(), StoreError>;

    // Queries
    async fn find_by_id(&self, worker_id: Uuid) -> Result<Option<Worker>, StoreError>;
    async fn list(&self, params: ListWorkersParams) -> Result<PaginatedResult<Worker, WorkerCursor>, StoreError>;
    async fn count(&self, namespace_id: &str, task_queue: Option<&str>, filter_status: Option<WorkerStatus>) -> Result<i64, StoreError>;
    async fn count_online(&self, namespace_id: &str, task_queue: &str) -> Result<i64, StoreError>;

    // Maintenance
    async fn mark_stale_offline(&self, threshold_secs: i64) -> Result<u64, StoreError>;
    async fn update_active_count(&self, worker_id: Uuid, namespace_id: &str, delta: i32) -> Result<(), StoreError>;
}
```

**Registration behavior:**

- Uses `ON CONFLICT` to handle worker restarts
- Updates worker metadata on re-registration
- Prevents duplicate workers (same namespace, queue, hostname, pid)

### WorkflowScheduleRepository

```rust
#[async_trait]
pub trait WorkflowScheduleRepository: Send + Sync {
    async fn create(&self, params: CreateSchedule) -> Result<Uuid, StoreError>;
    async fn find_by_id(&self, id: Uuid, namespace_id: &str) -> Result<Option<Schedule>, StoreError>;
    async fn list(&self, params: ListSchedulesParams) -> Result<PaginatedResult<Schedule, ScheduleCursor>, StoreError>;
    async fn update(&self, id: Uuid, namespace_id: &str, params: UpdateSchedule) -> Result<(), StoreError>;
    async fn delete(&self, id: Uuid, namespace_id: &str) -> Result<bool, StoreError>;

    // Scheduling operations
    async fn due_schedules(&self, now: DateTime<Utc>, limit: i64) -> Result<Vec<Schedule>, StoreError>;
    async fn advance_schedule(&self, id: Uuid, last_fired: DateTime<Utc>, next_fire: DateTime<Utc>) -> Result<(), StoreError>;
    async fn record_firing(&self, schedule_id: Uuid, fire_at: DateTime<Utc>, run_id: Uuid) -> Result<(), StoreError>;
}
```

**Due schedule processing:**

- Uses `FOR UPDATE SKIP LOCKED` for concurrent access
- Clamps `max_catchup` to prevent excessive catchup firings
- Records firings to prevent duplicates

## PostgreSQL Implementation

### PgStore

Main entry point for database access:

```rust
#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
    config: StoreConfig,
}

impl PgStore {
    pub fn new(pool: PgPool, config: StoreConfig) -> Self {
        Self { pool, config }
    }

    pub fn workflows(&self) -> PgWorkflowRepository { ... }
    pub fn steps(&self) -> PgStepRepository { ... }
    pub fn workers(&self) -> PgWorkerRepository { ... }
    pub fn schedules(&self) -> PgScheduleRepository { ... }
    pub fn health(&self) -> PgHealthRepository { ... }
}
```

### StoreConfig

Configuration for payload size limits:

```rust
pub struct StoreConfig {
    pub payload_warn_threshold_bytes: usize,
    pub payload_max_size_bytes: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            payload_warn_threshold_bytes: 1024 * 1024,     // 1 MB
            payload_max_size_bytes: 2 * 1024 * 1024,        // 2 MB
        }
    }
}
```

**Purpose:**

- Prevent abuse of Kagzi for blob storage
- Warn when payloads exceed reasonable size
- Reject payloads exceeding maximum size

### Key PostgreSQL Features

#### 1. SKIP LOCKED for Concurrent Polling

Used in `poll_workflow()` and `due_schedules()` to allow multiple workers to poll simultaneously without blocking:

```sql
SELECT run_id, workflow_type, input
FROM kagzi.workflow_runs
WHERE status = 'PENDING' AND task_queue = $1
ORDER BY created_at ASC
FOR UPDATE SKIP LOCKED
LIMIT 1
```

This enables true concurrent processing without worker contention.

#### 2. Worker Locking

Workflows are locked when claimed and must be extended periodically:

```sql
UPDATE kagzi.workflow_runs
SET locked_by = $1, locked_until = NOW() + ($2 * INTERVAL '1 second')
WHERE run_id = $3
```

Expired locks can be stolen by other workers for crash recovery.

#### 3. Orphaned Workflow Recovery

Detects workflows locked by offline workers and recovers them:

```sql
SELECT w.run_id, w.attempts, w.retry_policy
FROM kagzi.workflow_runs w
JOIN kagzi.workers wk ON w.locked_by = wk.worker_id::text
WHERE w.status = 'RUNNING' AND wk.status = 'OFFLINE'
FOR UPDATE SKIP LOCKED
```

Recovered workflows are scheduled for retry or marked as exhausted based on retry policy.

#### 4. Idempotency via External IDs

Prevents duplicate workflow execution:

```sql
SELECT run_id
FROM kagzi.workflow_runs
WHERE external_id = $1
  AND namespace_id = $2
  AND status IN ('PENDING', 'RUNNING', 'SLEEPING')
  AND ($3::TEXT IS NULL OR idempotency_suffix = $3)
```

Allows the same external ID to be used multiple times with different `idempotency_suffix`.

#### 5. Pagination with Cursors

Efficient cursor-based pagination for large datasets:

```sql
SELECT run_id, workflow_type, ...
FROM kagzi.workflow_runs
WHERE namespace_id = $1
  AND (created_at, run_id) > ($2, $3)
ORDER BY created_at ASC, run_id ASC
LIMIT $4
```

Cursor-based pagination avoids OFFSET performance issues on large tables.

## Error Handling

### StoreError

Comprehensive error type covering all failure modes:

```rust
pub enum StoreError {
    Database(sqlx::Error),
    Conflict { message: String },
    InvalidArgument { message: String },
    NotFound { entity: &'static str, id: String },
    InvalidState { message: String },
    AlreadyCompleted { message: String },
    Serialization(serde_json::Error),
    LockConflict { message: String },
    PreconditionFailed { message: String },
    Unauthorized { message: String },
    Unavailable { message: String },
    Timeout { message: String },
}
```

### SQL Error Mapping

PostgreSQL error codes are mapped to domain errors:

- `23505` (unique_violation) → `StoreError::Conflict`
- `23503` (foreign_key_violation) → `StoreError::InvalidArgument`

### Helper Methods

Convenient constructors for common errors:

```rust
StoreError::invalid_argument("message")
StoreError::not_found("workflow", "uuid")
StoreError::invalid_state("message")
StoreError::already_completed("message")
StoreError::lock_conflict("message")
StoreError::precondition_failed("message")
StoreError::unauthorized("message")
StoreError::unavailable("message")
StoreError::timeout("message")
```

## Database Schema

### Core Tables

#### workflow_runs

```sql
CREATE TABLE kagzi.workflow_runs (
    run_id UUID PRIMARY KEY,
    namespace_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL,
    input BYTEA NOT NULL,
    output BYTEA,
    locked_by TEXT,
    locked_until TIMESTAMP,
    attempts INTEGER DEFAULT 0,
    error TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    started_at TIMESTAMP,
    finished_at TIMESTAMP,
    wake_up_at TIMESTAMP,
    deadline_at TIMESTAMP,
    version TEXT,
    parent_step_attempt_id TEXT,
    idempotency_suffix TEXT,
    retry_policy JSONB
);

-- Indexes for query performance
CREATE INDEX idx_workflow_runs_namespace_status ON kagzi.workflow_runs(namespace_id, status);
CREATE INDEX idx_workflow_runs_queue_status ON kagzi.workflow_runs(task_queue, status);
CREATE INDEX idx_workflow_runs_external ON kagzi.workflow_runs(external_id, namespace_id);
CREATE INDEX idx_workflow_runs_wake_up ON kagzi.workflow_runs(wake_up_at) WHERE status = 'SLEEPING';
```

#### step_runs

```sql
CREATE TABLE kagzi.step_runs (
    attempt_id UUID PRIMARY KEY,
    run_id UUID NOT NULL,
    step_id TEXT NOT NULL,
    namespace_id TEXT NOT NULL,
    step_kind TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    status TEXT NOT NULL,
    input BYTEA,
    output BYTEA,
    error TEXT,
    child_workflow_run_id UUID,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    started_at TIMESTAMP,
    finished_at TIMESTAMP,
    retry_at TIMESTAMP,
    retry_policy JSONB,
    is_latest BOOLEAN DEFAULT true
);

CREATE INDEX idx_step_runs_run_id ON kagzi.step_runs(run_id);
CREATE INDEX idx_step_runs_retry_at ON kagzi.step_runs(retry_at) WHERE status = 'PENDING' AND retry_at IS NOT NULL;
```

#### workers

```sql
CREATE TABLE kagzi.workers (
    worker_id UUID PRIMARY KEY,
    namespace_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    status TEXT NOT NULL,
    hostname TEXT,
    pid INTEGER,
    version TEXT,
    workflow_types TEXT[] NOT NULL,
    max_concurrent INTEGER NOT NULL,
    active_count INTEGER DEFAULT 0,
    total_completed BIGINT DEFAULT 0,
    total_failed BIGINT DEFAULT 0,
    registered_at TIMESTAMP NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMP NOT NULL DEFAULT NOW(),
    deregistered_at TIMESTAMP,
    labels JSONB DEFAULT '{}'
);

-- Unique constraint for worker registration
CREATE UNIQUE INDEX idx_workers_unique ON kagzi.workers(namespace_id, task_queue, hostname, pid) WHERE status != 'OFFLINE';

CREATE INDEX idx_workers_queue_status ON kagzi.workers(task_queue, status);
CREATE INDEX idx_workers_heartbeat ON kagzi.workers(last_heartbeat_at) WHERE status != 'OFFLINE';
```

#### schedules

```sql
CREATE TABLE kagzi.schedules (
    schedule_id UUID PRIMARY KEY,
    namespace_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    workflow_type TEXT NOT NULL,
    cron_expr TEXT NOT NULL,
    input BYTEA NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    max_catchup INTEGER NOT NULL DEFAULT 100,
    next_fire_at TIMESTAMP NOT NULL,
    last_fired_at TIMESTAMP,
    version TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_schedules_next_fire ON kagzi.schedules(next_fire_at) WHERE enabled = true;
CREATE INDEX idx_schedules_namespace ON kagzi.schedules(namespace_id);
```

#### schedule_firings

```sql
CREATE TABLE kagzi.schedule_firings (
    schedule_id UUID NOT NULL,
    fire_at TIMESTAMP NOT NULL,
    run_id UUID NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (schedule_id, fire_at)
);
```

#### queue_counters

```sql
CREATE TABLE kagzi.queue_counters (
    task_queue TEXT PRIMARY KEY,
    pending_count INTEGER NOT NULL DEFAULT 0,
    running_count INTEGER NOT NULL DEFAULT 0
);
```

## Migrations

Migrations are located in the project's `migrations/` directory and are versioned by timestamp:

```
migrations/
├── 20251207180000_create_schema.sql
├── 20251207180001_create_workflow_runs.sql
├── 20251207180002_create_workflow_payloads.sql
├── 20251207180003_create_step_runs.sql
├── 20251207180004_create_workers.sql
├── 20251207180005_create_concurrency_configs.sql
├── 20251207180006_create_schedules.sql
├── 20251207180007_drop_create_step_attempt.sql
├── 20251207180008_create_queue_counters.sql
├── 20251208180000_optimize_architecture.sql
├── 20251210190000_payload_to_bytea.sql
├── 20251210190500_partial_indexes.sql
├── 20251210190600_notify_trigger.sql
├── 20251210190750_schedule_input_to_bytea.sql
└── 20251212160422_remove_unused_context_columns.sql
```

Running migrations:

```bash
just migrate
```

Or via sqlx CLI:

```bash
sqlx migrate run --database-url postgres://user:pass@localhost:5432/db
```

## Usage Examples

### Creating a PgStore

```rust
use sqlx::PgPool;
use kagzi_store::{PgStore, StoreConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = PgPool::connect("postgres://user:pass@localhost:5432/kagzi").await?;
    let config = StoreConfig::default();
    let store = PgStore::new(pool, config);

    Ok(())
}
```

### Creating a Workflow

```rust
use kagzi_store::{models::CreateWorkflow, repository::WorkflowRepository};
use chrono::Utc;

let params = CreateWorkflow {
    external_id: "order-123".to_string(),
    task_queue: "orders".to_string(),
    workflow_type: "ProcessOrder".to_string(),
    input: serde_json::to_vec(&order_data)?,
    namespace_id: "default".to_string(),
    idempotency_suffix: None,
    deadline_at: Some(Utc::now() + chrono::Duration::hours(1)),
    version: "1.0.0".to_string(),
    retry_policy: None,
};

let run_id = store.workflows().create(params).await?;
```

### Polling for Work

```rust
use kagzi_store::repository::WorkflowRepository;

let worker_id = "worker-1".to_string();
let workflow_types = vec!["ProcessOrder".to_string(), "ProcessRefund".to_string()];

if let Some(claimed) = store.workflows().poll_workflow(
    "default",
    "orders",
    &worker_id,
    &workflow_types,
    30, // lock for 30 seconds
).await? {
    println!("Claimed workflow: {}", claimed.run_id);
    // Execute workflow...
}
```

### Working with Steps

```rust
use kagzi_store::{models::BeginStepParams, repository::StepRepository};

let begin_params = BeginStepParams {
    run_id,
    step_id: "validate-payment".to_string(),
    step_kind: StepKind::Function,
    input: Some(serde_json::to_vec(&payment_data)?),
    retry_policy: None,
};

let result = store.steps().begin(begin_params).await?;

if result.should_execute {
    // Execute step logic...
    let output = serde_json::to_vec(&step_result)?;
    store.steps().complete(run_id, "validate-payment", output).await?;
} else if let Some(cached) = result.cached_output {
    // Step already completed, use cached output
}
```

### Step Retry Logic

```rust
use kagzi_store::{models::FailStepParams, repository::StepRepository};

let fail_params = FailStepParams {
    run_id,
    step_id: "process-payment".to_string(),
    error: "Payment gateway timeout".to_string(),
    non_retryable: false,
    retry_after_ms: None,
};

let result = store.steps().fail(fail_params).await?;

if result.scheduled_retry {
    println!("Step will retry at: {:?}", result.retry_at);
}
```

### Registering a Worker

```rust
use kagzi_store::{models::RegisterWorkerParams, repository::WorkerRepository};

let params = RegisterWorkerParams {
    namespace_id: "default".to_string(),
    task_queue: "orders".to_string(),
    workflow_types: vec!["ProcessOrder".to_string(), "ProcessRefund".to_string()],
    hostname: Some("worker-1.example.com".to_string()),
    pid: Some(std::process::id()),
    version: Some("1.0.0".to_string()),
    max_concurrent: 10,
    labels: serde_json::json!({"region": "us-east-1"}),
    queue_concurrency_limit: None,
    workflow_type_concurrency: vec![],
};

let worker_id = store.workers().register(params).await?;
```

### Worker Heartbeat

```rust
use kagzi_store::{models::WorkerHeartbeatParams, repository::WorkerRepository};

let heartbeat = WorkerHeartbeatParams {
    worker_id,
    active_count: 5,
    completed_delta: 10,
    failed_delta: 2,
};

store.workers().heartbeat(heartbeat).await?;
```

### Creating a Schedule

```rust
use kagzi_store::{models::CreateSchedule, repository::WorkflowScheduleRepository};
use chrono::Utc;

let params = CreateSchedule {
    namespace_id: "default".to_string(),
    task_queue: "reports".to_string(),
    workflow_type: "GenerateReport".to_string(),
    cron_expr: "0 0 * * *".to_string(), // Daily at midnight
    input: serde_json::to_vec(&report_config)?,
    enabled: true,
    max_catchup: 7, // Catch up to 7 missed executions
    next_fire_at: cron::Schedule::from_str("0 0 * * *")?.next(&Utc::now())?,
    version: Some("1.0.0".to_string()),
};

let schedule_id = store.schedules().create(params).await?;
```

### Processing Due Schedules

```rust
use kagzi_store::repository::WorkflowScheduleRepository;
use chrono::Utc;

let due_schedules = store.schedules().due_schedules(Utc::now(), 100).await?;

for schedule in due_schedules {
    // Create workflow for the schedule
    let workflow_params = CreateWorkflow {
        external_id: format!("schedule-{}", schedule.schedule_id),
        task_queue: schedule.task_queue,
        workflow_type: schedule.workflow_type,
        input: schedule.input,
        namespace_id: schedule.namespace_id,
        idempotency_suffix: Some(schedule.fire_time.to_rfc3339()),
        deadline_at: None,
        version: schedule.version.unwrap_or("1.0.0".to_string()),
        retry_policy: None,
    };

    let run_id = store.workflows().create(workflow_params).await?;

    // Record the firing
    store.schedules().record_firing(schedule.schedule_id, schedule.next_fire_at, run_id).await?;

    // Advance the schedule
    let next_fire = cron::Schedule::from_str(&schedule.cron_expr)?.next(&schedule.next_fire_at)?;
    store.schedules().advance_schedule(schedule.schedule_id, schedule.next_fire_at, next_fire).await?;
}
```

### Crash Recovery

```rust
use kagzi_store::repository::WorkflowRepository;

// Mark stale workers as offline
let offline_count = store.workers().mark_stale_offline(300).await?; // 5 minutes

// Recover workflows from offline workers
let recovered = store.workers().find_and_recover_offline_worker_workflows().await?;

println!("Marked {} workers as offline, recovered {} workflows", offline_count, recovered);
```

### Extending Worker Locks

```rust
use kagzi_store::repository::WorkflowRepository;

// Extend locks for all workflows held by this worker
let extended_count = store.workflows().extend_worker_locks(&worker_id, 30).await?;

// Or extend locks for specific runs
let run_ids = vec![run_id1, run_id2, run_id3];
let extended_count = store.workflows().extend_locks_for_runs(&run_ids, 30).await?;
```

## Query Patterns

### Pagination

All list operations support cursor-based pagination:

```rust
use kagzi_store::{models::ListWorkflowsParams, repository::WorkflowRepository};

let mut cursor = None;
let page_size = 50;

loop {
    let params = ListWorkflowsParams {
        namespace_id: "default".to_string(),
        filter_status: Some("RUNNING".to_string()),
        page_size,
        cursor: cursor.clone(),
    };

    let result = store.workflows().list(params).await?;

    for workflow in result.items {
        println!("{:?}", workflow);
    }

    if !result.has_more {
        break;
    }

    cursor = result.next_cursor;
}
```

### Filtering by Status

```rust
let pending_count = store.workflows().count("default", Some("PENDING")).await?;
let running_count = store.workflows().count("default", Some("RUNNING")).await?;
let failed_count = store.workflows().count("default", Some("FAILED")).await?;
```

### Listing Workers with Filters

```rust
use kagzi_store::{models::{ListWorkersParams, WorkerStatus}, repository::WorkerRepository};

let params = ListWorkersParams {
    namespace_id: "default".to_string(),
    task_queue: Some("orders".to_string()),
    filter_status: Some(WorkerStatus::Online),
    page_size: 100,
    cursor: None,
};

let workers = store.workers().list(params).await?;
```

## Concurrency and Consistency

### Transaction Usage

Critical operations use database transactions to ensure consistency:

```rust
async fn mark_exhausted_with_increment(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
    let mut tx = self.pool.begin().await?;

    sqlx::query!(
        "UPDATE kagzi.workflow_runs SET status = 'FAILED', error = $2, attempts = attempts + 1 WHERE run_id = $1",
        run_id,
        error
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
```

### Distributed Locking

Workflows are locked by workers to prevent duplicate processing:

- Lock is claimed via `poll_workflow()`
- Lock must be extended periodically via `extend_worker_locks()`
- Expired locks can be stolen by other workers
- Locks are released when workflow completes/fails

### Crash Recovery

The system automatically recovers from worker crashes:

1. Workers with stale heartbeats are marked `OFFLINE`
2. Workflows locked by offline workers are detected
3. Based on retry policy, workflows are:
   - Scheduled for retry (if attempts remaining)
   - Marked as exhausted/failed (if retries exhausted)

## Testing

The crate includes unit tests for payload validation:

```bash
cargo test -p kagzi-store
```

Integration tests are located in the parent crate's test directory.

## Performance Considerations

### Indexes

The database schema includes carefully designed indexes:

- Composite indexes for common query patterns (namespace + status, task_queue + status)
- Partial indexes for filtering (e.g., `WHERE enabled = true`)
- Unique constraints for data integrity

### Query Optimization

- `SKIP LOCKED` prevents contention in polling
- Cursor-based pagination avoids `OFFSET` performance issues
- Batch operations for bulk creates/updates
- Prepared statements via sqlx's compile-time verification

### Payload Size

Large payloads should be stored externally (e.g., S3) with only references in Kagzi:

```rust
// Bad: Storing large blobs in Kagzi
let large_blob = vec![0u8; 10 * 1024 * 1024]; // 10 MB
store.workflows().create(CreateWorkflow { input: large_blob, ... }).await?; // Will fail!

// Good: Store externally, reference in Kagzi
let s3_key = "blobs/order-123/data.json";
let reference = serde_json::json!({"s3_key": s3_key}).to_vec();
store.workflows().create(CreateWorkflow { input: reference, ... }).await?;
```

## Dependencies

- `sqlx` - PostgreSQL driver with compile-time query verification
- `async-trait` - Async trait support for Repository pattern
- `tokio` - Async runtime
- `chrono` - Date/time handling
- `uuid` - UUID generation (using v7 for time-ordered IDs)
- `serde` + `serde_json` - Serialization
- `thiserror` - Error handling
- `anyhow` - Error propagation
- `backoff` - Exponential backoff algorithms
- `strum` - Enum string conversion
- `tracing` - Structured logging

## License

This crate is part of the Kagzi project. See the project LICENSE file for details.
