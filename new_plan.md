# Kagzi Feature Implementation Plan

This document outlines the implementation plan for production-grade features in Kagzi, based on research from Temporal and Inngest workflow engines.

---

## Table of Contents

1. [Automatic Retries with RetryPolicy](#1-automatic-retries-with-retrypolicy)
2. [Configurable Timeouts](#2-configurable-timeouts)
3. [Failure Handlers (Inngest-style)](#3-failure-handlers-inngest-style)
4. [Workflow Timeouts (Deadline Enforcement)](#4-workflow-timeouts-deadline-enforcement)
5. [Backward Pagination](#5-backward-pagination)
6. [Implementation Priority](#6-implementation-priority)

---

## 1. Automatic Retries with RetryPolicy

> **Priority: P0** - High value, utilizes existing Reaper patterns with `wake_up_at`.

### Current State

Attempt tracking exists (`attempt_number` column) but no automatic retry logic.

### How Temporal Does It

```go
RetryPolicy{
    InitialInterval:    time.Second,
    BackoffCoefficient: 2.0,
    MaximumInterval:    time.Second * 100,
    MaximumAttempts:    5,
    NonRetryableErrorTypes: []string{"InvalidInputError"},
}
```

### How Inngest Does It

```typescript
inngest.createFunction({
  id: "my-function",
  retries: 10,
}, ...);

throw new NonRetriableError("Invalid input");     // Stop immediately
throw new RetryAfterError("Rate limited", "30s"); // Custom delay
```

### Kagzi Implementation

#### 1.1 Protocol Buffer Changes

```protobuf
message RetryPolicy {
  int32 maximum_attempts = 1;           // default: 5, -1 = unlimited
  int64 initial_interval_ms = 2;        // default: 1000
  double backoff_coefficient = 3;       // default: 2.0
  int64 maximum_interval_ms = 4;        // default: 60000
  repeated string non_retryable_errors = 5;
}

message StartWorkflowRequest {
  // ... existing fields ...
  RetryPolicy retry_policy = 21;
}

message BeginStepRequest {
  // ... existing fields ...
  RetryPolicy retry_policy = 6;
}

message FailStepRequest {
  // ... existing fields ...
  bool non_retryable = 4;
  int64 retry_after_ms = 5;
}
```

#### 1.2 Database Schema

```sql
-- Migration: add_retry_support.sql

-- Add retry policy to workflow_runs
ALTER TABLE kagzi.workflow_runs
ADD COLUMN retry_policy JSONB;

-- Backfill existing rows to prevent NULL in application logic
UPDATE kagzi.workflow_runs
SET retry_policy = '{"maximum_attempts": 5, "initial_interval_ms": 1000, "backoff_coefficient": 2.0, "maximum_interval_ms": 60000}'::jsonb
WHERE retry_policy IS NULL;

-- Add retry scheduling to step_runs
ALTER TABLE kagzi.step_runs
ADD COLUMN retry_at TIMESTAMPTZ,
ADD COLUMN retry_policy JSONB;

-- Index for retry scheduling
CREATE INDEX idx_step_runs_retry
ON kagzi.step_runs (run_id, retry_at)
WHERE retry_at IS NOT NULL;
```

#### 1.3 Server Implementation

```rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RetryPolicyConfig {
    pub maximum_attempts: i32,
    pub initial_interval_ms: i64,
    pub backoff_coefficient: f64,
    pub maximum_interval_ms: i64,
    #[serde(default)]
    pub non_retryable_errors: Vec<String>,
}

impl Default for RetryPolicyConfig {
    fn default() -> Self {
        Self {
            maximum_attempts: 5,
            initial_interval_ms: 1000,
            backoff_coefficient: 2.0,
            maximum_interval_ms: 60000,
            non_retryable_errors: vec![],
        }
    }
}

impl RetryPolicyConfig {
    pub fn calculate_delay(&self, attempt: i32) -> Duration {
        let delay_ms = (self.initial_interval_ms as f64
            * self.backoff_coefficient.powi(attempt)) as i64;
        Duration::from_millis(delay_ms.min(self.maximum_interval_ms) as u64)
    }

    pub fn is_non_retryable(&self, error: &str) -> bool {
        self.non_retryable_errors.iter().any(|e| error.starts_with(e))
    }

    pub fn should_retry(&self, current_attempt: i32) -> bool {
        self.maximum_attempts < 0 || current_attempt < self.maximum_attempts
    }
}

// Updated FailStep handler
async fn fail_step(&self, request: Request<FailStepRequest>) -> Result<Response<Empty>, Status> {
    let req = request.into_inner();
    let run_id = uuid::Uuid::parse_str(&req.run_id)?;

    let step_info = sqlx::query!(
        "SELECT attempt_number, COALESCE(sr.retry_policy, wr.retry_policy) as policy
         FROM kagzi.step_runs sr
         JOIN kagzi.workflow_runs wr ON sr.run_id = wr.run_id
         WHERE sr.run_id = $1 AND sr.step_id = $2 AND sr.is_latest = true",
        run_id, req.step_id
    ).fetch_one(&self.pool).await?;

    let policy: RetryPolicyConfig = step_info.policy
        .map(|v| serde_json::from_value(v).unwrap_or_default())
        .unwrap_or_default();

    let should_retry = !req.non_retryable
        && !policy.is_non_retryable(&req.error)
        && policy.should_retry(step_info.attempt_number);

    if should_retry {
        let delay = if req.retry_after_ms > 0 {
            Duration::from_millis(req.retry_after_ms as u64)
        } else {
            policy.calculate_delay(step_info.attempt_number)
        };

        // Schedule retry with backoff delay
        sqlx::query!(
            "UPDATE kagzi.step_runs
             SET status = 'PENDING',
                 retry_at = NOW() + ($3 * INTERVAL '1 millisecond'),
                 error = $4
             WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
            run_id, req.step_id, delay.as_millis() as f64, req.error
        ).execute(&self.pool).await?;

        info!(run_id = %run_id, step_id = %req.step_id,
              attempt = step_info.attempt_number, delay_ms = delay.as_millis(),
              "Step failed, scheduling retry with backoff");
    } else {
        // Mark permanently failed
        sqlx::query!(
            "UPDATE kagzi.step_runs
             SET status = 'FAILED', error = $3, finished_at = NOW()
             WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
            run_id, req.step_id, req.error
        ).execute(&self.pool).await?;

        info!(run_id = %run_id, step_id = %req.step_id,
              "Step permanently failed (retries exhausted or non-retryable)");
    }

    Ok(Response::new(Empty {}))
}
```

#### 1.4 Reaper: Process Step Retries

```rust
// In reaper.rs

// Process step retries (steps waiting for retry_at)
let retry_result = sqlx::query!(
    r#"
    UPDATE kagzi.step_runs
    SET status = 'PENDING', retry_at = NULL
    WHERE status = 'PENDING'
      AND retry_at IS NOT NULL
      AND retry_at <= NOW()
    RETURNING run_id, step_id, attempt_number
    "#
).fetch_all(&pool).await;

match retry_result {
    Ok(rows) => {
        for row in &rows {
            info!(run_id = %row.run_id, step_id = %row.step_id,
                  attempt = row.attempt_number, "Reaper triggered step retry");
        }
    }
    Err(e) => error!("Reaper failed to process step retries: {:?}", e),
}
```

#### 1.5 Reaper: Orphan Recovery with Backoff

> **Important:** When a worker crashes, the reaper must respect retry policy instead of instantly restarting.

```rust
// In reaper.rs - UPDATED orphan recovery logic

// Recover orphaned workflows (crashed workers) - WITH BACKOFF
let orphan_result = sqlx::query!(
    r#"
    SELECT run_id, locked_by, attempts, retry_policy
    FROM kagzi.workflow_runs
    WHERE status = 'RUNNING'
      AND locked_until IS NOT NULL
      AND locked_until < NOW()
    FOR UPDATE SKIP LOCKED
    "#
).fetch_all(&pool).await;

match orphan_result {
    Ok(rows) => {
        for row in rows {
            let policy: RetryPolicyConfig = row.retry_policy
                .map(|v| serde_json::from_value(v).unwrap_or_default())
                .unwrap_or_default();

            if policy.should_retry(row.attempts) {
                // Calculate backoff delay
                let delay = policy.calculate_delay(row.attempts);

                // Schedule retry with backoff (not immediate restart!)
                sqlx::query!(
                    r#"
                    UPDATE kagzi.workflow_runs
                    SET status = 'PENDING',
                        locked_by = NULL,
                        locked_until = NULL,
                        wake_up_at = NOW() + ($2 * INTERVAL '1 millisecond'),
                        attempts = attempts + 1
                    WHERE run_id = $1
                    "#,
                    row.run_id,
                    delay.as_millis() as f64
                ).execute(&pool).await.ok();

                warn!(run_id = %row.run_id, previous_worker = ?row.locked_by,
                      delay_ms = delay.as_millis(),
                      "Recovered orphaned workflow - scheduling retry with backoff");
            } else {
                // Max retries exhausted - mark as failed
                sqlx::query!(
                    r#"
                    UPDATE kagzi.workflow_runs
                    SET status = 'FAILED',
                        error = 'Workflow crashed and exhausted all retry attempts',
                        finished_at = NOW(),
                        locked_by = NULL,
                        locked_until = NULL
                    WHERE run_id = $1
                    "#,
                    row.run_id
                ).execute(&pool).await.ok();

                error!(run_id = %row.run_id, attempts = row.attempts,
                       "Orphaned workflow exhausted retries - marked as failed");
            }
        }
    }
    Err(e) => error!("Reaper failed to recover orphaned workflows: {:?}", e),
}
```

#### 1.6 SDK Implementation

```rust
#[derive(Clone, Default)]
pub struct RetryPolicy {
    pub maximum_attempts: Option<i32>,
    pub initial_interval: Option<Duration>,
    pub backoff_coefficient: Option<f64>,
    pub maximum_interval: Option<Duration>,
    pub non_retryable_errors: Vec<String>,
}

/// Error that skips all retries
#[derive(Debug)]
pub struct NonRetryableError(pub String);

impl std::fmt::Display for NonRetryableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for NonRetryableError {}

/// Error with custom retry delay
#[derive(Debug)]
pub struct RetryAfterError {
    pub message: String,
    pub retry_after: Duration,
}

impl RetryAfterError {
    pub fn new(message: impl Into<String>, retry_after: Duration) -> Self {
        Self { message: message.into(), retry_after }
    }
}

impl std::fmt::Display for RetryAfterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for RetryAfterError {}

// Builder pattern
impl<'a, I: Serialize> WorkflowBuilder<'a, I> {
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    pub fn retries(mut self, max_attempts: i32) -> Self {
        self.retry_policy.get_or_insert_with(Default::default)
            .maximum_attempts = Some(max_attempts);
        self
    }
}

// Usage:
let run_id = client
    .workflow("ProcessPayment", "default", input)
    .retries(10)
    .retry_policy(RetryPolicy {
        initial_interval: Some(Duration::from_secs(5)),
        backoff_coefficient: Some(1.5),
        ..Default::default()
    })
    .await?;

// In step code:
async fn charge_card(input: &PaymentInput) -> anyhow::Result<PaymentResult> {
    if input.amount <= 0.0 {
        return Err(NonRetryableError("Invalid amount".into()).into());
    }

    match payment_gateway.charge(input).await {
        Ok(result) => Ok(result),
        Err(e) if e.is_rate_limited() => {
            Err(RetryAfterError::new("Rate limited", Duration::from_secs(30)).into())
        }
        Err(e) => Err(e.into()),
    }
}
```

---

## 2. Configurable Timeouts

> **Priority: P1** - Requires SDK <-> Server contract changes.

### Current State

Hardcoded timeouts:

- Worker lock: 30 seconds
- Poll timeout: 60 seconds
- Heartbeat interval: 10 seconds

### How Temporal Does It

```go
ActivityOptions{
    StartToCloseTimeout: time.Minute * 5,
    HeartbeatTimeout:    time.Minute * 1,
}
```

### Kagzi Implementation

All timeouts configured via the SDK builder pattern at workflow/step level.

#### 2.1 Protocol Buffer Changes

```protobuf
// Add to kagzi.proto

message TimeoutConfig {
  uint64 start_to_close_timeout_seconds = 1;  // Max step execution (default: 600s)
  uint64 heartbeat_timeout_seconds = 2;        // Lock refresh interval (default: 30s)
  uint64 execution_timeout_seconds = 3;        // Overall workflow deadline (default: 0 = none)
}

message StartWorkflowRequest {
  // ... existing fields ...
  TimeoutConfig timeouts = 20;
}

message BeginStepRequest {
  // ... existing fields ...
  uint64 start_to_close_timeout_seconds = 5;  // Step-specific override
}
```

#### 2.2 Database Schema

```sql
-- Migration: add_timeout_support.sql

-- Add step-level timeout tracking
ALTER TABLE kagzi.step_runs
ADD COLUMN timeout_at TIMESTAMPTZ;  -- Calculated as started_at + start_to_close_timeout

-- Add workflow-level timeout config
ALTER TABLE kagzi.workflow_runs
ADD COLUMN timeout_config JSONB;
```

#### 2.3 Server Implementation

**BeginStep - Set timeout_at:**

```rust
async fn begin_step(&self, request: Request<BeginStepRequest>) -> Result<Response<BeginStepResponse>, Status> {
    let req = request.into_inner();
    // ... existing memoization check ...

    // Get timeout from request or workflow default
    let timeout_seconds = if req.start_to_close_timeout_seconds > 0 {
        req.start_to_close_timeout_seconds
    } else {
        // Get from workflow's timeout_config or use default (600s)
        600
    };

    // Create step with timeout_at
    sqlx::query!(
        r#"
        INSERT INTO kagzi.step_runs
        (run_id, step_id, status, input, started_at, is_latest, attempt_number, namespace_id, timeout_at)
        VALUES ($1, $2, 'RUNNING', $3, NOW(), true,
                COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2), 0) + 1,
                COALESCE((SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $1), 'default'),
                NOW() + ($4 * INTERVAL '1 second'))
        "#,
        run_id, req.step_id, input_json, timeout_seconds as f64
    ).execute(&self.pool).await?;

    Ok(Response::new(BeginStepResponse { should_execute: true, cached_result: vec![] }))
}
```

**PollActivity - Use workflow-specific lock timeout:**

```rust
async fn poll_activity(&self, request: Request<PollActivityRequest>) -> ... {
    // ... existing code ...

    // Use workflow's heartbeat_timeout or default to 30s
    let lock_timeout = 30; // TODO: read from workflow's timeout_config

    let work = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'RUNNING',
            locked_by = $1,
            locked_until = NOW() + ($4 * INTERVAL '1 second'),
            started_at = COALESCE(started_at, NOW()),
            attempts = attempts + 1
        WHERE run_id = (...)
        RETURNING ...
        "#,
        req.worker_id, req.task_queue, namespace_id, lock_timeout as f64
    )
    // ...
}
```

#### 2.4 Reaper: Enforce Step Timeouts

> **Critical:** Server-side enforcement for hung steps (independent of worker lock).

```rust
// In reaper.rs - NEW: Step timeout enforcement

let step_timeout_result = sqlx::query!(
    r#"
    UPDATE kagzi.step_runs
    SET status = 'FAILED',
        error = 'Step execution timed out (start_to_close_timeout exceeded)',
        finished_at = NOW()
    WHERE status = 'RUNNING'
      AND timeout_at IS NOT NULL
      AND timeout_at < NOW()
    RETURNING run_id, step_id
    "#
).fetch_all(&pool).await;

match step_timeout_result {
    Ok(rows) => {
        for row in &rows {
            warn!(run_id = %row.run_id, step_id = %row.step_id,
                  "Reaper failed step due to timeout");
        }
    }
    Err(e) => error!("Reaper failed to enforce step timeouts: {:?}", e),
}
```

#### 2.5 SDK Implementation

**Worker: Wrap execution with tokio::time::timeout**

```rust
// In crates/kagzi/src/lib.rs - execute_workflow function

async fn execute_workflow(
    mut client: WorkflowServiceClient<Channel>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    worker_id: String,
    input: serde_json::Value,
    timeout_config: TimeoutConfig,  // NEW: pass timeout config
) {
    // ... heartbeat setup ...

    let ctx = WorkflowContext {
        client: client.clone(),
        run_id: run_id.clone(),
        sleep_counter: 0,
        step_timeout: timeout_config.start_to_close_timeout
            .unwrap_or(Duration::from_secs(600)),  // Default 10 min
    };

    let result = handler(ctx, input).await;
    // ... rest of handling ...
}

// In WorkflowContext::run()
impl WorkflowContext {
    pub async fn run<R, Fut>(&mut self, step_id: &str, fut: Fut) -> anyhow::Result<R>
    where
        R: Serialize + DeserializeOwned + Send + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send,
    {
        // ... begin_step call ...

        if !begin_resp.should_execute {
            return Ok(serde_json::from_slice(&begin_resp.cached_result)?);
        }

        // Wrap user's future with timeout
        let result = match tokio::time::timeout(self.step_timeout, fut).await {
            Ok(Ok(val)) => {
                // Success - complete step
                let output_bytes = serde_json::to_vec(&val)?;
                self.client.complete_step(CompleteStepRequest { ... }).await?;
                Ok(val)
            }
            Ok(Err(e)) => {
                // User logic error - fail step
                self.client.fail_step(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    error: e.to_string(),
                    non_retryable: e.downcast_ref::<NonRetryableError>().is_some(),
                    retry_after_ms: e.downcast_ref::<RetryAfterError>()
                        .map(|e| e.retry_after.as_millis() as i64)
                        .unwrap_or(0),
                }).await?;
                Err(e)
            }
            Err(_elapsed) => {
                // Timeout - fail step explicitly
                self.client.fail_step(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    error: format!("Step '{}' timed out after {:?}", step_id, self.step_timeout),
                    non_retryable: false,  // Timeouts can be retried
                    retry_after_ms: 0,
                }).await?;
                Err(anyhow::anyhow!("Step timed out"))
            }
        };

        result
    }
}
```

**Builder pattern:**

```rust
#[derive(Clone, Default)]
pub struct TimeoutConfig {
    pub start_to_close_timeout: Option<Duration>,
    pub heartbeat_timeout: Option<Duration>,
    pub execution_timeout: Option<Duration>,
}

impl<'a, I: Serialize> WorkflowBuilder<'a, I> {
    pub fn timeouts(mut self, config: TimeoutConfig) -> Self {
        self.timeouts = Some(config);
        self
    }

    pub fn execution_timeout(mut self, timeout: Duration) -> Self {
        self.timeouts.get_or_insert_with(Default::default)
            .execution_timeout = Some(timeout);
        self
    }

    pub fn step_timeout(mut self, timeout: Duration) -> Self {
        self.timeouts.get_or_insert_with(Default::default)
            .start_to_close_timeout = Some(timeout);
        self
    }

    pub fn heartbeat_timeout(mut self, timeout: Duration) -> Self {
        self.timeouts.get_or_insert_with(Default::default)
            .heartbeat_timeout = Some(timeout);
        self
    }
}

// Usage:
let run_id = client
    .workflow("ProcessOrder", "default", input)
    .execution_timeout(Duration::from_secs(3600))  // 1 hour max
    .step_timeout(Duration::from_secs(300))         // 5 min per step
    .heartbeat_timeout(Duration::from_secs(60))     // 1 min heartbeat
    .await?;
```

---

## 3. Failure Handlers (Inngest-style)

> **Priority: P2** - Purely additive logic.

### Current State

Failed workflows are marked `FAILED` with no automated handling.

### How Inngest Does It

Two patterns for handling failures:

```typescript
// Pattern 1: Per-function handler
inngest.createFunction({
  id: "process-payment",
  onFailure: async ({ error, event }) => {
    await sendAlert(`Payment failed: ${error}`);
  },
}, ...);

// Pattern 2: Global handler - catches ALL failures
inngest.createFunction(
  { id: "handle-all-failures" },
  { event: "inngest/function.failed" },  // System event
  async ({ event }) => {
    await datadog.sendMetric("function.failed", 1);
  }
);
```

Key Inngest concepts:

- **No DLQ** - failed functions stay queryable in history
- **System events** - `inngest/function.failed` emitted for every failure
- **Bulk replay** - re-run failed functions from dashboard after bug fix

### Kagzi Implementation

We'll implement both patterns:

1. **Per-workflow `onFailure`** - specify a handler workflow when starting
2. **Global `kagzi/workflow.failed`** - system event for all failures

#### 3.1 Protocol Buffer Changes

```protobuf
// System event payload for failures
message WorkflowFailedEvent {
  string failed_run_id = 1;
  string namespace_id = 2;
  string workflow_type = 3;
  string task_queue = 4;
  string error = 5;
  bytes original_input = 6;
  int32 attempts = 7;
  google.protobuf.Timestamp started_at = 8;
  google.protobuf.Timestamp failed_at = 9;
}

message StartWorkflowRequest {
  // ... existing fields ...
  string on_failure_workflow = 22;       // Per-workflow handler
  string on_failure_task_queue = 23;     // Queue for handler (optional)
}
```

#### 3.2 Database Schema

```sql
ALTER TABLE kagzi.workflow_runs
ADD COLUMN on_failure_workflow TEXT,
ADD COLUMN on_failure_task_queue TEXT;
```

#### 3.3 Server Implementation

> **Critical:** Prevent infinite recursion when failure handlers fail.

```rust
async fn fail_workflow(&self, request: Request<FailWorkflowRequest>) -> Result<Response<Empty>, Status> {
    let req = request.into_inner();
    let run_id = uuid::Uuid::parse_str(&req.run_id)?;

    let workflow = sqlx::query!(
        "SELECT namespace_id, workflow_type, task_queue, input, attempts, started_at,
                on_failure_workflow, on_failure_task_queue, business_id
         FROM kagzi.workflow_runs WHERE run_id = $1",
        run_id
    ).fetch_one(&self.pool).await?;

    // Mark as failed
    sqlx::query!(
        "UPDATE kagzi.workflow_runs SET status = 'FAILED', error = $2, finished_at = NOW(),
         locked_by = NULL, locked_until = NULL WHERE run_id = $1",
        run_id, req.error
    ).execute(&self.pool).await?;

    // ============================================
    // CRITICAL: Prevent infinite recursion!
    // Don't trigger failure handlers for failure handler workflows
    // ============================================
    let is_failure_handler = workflow.business_id.starts_with("failure-")
                          || workflow.business_id.starts_with("system-failure-");

    if is_failure_handler {
        error!(run_id = %run_id, workflow_type = %workflow.workflow_type,
               "Failure handler workflow failed. Halting recursion.");
        return Ok(Response::new(Empty {}));
    }

    // Build failure event payload
    let failure_event = WorkflowFailedEvent {
        failed_run_id: run_id.to_string(),
        namespace_id: workflow.namespace_id.clone(),
        workflow_type: workflow.workflow_type.clone(),
        task_queue: workflow.task_queue.clone(),
        error: req.error.clone(),
        original_input: serde_json::to_vec(&workflow.input)?,
        attempts: workflow.attempts,
        started_at: workflow.started_at.map(to_timestamp),
        failed_at: Some(now_timestamp()),
    };
    let failure_input = serde_json::to_value(&failure_event)?;

    // 1. Trigger per-workflow onFailure handler if configured
    if let Some(ref on_failure) = workflow.on_failure_workflow {
        let task_queue = workflow.on_failure_task_queue.as_ref()
            .unwrap_or(&workflow.task_queue);

        sqlx::query!(
            "INSERT INTO kagzi.workflow_runs
             (business_id, namespace_id, task_queue, workflow_type, status, input)
             VALUES ($1, $2, $3, $4, 'PENDING', $5)",
            format!("failure-{}", run_id),
            workflow.namespace_id,
            task_queue,
            on_failure,
            failure_input.clone()
        ).execute(&self.pool).await?;

        info!(run_id = %run_id, handler = %on_failure, "Triggered per-workflow failure handler");
    }

    // 2. Trigger global system event handler if registered
    // Look for any workflow that has successfully processed failures before
    let global_handler_exists = sqlx::query_scalar!(
        "SELECT 1 FROM kagzi.workflow_runs
         WHERE workflow_type = '__kagzi_failure_handler'
           AND namespace_id = $1
         LIMIT 1",
        workflow.namespace_id
    ).fetch_optional(&self.pool).await?.is_some();

    if global_handler_exists {
        sqlx::query!(
            "INSERT INTO kagzi.workflow_runs
             (business_id, namespace_id, task_queue, workflow_type, status, input)
             VALUES ($1, $2, $3, '__kagzi_failure_handler', 'PENDING', $4)",
            format!("system-failure-{}", run_id),
            workflow.namespace_id,
            workflow.task_queue,
            failure_input
        ).execute(&self.pool).await.ok();

        info!(run_id = %run_id, "Triggered global failure handler");
    }

    Ok(Response::new(Empty {}))
}
```

#### 3.4 SDK Usage

```rust
// ============================================
// Pattern 1: Per-workflow failure handler
// ============================================

let run_id = client
    .workflow("ProcessPayment", "payments", input)
    .on_failure("HandlePaymentFailure")
    .await?;

async fn handle_payment_failure(
    mut ctx: WorkflowContext,
    event: WorkflowFailedEvent,
) -> anyhow::Result<()> {
    ctx.run("send_alert", async {
        slack.send(format!(
            "Payment {} failed after {} attempts: {}",
            event.failed_run_id,
            event.attempts,
            event.error
        )).await
    }).await?;

    ctx.run("save_to_dlq", async {
        db.dead_letter_queue.insert(DeadLetterEntry {
            original_run_id: event.failed_run_id,
            workflow_type: event.workflow_type,
            error: event.error,
            input: event.original_input,
            failed_at: event.failed_at,
        }).await
    }).await?;

    Ok(())
}

worker.register("HandlePaymentFailure", handle_payment_failure);


// ============================================
// Pattern 2: Global failure handler (catch-all)
// ============================================

async fn global_failure_handler(
    mut ctx: WorkflowContext,
    event: WorkflowFailedEvent,
) -> anyhow::Result<()> {
    ctx.run("send_to_datadog", async {
        datadog.increment("workflow.failed", &[
            format!("workflow_type:{}", event.workflow_type),
        ]).await
    }).await?;

    ctx.run("page_oncall", async {
        pagerduty.trigger(PagerDutyEvent {
            severity: "error",
            summary: format!("{} failed: {}", event.workflow_type, event.error),
        }).await
    }).await?;

    Ok(())
}

// Register with reserved name
worker.register("__kagzi_failure_handler", global_failure_handler);
```

#### 3.5 Design Notes

| Aspect                   | Kagzi Approach                                                      |
| ------------------------ | ------------------------------------------------------------------- |
| **DLQ**                  | None built-in. Users create their own in `onFailure` handler        |
| **Failure Event**        | `kagzi/workflow.failed` - emitted for ALL failures                  |
| **Per-Workflow Handler** | `.on_failure("HandlerWorkflow")` builder method                     |
| **Global Handler**       | Register `__kagzi_failure_handler` workflow                         |
| **Recursion Prevention** | Failure handlers are marked and won't trigger more handlers         |
| **Recovery**             | Query failed workflows + restart manually (future: bulk replay API) |

---

## 4. Workflow Timeouts (Deadline Enforcement)

### Current State

`deadline_at` field exists but is never enforced.

### Kagzi Implementation

#### 4.1 Add TIMED_OUT Status

```protobuf
enum WorkflowStatus {
  // ... existing ...
  WORKFLOW_STATUS_TIMED_OUT = 7;
}
```

#### 4.2 Reaper Enforcement

```rust
// In reaper.rs

// Enforce workflow deadlines
let deadline_result = sqlx::query!(
    r#"
    UPDATE kagzi.workflow_runs
    SET status = 'TIMED_OUT',
        error = 'Workflow execution timeout exceeded',
        finished_at = NOW(),
        locked_by = NULL,
        locked_until = NULL
    WHERE status IN ('RUNNING', 'SLEEPING', 'PENDING')
      AND deadline_at IS NOT NULL
      AND deadline_at < NOW()
    RETURNING run_id, workflow_type, namespace_id, task_queue, input,
              attempts, started_at, on_failure_workflow, on_failure_task_queue, business_id
    "#
).fetch_all(&pool).await;

match deadline_result {
    Ok(rows) => {
        for row in &rows {
            warn!(run_id = %row.run_id, "Workflow timed out - deadline exceeded");

            // Trigger failure handler if configured (respecting recursion prevention)
            let is_failure_handler = row.business_id.starts_with("failure-")
                                  || row.business_id.starts_with("system-failure-");

            if !is_failure_handler {
                if let Some(ref handler) = row.on_failure_workflow {
                    // Start failure handler workflow...
                }
            }
        }
    }
    Err(e) => error!("Failed to enforce deadlines: {:?}", e),
}
```

#### 4.3 SDK Usage

```rust
use chrono::{Utc, Duration as ChronoDuration};

// Using duration helper
let run_id = client
    .workflow("LongProcess", "default", input)
    .execution_timeout(Duration::from_secs(3600))  // 1 hour
    .await?;

// Using explicit deadline
let run_id = client
    .workflow("LongProcess", "default", input)
    .deadline(Utc::now() + ChronoDuration::hours(1))
    .await?;
```

---

## 5. Backward Pagination

### Current State

Only forward pagination. `prev_page_token` always empty.

### Implementation

Use direction prefix in cursor: `next:<cursor>` or `prev:<cursor>`

```rust
async fn list_workflow_runs(&self, request: Request<ListWorkflowRunsRequest>) -> ... {
    let req = request.into_inner();

    enum Direction {
        Forward(Option<Cursor>),
        Backward(Cursor),
    }

    let direction = if req.page_token.is_empty() {
        Direction::Forward(None)
    } else if req.page_token.starts_with("prev:") {
        Direction::Backward(parse_cursor(&req.page_token[5..])?)
    } else {
        let token = req.page_token.strip_prefix("next:").unwrap_or(&req.page_token);
        Direction::Forward(Some(parse_cursor(token)?))
    };

    let (rows, is_backward) = match direction {
        Direction::Forward(cursor) => {
            let rows = query_forward(&self.pool, &namespace_id, cursor, limit).await?;
            (rows, false)
        }
        Direction::Backward(cursor) => {
            let mut rows = query_backward(&self.pool, &namespace_id, cursor, limit).await?;
            rows.reverse();  // Maintain DESC order
            (rows, true)
        }
    };

    // Generate tokens
    let next_token = if has_more { format!("next:{}", encode(rows.last())) } else { String::new() };
    let prev_token = if has_prev { format!("prev:{}", encode(rows.first())) } else { String::new() };

    Ok(Response::new(ListWorkflowRunsResponse {
        workflow_runs,
        next_page_token: next_token,
        prev_page_token: prev_token,
        has_more,
    }))
}
```

---

## 6. Implementation Priority

| Priority | Feature                          | Effort | Impact    | Notes                             |
| -------- | -------------------------------- | ------ | --------- | --------------------------------- |
| **P0**   | **Automatic Retries**            | High   | Very High | Utilizes existing Reaper patterns |
| **P1**   | **Configurable Timeouts**        | Medium | Medium    | Requires SDK <-> Server contract  |
| **P2**   | **Failure Handlers**             | Medium | Medium    | Purely additive logic             |
| **P3**   | **Workflow Timeout Enforcement** | Low    | Medium    | `deadline_at` already exists      |
| **P4**   | **Backward Pagination**          | Low    | Low       | Nice to have                      |

### Suggested Order

**Phase 1 (Week 1-2)**: Automatic Retries

- Add `RetryPolicy` to proto
- Database migration (with backfill for existing rows)
- Server-side retry logic in `FailStep`
- **Update Reaper orphan recovery to use backoff** (critical!)
- SDK error types (`NonRetryableError`, `RetryAfterError`)

**Phase 2 (Week 2-3)**: Configurable Timeouts

- Add `TimeoutConfig` to proto
- Add `timeout_at` column to `step_runs`
- **Reaper: Enforce step timeouts server-side**
- **SDK: Wrap step execution with `tokio::time::timeout`**
- Update SDK builder

**Phase 3 (Week 3-4)**: Failure Handlers + Deadline Enforcement

- Add `on_failure_workflow` fields
- **Implement recursion prevention in `FailWorkflow`**
- Enforce `deadline_at` in reaper
- Add `TIMED_OUT` status
- Document patterns

**Phase 4 (Week 4)**: Polish

- Backward pagination
- Documentation updates
- Example updates

---

## Migration Checklist

For each feature:

- [ ] Protocol buffer changes
- [ ] Database migration (with backfill for existing data)
- [ ] Server implementation
- [ ] Reaper updates
- [ ] SDK implementation
- [ ] Integration tests
- [ ] Example updated
