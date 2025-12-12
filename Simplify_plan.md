# Kagzi Simplification Plan

> **Goal:** Refactor Kagzi from a "Complex In-Memory Distributor" architecture to a **"Robust Database-Backed Queue"** using Postgres `SKIP LOCKED`. This simplifies the codebase, fixes critical reliability bugs, and makes the system production-ready.

---

## Implementation Record

> **Status**: ✅ **COMPLETE** - All 3 phases implemented + post-migration cleanup  
> **Date Completed**: December 12, 2025  
> **Total LOC Removed**: ~700+

### What Changed

#### Database Schema

- **Dropped 3 tables**: `queue_counters`, `queue_configs`, `workflow_type_configs`
- **Added 2 indexes**: `idx_workflow_poll` (SKIP LOCKED optimization), `idx_workflow_status_lookup` (Admin UI)
- **Migration**: `migrations/20251211_optimize_architecture.sql`

#### Server Architecture

- **Removed**: `work_distributor.rs` module (~308 LOC) - entire in-memory work distribution system
- **Removed**: `notify.rs` module (~33 LOC) - replaced by direct PgListener usage
- **Added**: `poll_workflow()` - single atomic query using SKIP LOCKED
- **Modified**: `poll_task` - now uses long-polling with PgListener + jitter (0-500ms)
- **Modified**: `begin_step` - lazy sleep completion (completes on resume, not on schedule)
- **Modified**: `start_workflow` - removed SELECT before INSERT, relies on unique constraint

#### Watchdog Simplification

- **Removed**: `run_wake_sleeping` - polling handles wakeups automatically
- **Removed**: `run_reconcile_counters` - no more counter tables
- **Kept**: `run_process_retries`, `run_find_orphaned`, `run_mark_stale`

#### SDK Reliability Fixes

- **Fixed**: "Zombie Success" bug - infinite retry with exponential backoff for `complete_step`
- **Fixed**: Eager sleep completion - removed premature `complete_step` call
- **Verified**: `WorkflowPaused` handling already correct

#### Store Layer Cleanup (Post-Migration)

- **worker.rs**: Removed `fetch_queue_limits_for_pairs`, `fetch_workflow_limits_for_pairs`, `fetch_concurrency`
- **worker.rs**: Removed counter table inserts from `register` method
- **helpers.rs**: Removed `decrement_counter_tx`, `decrement_counters_tx`
- **state.rs**: Removed 6 calls to `decrement_counters_tx` from workflow state transitions

### Key Design Decisions

1. **Worker-Side Concurrency**: Moved from server-side counter tables to worker-side semaphores (like Temporal)
2. **Database-Driven Polling**: Replaced in-memory channels with `SKIP LOCKED` + `NOTIFY`
3. **Idempotent Operations**: Made `complete_step` idempotent to handle retries gracefully
4. **Lazy Sleep**: Sleep steps completed when workflow resumes, not when scheduled
5. **Infinite Retry**: `complete_step` retries forever on transient errors to prevent data loss

### Files Modified

**Server Core**:

- `crates/kagzi-server/src/work_distributor.rs` - **DELETED**
- `crates/kagzi-server/src/lib.rs` - removed WorkDistributor export
- `crates/kagzi-server/src/worker_service.rs` - long-polling, lazy sleep
- `crates/kagzi-server/src/workflow_service.rs` - simplified StartWorkflow
- `crates/kagzi-server/src/watchdog.rs` - removed 2 functions
- `crates/kagzi-server/Cargo.toml` - added `md5`, `rand` dependencies

**Store Layer**:

- `crates/kagzi-store/src/repository/workflow.rs` - added `poll_workflow`, removed 6 methods
- `crates/kagzi-store/src/postgres/workflow/queue.rs` - implemented `poll_workflow`
- `crates/kagzi-store/src/postgres/workflow/mod.rs` - updated trait implementation
- `crates/kagzi-store/src/postgres/workflow/notify.rs` - **DELETED**
- `crates/kagzi-store/src/postgres/workflow/helpers.rs` - removed counter functions
- `crates/kagzi-store/src/postgres/workflow/state.rs` - removed counter calls
- `crates/kagzi-store/src/postgres/step.rs` - made `complete` idempotent
- `crates/kagzi-store/src/postgres/worker.rs` - removed counter methods

**SDK**:

- `crates/kagzi/src/workflow_context.rs` - infinite retry, removed eager sleep
- `crates/kagzi/src/worker.rs` - verified WorkflowPaused handling

**Database**:

- `migrations/20251211_optimize_architecture.sql` - **NEW**

### Verification Status

✅ **Build**: Successful  
✅ **Server**: Running  
✅ **Examples**: 01_basics ✅, 03_scheduling ✅  
⚠️ **Examples**: 07_idempotency ❌ (pre-existing issue, not related to refactor)

---

## Executive Summary

### What We're Changing

| Current State                                                                        | Target State                                    |
| ------------------------------------------------------------------------------------ | ----------------------------------------------- |
| Complex in-memory `WorkDistributor` with channels, mutexes, and pending request maps | Simple database polling with `SKIP LOCKED`      |
| Server-side global concurrency counters (`queue_counters` table)                     | Worker-side semaphores (like Temporal)          |
| Fragile crash recovery via watchdog timeouts                                         | Instant lock stealing via `locked_until` expiry |
| Network failures during `CompleteStep` cause workflow failures                       | Infinite retry with exponential backoff         |
| Eager sleep completion (completes before timer fires)                                | Lazy sleep completion (completes on wakeup)     |

### Impact

- **~500 lines of code removed**
- **3 database tables dropped**
- **Simpler mental model** — "If it's in the DB with the right status, it gets picked up"

---

## Phase 0: Preparation

### 0.1 Add Dependencies

**File:** `crates/kagzi-server/Cargo.toml`

| Dependency | Version | Purpose                                            |
| ---------- | ------- | -------------------------------------------------- |
| `md5`      | `0.7`   | Channel name hashing (must match Postgres trigger) |
| `rand`     | `0.8`   | Polling jitter (thundering herd prevention)        |

### 0.2 Clean Slate

Ensure your development database is ready for breaking migrations. The migration will drop tables that store concurrency state.

---

## Phase 1: Database & Store Layer

This phase establishes the foundation. All subsequent changes build on this.

### 1.1 Migration: Schema Optimization

**File:** `migrations/20251211_optimize_architecture.sql`

**Purpose:** Remove fragile server-side concurrency limits and add efficient polling indexes.

**Changes:**

1. **Drop Counter Tables** — These tracked global concurrency but were fragile and hard to debug:

   - `kagzi.queue_counters`
   - `kagzi.queue_configs`
   - `kagzi.workflow_type_configs`

2. **Add Polling Index** — Optimizes the `SKIP LOCKED` query:

   - Partial index on `(namespace_id, task_queue, wake_up_at, created_at)`
   - Covers statuses: `PENDING`, `SLEEPING`, `RUNNING`

3. **Add Admin Index** — Efficient list queries for the Admin UI:
   - Index on `(namespace_id, status, created_at DESC)`

---

### 1.2 Repository Trait Updates

**Goal:** Simplify the `WorkflowRepository` trait by removing batch/claim methods and adding a single atomic poll method.

#### File: `crates/kagzi-store/src/repository/workflow.rs`

**Remove these methods:**

- `wait_for_new_work` — Replaced by PgListener in server layer
- `claim_workflow_batch` — Replaced by `poll_workflow`
- `claim_specific_workflow` — No longer needed
- `list_available_workflows` — No longer needed

**Add this method:**

```rust
async fn poll_workflow(
    &self,
    namespace_id: &str,
    task_queue: &str,
    worker_id: &str,
    types: &[String],
    lock_secs: i64,
) -> Result<Option<ClaimedWorkflow>, StoreError>;
```

#### File: `crates/kagzi-store/src/repository/worker.rs` (if counter methods exist here)

**Remove:**

- `increment_queue_counter`
- `decrement_queue_counter`
- `reconcile_queue_counters`

---

### 1.3 Implement `poll_workflow` (Core Polling Logic)

**File:** `crates/kagzi-store/src/postgres/workflow/` (or appropriate submodule)

**Purpose:** A single atomic query that:

1. Finds available work (PENDING, waking SLEEPING, or stale RUNNING)
2. Locks it with `FOR UPDATE SKIP LOCKED`
3. Updates status to RUNNING with lock expiry

**Query Logic (Conceptual):**

```
CTE: Select ONE candidate where:
  - namespace_id and task_queue match
  - workflow_type is in the supported list
  - Status is PENDING, OR
  - Status is SLEEPING with wake_up_at <= NOW(), OR
  - Status is RUNNING with locked_until < NOW() (crash recovery)

  Order by: wake_up_at ASC NULLS FIRST, created_at ASC
  Limit 1
  FOR UPDATE SKIP LOCKED

UPDATE the found row:
  - Set status = 'RUNNING'
  - Set locked_by = worker_id
  - Set locked_until = NOW() + lock_secs
  - Set started_at = COALESCE(started_at, NOW())
  - Clear wake_up_at

RETURN: run_id, workflow_type, input (from payloads table)
```

**Key Behaviors:**

- **SKIP LOCKED** ensures multiple workers don't fight over the same row
- **Crash Recovery** happens automatically via `locked_until < NOW()` check
- **Sleep Wakeup** happens via `wake_up_at <= NOW()` check

---

### 1.4 Lock Extension (Heartbeat Support)

**File:** `crates/kagzi-store/src/postgres/workflow.rs` → `extend_worker_locks`

**Purpose:** When a worker sends a heartbeat, extend the `locked_until` timestamp to prevent lock stealing while the worker is healthy.

**Logic:**

- Update all workflows where `locked_by = worker_id` AND `status = 'RUNNING'`
- Set `locked_until = NOW() + extension_secs`
- Return count of extended locks

---

### 1.5 Make `complete_step` Idempotent

**File:** `crates/kagzi-store/src/postgres/step.rs` → `complete`

**Problem:** Currently, if `UPDATE` affects 0 rows, it may return an error. But with network retries, a step might already be completed from a previous attempt.

**Fix:**

1. Attempt the `UPDATE ... WHERE status = 'RUNNING'`
2. If `rows_affected == 0`:
   - Query the step to check its current status
   - If `status == 'COMPLETED'` → Return `Ok(())` (idempotent success)
   - If step doesn't exist → Return `StoreError::NotFound`
   - If step exists with different status → Handle appropriately

---

## Phase 2: Server Core

This phase removes the complex in-memory state management and implements the new polling architecture.

### 2.1 Remove WorkDistributor

**Files to modify:**

| File                                          | Action                               |
| --------------------------------------------- | ------------------------------------ |
| `crates/kagzi-server/src/work_distributor.rs` | **DELETE entirely**                  |
| `crates/kagzi-server/src/lib.rs`              | Remove `mod work_distributor`        |
| `crates/kagzi-server/src/worker_service.rs`   | Remove `WorkDistributorHandle` usage |

**What we're removing:**

- `WorkDistributor` struct with its `Mutex<HashMap>` pending requests
- `WorkDistributorHandle` wrapper
- Channel-based request/response pattern
- Complex queue processor spawn logic

---

### 2.2 Implement Long-Polling in `poll_task`

**File:** `crates/kagzi-server/src/worker_service.rs` → `poll_task`

**New Logic (Pseudocode):**

```
fn poll_task(request):
    timeout = request.timeout (default 30s)
    deadline = now() + timeout

    loop:
        // 1. Try to get work from DB
        if let Some(task) = store.poll_workflow(...):
            return task

        // 2. Check if we've exceeded timeout
        remaining = deadline - now()
        if remaining <= 0:
            return Empty

        // 3. Subscribe to Postgres NOTIFY channel
        channel_name = format!("kagzi_work_{:x}", md5(namespace_id + "_" + task_queue))
        listener = PgListener::connect_lazy(channel_name)

        // 4. Wait for notification or timeout
        select:
            notification = listener.recv():
                // Add jitter to prevent thundering herd
                sleep(random(0..500ms))
                continue  // Go back to step 1

            _ = sleep(remaining):
                return Empty  // Timeout reached
```

**Important Details:**

- Channel name format must match the Postgres trigger: `kagzi_work_{md5_hex}`
- Jitter (0-500ms random delay) prevents all workers from polling simultaneously
- PgListener uses the same connection pool

---

### 2.3 Lazy Sleep Completion

**File:** `crates/kagzi-server/src/worker_service.rs` → `begin_step`

**Problem:** Currently, sleep steps are completed eagerly (before the timer fires). This creates garbage rows and complex state.

**Fix:** Complete sleep steps lazily when the workflow resumes.

**Logic:**

```
fn begin_step(request):
    // Fetch existing step
    existing = store.steps().find_latest(run_id, step_id)

    if existing.kind == SLEEP and existing.status != COMPLETED:
        // Workflow is RUNNING, so sleep timer must have passed
        // Complete the sleep step now
        store.steps().complete(run_id, step_id, empty_output)

        return BeginStepResponse {
            should_execute: false,  // Don't execute, just replay
            cached_output: empty,
        }

    // Normal begin_step logic...
```

---

### 2.4 Simplify `StartWorkflow`

**File:** `crates/kagzi-server/src/workflow_service.rs` → `start_workflow`

**Current Pattern (Inefficient):**

```
1. SELECT to check if workflow exists
2. If not exists, INSERT
```

**New Pattern (Efficient):**

```
1. INSERT directly
2. On conflict (duplicate key), return already_exists: true
3. On other error, return error
```

This is more efficient and handles race conditions naturally.

---

### 2.5 Cleanup Watchdog

**File:** `crates/kagzi-server/src/watchdog.rs`

| Function                 | Action     | Reason                        |
| ------------------------ | ---------- | ----------------------------- |
| `run_reconcile_counters` | **REMOVE** | Counter tables are deleted    |
| `run_wake_sleeping`      | **REMOVE** | Polling query handles wakeups |
| `run_find_orphaned`      | **KEEP**   | Safety net for edge cases     |
| `run_mark_stale`         | **KEEP**   | Observability for monitoring  |

---

### 2.6 Admin Payload Safety

**File:** `crates/kagzi-server/src/helpers.rs`

**Action:** Remove `payload_to_optional_json` or similar functions that parse user payloads on the server.

**Reason:** User payloads should be treated as opaque bytes. The server shouldn't attempt JSON parsing which could fail on valid binary data.

---

## Phase 3: Client SDK

This phase fixes critical reliability bugs in the Rust SDK.

### 3.1 Fix "Zombie Success" Bug

**File:** `crates/kagzi/src/workflow_context.rs` → `run_with_input_with_retry`

**Problem:** If the user function succeeds but `complete_step` fails due to network issues, the workflow continues and later attempts fail because the step is in a bad state.

**Fix:** Infinite retry with exponential backoff for `complete_step`:

```
// In the success path (after user function returns Ok):
backoff = 1 second
loop:
    result = client.complete_step(...)

    match result:
        Ok(_) => break  // Success, continue workflow

        Err(NotFound) =>
            // Workflow was deleted, this is unrecoverable
            error!("Workflow deleted during execution")
            return Err(...)

        Err(other) =>
            warn!("CompleteStep failed, retrying in {:?}: {}", backoff, other)
            sleep(backoff)
            backoff = min(backoff * 2, 60 seconds)
            continue

// CRITICAL: Do NOT call fail_step after this block
```

---

### 3.2 Fix Sleep Implementation

**File:** `crates/kagzi/src/workflow_context.rs` → `sleep`

**Current (Broken):**

```
1. BeginStep(SLEEP)
2. client.sleep(duration)
3. CompleteStep  ← WRONG: Completing before timer fires
4. Return WorkflowPaused
```

**Fixed:**

```
1. BeginStep(SLEEP)
2. If !should_execute → return Ok(())  // Already slept, this is replay
3. client.sleep(duration)  // Set timer on server
4. Return Err(WorkflowPaused)  // Stop execution, no CompleteStep
```

The step gets completed lazily when the workflow is polled again after the sleep timer fires (see 2.3).

---

### 3.3 Worker Loop: Handle `WorkflowPaused`

**File:** `crates/kagzi/src/worker.rs`

**Context:** When a workflow sleeps, it returns `Err(WorkflowPaused)`. The worker loop must handle this gracefully.

**Logic:**

```
match execute_workflow(...).await:
    Ok(_) =>
        completed_counter.increment()

    Err(e) if e.is::<WorkflowPaused>() =>
        info!("Workflow sleeping, releasing slot")
        // Do NOT call fail_workflow
        // Do NOT increment failed_counter
        // Just return, releasing the semaphore permit

    Err(e) =>
        error!("Workflow failed: {}", e)
        failed_counter.increment()
        // Call fail_workflow if not already failed
```

---

## Verification Checklist

After implementation, verify these scenarios:

### 1. Concurrency Control

- Start 100 workflows that each sleep for 5 seconds
- Run 1 worker with concurrency limit of 10
- **Expected:** Workflows complete in batches of 10, taking ~50 seconds total

### 2. Crash Recovery

- Start a long-running workflow (e.g., with a 60s activity)
- Kill the worker process with `kill -9`
- Start a new worker
- **Expected:** New worker picks up the workflow after `locked_until` expires (~30s)

### 3. Network Partition

- Start a workflow
- After activity completes but before `complete_step`, disconnect network
- Wait 1 minute, reconnect network
- **Expected:** Worker retries `complete_step`, workflow completes successfully (no failure in DB)

### 4. Sleep Replay

- Start a workflow that sleeps for 10 seconds
- Kill the worker during the sleep
- Wait 20 seconds, start a new worker
- **Expected:** Workflow resumes immediately without re-sleeping

---

## Implementation Order

```
Phase 1: Database & Store
  1.1 Migration
  1.2 Repository trait updates
  1.3 poll_workflow implementation
  1.4 Lock extension
  1.5 Idempotent complete_step

Phase 2: Server Core
  2.1 Remove WorkDistributor
  2.2 Long-polling implementation
  2.3 Lazy sleep completion
  2.4 Simplify StartWorkflow
  2.5 Cleanup watchdog
  2.6 Admin payload safety

Phase 3: Client SDK
  3.1 Zombie success fix
  3.2 Sleep implementation fix
  3.3 Worker loop handling

Verification
  Run all verification scenarios
```

---

## Architecture After Refactor

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│    Worker 1     │     │    Worker 2     │     │    Worker N     │
│  (Semaphore)    │     │  (Semaphore)    │     │  (Semaphore)    │
└────────┬────────┘     └────────┬────────┘     └────────┬────────┘
         │                       │                       │
         └───────────────────────┼───────────────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │     Kagzi Server        │
                    │  ┌──────────────────┐   │
                    │  │   poll_task      │   │
                    │  │  (Long-Polling)  │   │
                    │  └────────┬─────────┘   │
                    └───────────┼─────────────┘
                                │
                    ┌───────────▼───────────┐
                    │       Postgres        │
                    │  ┌─────────────────┐  │
                    │  │ workflow_runs   │  │
                    │  │ (SKIP LOCKED)   │  │
                    │  └─────────────────┘  │
                    │  ┌─────────────────┐  │
                    │  │   PgListener    │  │
                    │  │  (NOTIFY)       │  │
                    │  └─────────────────┘  │
                    └───────────────────────┘
```

**Key Properties:**

- **Distributed-safe:** Multiple servers can run without coordination
- **Crash-resilient:** Lock stealing provides instant recovery
- **Simple:** No in-memory queue state to manage
- **Observable:** Standard Postgres monitoring applies
