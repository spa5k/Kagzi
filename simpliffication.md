# Kagzi Scheduling Simplification Plan

## Goal

Simplify Kagzi's scheduling architecture from a complex multi-component system to a minimal, OpenWorkflow-inspired design.

| Before                                      | After                                        |
| ------------------------------------------- | -------------------------------------------- |
| Scheduler + Watchdog + Queue Listener       | Coordinator + Queue Listener                 |
| `wake_up_at`, `locked_until`, `retry_at`    | `available_at`                               |
| Complex poll query (3 timestamp conditions) | Simple `available_at <= NOW()`               |
| Heartbeat-based lock extension              | No lock extension (visibility timeout model) |
| Step-level retries                          | Workflow replay with memoization             |

---

## Architecture

### Design Principle

```
kagzi-queue = Wake-up signal (pg_notify via LISTEN/NOTIFY)
PostgreSQL  = Source of truth (FOR UPDATE SKIP LOCKED)
Server      = Mediates all worker ↔ database communication
```

### Component Roles

| Component       | Role                                                                      |
| --------------- | ------------------------------------------------------------------------- |
| **Worker**      | gRPC client that polls server for work, executes workflow code            |
| **Server**      | gRPC server that handles all database operations                          |
| **kagzi-queue** | LISTEN/NOTIFY layer - wakes up long-polling workers when new work arrives |
| **PostgreSQL**  | Stores workflow state, step results, worker registry                      |
| **Coordinator** | Background task in server - fires cron schedules, marks stale workers     |

### Request Flow

```
  Client           Server (gRPC)        kagzi-queue       PostgreSQL
    │                   │                    │                 │
    │ CreateWorkflow()  │                    │                 │
    │──────────────────>│                    │                 │
    │                   │ INSERT workflow_runs                 │
    │                   │────────────────────────────────────>│
    │                   │                    │                 │
    │                   │ notify()           │                 │
    │                   │───────────────────>│                 │
    │                   │                    │ pg_notify()     │
    │                   │                    │────────────────>│
    │                   │                    │                 │


  Worker             Server (gRPC)        kagzi-queue       PostgreSQL
    │                    │                    │                 │
    │ PollTask()         │                    │                 │
    │───────────────────>│                    │                 │
    │                    │ subscribe()        │                 │
    │                    │<──────────────────>│ (LISTEN)        │
    │                    │                    │                 │
    │                    │ poll_workflow()    │                 │
    │                    │ (FOR UPDATE SKIP LOCKED)             │
    │                    │─────────────────────────────────────>│
    │                    │<─────────────────────────────────────│
    │<───────────────────│                    │                 │
    │                    │                    │                 │
    │ BeginStep()        │                    │                 │
    │───────────────────>│ INSERT step_runs   │                 │
    │                    │─────────────────────────────────────>│
    │<───────────────────│                    │                 │
    │                    │                    │                 │
    │ CompleteStep()     │                    │                 │
    │───────────────────>│ UPDATE step_runs   │                 │
    │                    │─────────────────────────────────────>│
    │<───────────────────│                    │                 │
    │                    │                    │                 │
    │ CompleteWorkflow() │                    │                 │
    │───────────────────>│ UPDATE workflow_runs                 │
    │                    │─────────────────────────────────────>│
```

> **Important**: Workers never access the database directly. All database operations go through the server via gRPC.

### Key Simplification: No Lock Extension

Workers claim workflows with a fixed visibility timeout. If workflow execution exceeds this timeout:

- `available_at` expires
- Another worker can claim the workflow
- New worker replays from beginning, using cached step results

This eliminates the heartbeat → lock extension coupling entirely.

---

## Phase 1: Fix LISTEN/NOTIFY Channel Mismatch

### Problem

Trigger sends to wrong channel:

```sql
-- schema.sql line 51 (WRONG)
pg_notify('kagzi_work_' || md5(NEW.namespace_id || '_' || NEW.task_queue), ...);
```

Listener expects:

```rust
// postgres.rs
listener.listen("kagzi_work").await?;
```

### Solution

Remove the trigger entirely. Rely on explicit `notify()` calls in application code.

### Files to Change

#### [NEW] migrations/XXXXXX_remove_work_trigger.sql

```sql
DROP TRIGGER IF EXISTS trigger_workflow_new_work ON kagzi.workflow_runs;
DROP FUNCTION IF EXISTS kagzi.notify_new_work();
```

#### [MODIFY] [schema.sql](file:///Users/atlantic/Developer/Kagzi/schema.sql)

Remove lines 47-54 (function definition) and lines 401-404 (trigger definition).

---

## Phase 2: Unify Timestamps → `available_at`

### Current State

| Column         | Table         | Purpose                           |
| -------------- | ------------- | --------------------------------- |
| `wake_up_at`   | workflow_runs | Sleep timer                       |
| `locked_until` | workflow_runs | Heartbeat lease                   |
| `retry_at`     | step_runs     | Step retry time                   |
| `deadline_at`  | workflow_runs | Workflow deadline (to be removed) |

### New Model

Single `available_at` column controls when workflow is claimable:

| Scenario          | `available_at` value                           |
| ----------------- | ---------------------------------------------- |
| New workflow      | `NOW()` (immediately available)                |
| Running (claimed) | `NOW() + visibility_timeout` (e.g., 5 minutes) |
| Sleeping          | `NOW() + sleep_duration`                       |
| Retry scheduled   | `NOW() + backoff_delay`                        |

### Files to Change

#### [NEW] migrations/XXXXXX_unify_timestamps.sql

```sql
-- Add new column
ALTER TABLE kagzi.workflow_runs ADD COLUMN available_at TIMESTAMPTZ;

-- Populate from existing data
UPDATE kagzi.workflow_runs SET available_at =
    CASE
        WHEN status = 'PENDING' THEN COALESCE(wake_up_at, created_at)
        WHEN status = 'SLEEPING' THEN wake_up_at
        WHEN status = 'RUNNING' THEN locked_until
        ELSE NULL  -- terminal states don't need available_at
    END;

-- Make non-null for active workflows
ALTER TABLE kagzi.workflow_runs
    ADD CONSTRAINT chk_available_at
    CHECK (status IN ('COMPLETED', 'FAILED', 'CANCELLED') OR available_at IS NOT NULL);

-- Drop old columns
ALTER TABLE kagzi.workflow_runs DROP COLUMN wake_up_at;
ALTER TABLE kagzi.workflow_runs DROP COLUMN locked_until;
ALTER TABLE kagzi.workflow_runs DROP COLUMN deadline_at;

-- Update indexes
DROP INDEX IF EXISTS kagzi.idx_queue_poll;
DROP INDEX IF EXISTS kagzi.idx_queue_poll_hot;
DROP INDEX IF EXISTS kagzi.idx_workflow_poll;
DROP INDEX IF EXISTS kagzi.idx_workflow_runs_claimable;

CREATE INDEX idx_workflow_available ON kagzi.workflow_runs
    USING btree (namespace_id, task_queue, available_at)
    WHERE status IN ('PENDING', 'SLEEPING', 'RUNNING');
```

#### [MODIFY] [queue.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs)

Simplify [poll_workflow](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#9-65) query:

```rust
pub(super) async fn poll_workflow(
    repo: &PgWorkflowRepository,
    namespace_id: &str,
    task_queue: &str,
    worker_id: &str,
    types: &[String],
    visibility_timeout_secs: i64,
) -> Result<Option<ClaimedWorkflow>, StoreError> {
    let row = sqlx::query_as!(
        ClaimedRow,
        r#"
        WITH task AS (
            SELECT run_id
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1
              AND task_queue = $2
              AND status IN ('PENDING', 'SLEEPING', 'RUNNING')
              AND available_at <= NOW()
              AND (array_length($3::TEXT[], 1) IS NULL
                   OR array_length($3::TEXT[], 1) = 0
                   OR workflow_type = ANY($3))
            ORDER BY available_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE kagzi.workflow_runs w
        SET status = 'RUNNING',
            locked_by = $4,
            available_at = NOW() + ($5 * interval '1 second'),
            started_at = COALESCE(started_at, NOW()),
            attempts = attempts + 1
        FROM task
        WHERE w.run_id = task.run_id
        RETURNING w.run_id, w.workflow_type,
                  (SELECT input FROM kagzi.workflow_payloads p
                   WHERE p.run_id = w.run_id) as "input!",
                  w.locked_by
        "#,
        namespace_id,
        task_queue,
        types,
        worker_id,
        visibility_timeout_secs as f64
    )
    .fetch_optional(&repo.pool)
    .await?;

    Ok(row.map(|r| ClaimedWorkflow {
        run_id: r.run_id,
        workflow_type: r.workflow_type,
        input: r.input,
        locked_by: r.locked_by,
    }))
}
```

**Delete these functions from queue.rs:**

- [extend_worker_locks()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#66-87)
- [extend_locks_for_runs()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#88-112)
- [wake_sleeping()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#113-152) (no longer needed - poll query handles it)
- [find_orphaned()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#153-190) (no longer needed - poll query handles it)

#### [MODIFY] [state.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/state.rs)

Update sleep and retry functions:

```rust
pub async fn schedule_sleep(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    duration_secs: u64
) -> Result<(), StoreError> {
    sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'SLEEPING',
            available_at = NOW() + ($2 * INTERVAL '1 second'),
            locked_by = NULL
        WHERE run_id = $1 AND status = 'RUNNING'
        "#,
        run_id,
        duration_secs as f64
    )
    .execute(&repo.pool)
    .await?;
    Ok(())
}

pub async fn schedule_retry(
    repo: &PgWorkflowRepository,
    run_id: Uuid,
    delay_ms: u64
) -> Result<(), StoreError> {
    sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'PENDING',
            available_at = NOW() + ($2 * INTERVAL '1 millisecond'),
            locked_by = NULL
        WHERE run_id = $1 AND status = 'RUNNING'
        "#,
        run_id,
        delay_ms as f64
    )
    .execute(&repo.pool)
    .await?;
    Ok(())
}
```

#### [MODIFY] [mod.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/mod.rs)

Remove from `WorkflowRepository` trait:

- [extend_worker_locks()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#66-87)
- [wake_sleeping()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#113-152)
- [find_orphaned()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/workflow/queue.rs#153-190)
- `find_and_recover_offline_worker_workflows()`

#### [MODIFY] [repository/workflow.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/repository/workflow.rs)

Remove same methods from trait definition.

---

## Phase 3: Remove Heartbeat Lock Extension

### Current Flow

```
Worker heartbeat() → workers.last_heartbeat_at = NOW()
                   → workflow_runs.locked_until = NOW() + 30s (via extend_worker_locks)
```

### New Flow

```
Worker heartbeat() → workers.last_heartbeat_at = NOW()
                   → (no lock extension)
```

### Files to Change

#### [MODIFY] [worker_service.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/worker_service.rs#132-184)

Remove lock extension from heartbeat:

```rust
async fn heartbeat(
    &self,
    request: Request<HeartbeatRequest>,
) -> Result<Response<HeartbeatResponse>, Status> {
    let req = request.into_inner();
    let worker_id = Uuid::parse_str(&req.worker_id)
        .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

    let accepted = self
        .store
        .workers()
        .heartbeat(WorkerHeartbeatParams {
            worker_id,
            active_count: req.active_count,
            completed_delta: req.completed_delta,
            failed_delta: req.failed_delta,
        })
        .await
        .map_err(map_store_error)?;

    if !accepted {
        return Err(not_found_error(
            "Worker not found or offline",
            "worker",
            req.worker_id,
        ));
    }

    // REMOVED: extend_worker_locks() call

    let worker = self
        .store
        .workers()
        .find_by_id(worker_id)
        .await
        .map_err(map_store_error)?;

    let should_drain = worker
        .map(|w| w.status == StoreWorkerStatus::Draining)
        .unwrap_or(false);

    Ok(Response::new(HeartbeatResponse {
        accepted: true,
        should_drain,
    }))
}
```

#### [MODIFY] [config.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/config.rs)

Add visibility timeout config:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerSettings {
    // ... existing fields ...

    /// How long a worker has to complete a workflow before it becomes
    /// available to other workers. Default: 300 seconds (5 minutes).
    #[serde(default = "default_visibility_timeout_secs")]
    pub visibility_timeout_secs: u64,
}

fn default_visibility_timeout_secs() -> u64 { 300 }
```

#### [MODIFY] [worker_service.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/worker_service.rs#52)

Use configurable visibility timeout in poll:

```rust
impl<Q: QueueNotifier> WorkerServiceImpl<Q> {
    // Remove: const WORKFLOW_LOCK_DURATION_SECS: i64 = 30;

    // In poll_task(), use self.worker_settings.visibility_timeout_secs
}
```

---

## Phase 4: Remove Step-Level Retries

### Current Model

```
Step fails → step_runs.retry_at set → watchdog picks it up → re-executes step
```

### New Model (Workflow Replay)

```
Step fails → workflow_runs.available_at set → worker claims → replays workflow
           → cached steps return instantly → failed step re-executes
```

### Files to Change

#### [NEW] migrations/XXXXXX_remove_step_retry_at.sql

```sql
DROP INDEX IF EXISTS kagzi.idx_step_runs_pending_retry;
DROP INDEX IF EXISTS kagzi.idx_step_runs_retry;
ALTER TABLE kagzi.step_runs DROP COLUMN retry_at;
```

#### [MODIFY] [step.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/postgres/step.rs)

Remove `process_pending_retries()` function entirely.

Update [fail()](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/worker_service.rs#484-547) to schedule workflow retry instead of step retry:

```rust
pub async fn fail(&self, params: FailStepParams) -> Result<FailStepResult, StoreError> {
    // Record step as failed
    sqlx::query!(
        r#"
        UPDATE kagzi.step_runs
        SET status = 'FAILED', error = $3, finished_at = NOW()
        WHERE run_id = $1 AND step_id = $2 AND is_latest = true
        "#,
        params.run_id,
        params.step_id,
        params.error
    )
    .execute(&self.pool)
    .await?;

    // Check retry policy
    let retry_info = self.get_retry_info(params.run_id, &params.step_id).await?;

    if let Some(info) = retry_info {
        let policy = info.retry_policy.unwrap_or_default();

        if !params.non_retryable && policy.should_retry(info.attempt_number) {
            let delay_ms = params.retry_after_ms
                .unwrap_or_else(|| policy.calculate_delay_ms(info.attempt_number));

            // Return indication to schedule WORKFLOW retry
            return Ok(FailStepResult {
                scheduled_retry: true,
                schedule_workflow_retry_ms: Some(delay_ms as u64),
            });
        }
    }

    Ok(FailStepResult {
        scheduled_retry: false,
        schedule_workflow_retry_ms: None,
    })
}
```

#### [MODIFY] [models/step.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/models/step.rs)

Update `FailStepResult`:

```rust
pub struct FailStepResult {
    pub scheduled_retry: bool,
    pub schedule_workflow_retry_ms: Option<u64>,
    // Remove: pub retry_at: Option<DateTime<Utc>>,
}
```

#### [MODIFY] [worker_service.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/worker_service.rs#484-546)

Update [fail_step](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/worker_service.rs#484-547) to schedule workflow retry:

```rust
async fn fail_step(&self, request: Request<FailStepRequest>) -> Result<Response<FailStepResponse>, Status> {
    // ... existing validation ...

    let result = self.store.steps().fail(params).await.map_err(map_store_error)?;

    // If step failure schedules a workflow retry
    if let Some(delay_ms) = result.schedule_workflow_retry_ms {
        self.store
            .workflows()
            .schedule_retry(run_id, delay_ms)
            .await
            .map_err(map_store_error)?;
    }

    Ok(Response::new(FailStepResponse {
        scheduled_retry: result.scheduled_retry,
        retry_at: None, // No longer used
    }))
}
```

---

## Phase 5: Merge Scheduler + Watchdog → Coordinator

### Current

- [scheduler.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/scheduler.rs): wakes sleeps, fires cron (5s interval)
- [watchdog.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/watchdog.rs): step retries, orphan recovery, stale workers (1s interval)

### New

Single `coordinator.rs` that only handles:

1. Fire due cron schedules
2. Mark stale workers offline

**Note:** Orphan recovery and sleep waking are now handled by the poll query itself.

### Files to Change

#### [NEW] [coordinator.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/coordinator.rs)

```rust
use std::time::Duration;

use kagzi_queue::QueueNotifier;
use kagzi_store::PgStore;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::CoordinatorSettings;

pub async fn run<Q: QueueNotifier>(
    store: PgStore,
    queue: Q,
    settings: CoordinatorSettings,
    shutdown: CancellationToken,
) {
    let interval = Duration::from_secs(settings.interval_secs);
    let mut ticker = tokio::time::interval(interval);

    info!(interval_secs = settings.interval_secs, "Coordinator started");

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Coordinator shutting down");
                break;
            }
            _ = ticker.tick() => {
                // 1. Fire due cron schedules
                if let Err(e) = fire_due_schedules(&store, &queue, &settings).await {
                    error!("Failed to fire schedules: {:?}", e);
                }

                // 2. Mark stale workers offline
                if let Err(e) = mark_stale_workers(&store, settings.worker_stale_threshold_secs).await {
                    error!("Failed to mark stale workers: {:?}", e);
                }
            }
        }
    }
}

async fn fire_due_schedules<Q: QueueNotifier>(
    store: &PgStore,
    queue: &Q,
    settings: &CoordinatorSettings,
) -> Result<(), kagzi_store::StoreError> {
    // Move cron logic from scheduler.rs here
    // ...
}

async fn mark_stale_workers(
    store: &PgStore,
    threshold_secs: i64,
) -> Result<(), kagzi_store::StoreError> {
    let count = store.workers().mark_stale_offline(threshold_secs).await?;
    if count > 0 {
        warn!(count, "Marked stale workers offline");
    }
    Ok(())
}
```

#### [MODIFY] [config.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/config.rs)

Replace scheduler + watchdog settings:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct CoordinatorSettings {
    #[serde(default = "default_coordinator_interval_secs")]
    pub interval_secs: u64,

    #[serde(default = "default_coordinator_batch_size")]
    pub batch_size: i32,

    #[serde(default = "default_worker_stale_threshold_secs")]
    pub worker_stale_threshold_secs: i64,
}

fn default_coordinator_interval_secs() -> u64 { 5 }
fn default_coordinator_batch_size() -> i32 { 100 }
fn default_worker_stale_threshold_secs() -> i64 { 30 }
```

#### [MODIFY] [main.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/main.rs)

Replace scheduler + watchdog spawns:

```rust
// Remove:
// tokio::spawn(scheduler::run(...));
// watchdog::spawn(...);

// Add:
tokio::spawn(coordinator::run(
    store.clone(),
    queue.clone(),
    settings.coordinator,
    shutdown.child_token(),
));
```

#### [DELETE] [scheduler.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/scheduler.rs)

Move cron logic to coordinator, then delete.

#### [DELETE] [watchdog.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/watchdog.rs)

Delete entirely (all functionality absorbed by poll query + coordinator).

---

## Phase 6: Cleanup

### Files to Update

1. **[lib.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-server/src/lib.rs)** - Remove `scheduler`, `watchdog` exports, add `coordinator`

2. **[models/workflow.rs](file:///Users/atlantic/Developer/Kagzi/crates/kagzi-store/src/models/workflow.rs)** - Remove `WokenWorkflow`, `OrphanedWorkflow` if unused

3. **[tests/common.rs](file:///Users/atlantic/Developer/Kagzi/crates/tests/src/common.rs)** - Update test harness to use coordinator

4. **[proto/worker.proto](file:///Users/atlantic/Developer/Kagzi/proto/worker.proto)** - Remove `retry_at` from `FailStepResponse`

### Tests to Update/Remove

- `step_retry_at_honored_before_reexecution` - behavior changed, update or remove

### Config Changes

| Old                                          | New                                             |
| -------------------------------------------- | ----------------------------------------------- |
| `KAGZI_SCHEDULER_INTERVAL_SECS`              | `KAGZI_COORDINATOR_INTERVAL_SECS`               |
| `KAGZI_SCHEDULER_BATCH_SIZE`                 | `KAGZI_COORDINATOR_BATCH_SIZE`                  |
| `KAGZI_WATCHDOG_INTERVAL_SECS`               | (removed)                                       |
| `KAGZI_WATCHDOG_WORKER_STALE_THRESHOLD_SECS` | `KAGZI_COORDINATOR_WORKER_STALE_THRESHOLD_SECS` |
| (new)                                        | `KAGZI_WORKER_VISIBILITY_TIMEOUT_SECS`          |

---

## Summary

| Phase | Description                              | Breaking?       | Effort |
| ----- | ---------------------------------------- | --------------- | ------ |
| 1     | Fix LISTEN/NOTIFY                        | No              | Small  |
| 2     | Unify timestamps → `available_at`        | Yes (migration) | Medium |
| 3     | Remove heartbeat lock extension          | Yes (behavior)  | Small  |
| 4     | Remove step-level retries                | Yes (behavior)  | Medium |
| 5     | Merge scheduler + watchdog → coordinator | Yes (config)    | Medium |
| 6     | Cleanup                                  | No              | Small  |

**Total: ~3-4 days of focused work**

---

## Verification Plan

### Automated Tests

```bash
# Run existing test suite
cargo test --workspace

# Specific tests to verify
cargo test -p kagzi-tests workflow
cargo test -p kagzi-tests sleep
cargo test -p kagzi-tests retry
```

### Manual Verification

1. Create workflow → verify `available_at` set correctly
2. Sleep workflow → verify `available_at` extended
3. Let workflow timeout → verify another worker claims it
4. Cron schedule → verify coordinator fires it
5. Worker goes stale → verify marked offline
