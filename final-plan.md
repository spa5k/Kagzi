# Kagzi API Redesign - Final Implementation Plan

> **Goal**: Simplify and standardize the API before public release. Prioritize developer experience over flexibility. Avoid Temporal-level complexity.

---

## Table of Contents

1. [Design Decisions](#1-design-decisions)
2. [Worker API](#2-worker-api)
3. [Retry API](#3-retry-api)
4. [Step API](#4-step-api)
5. [Sleep API](#5-sleep-api)
6. [Client API](#6-client-api)
7. [Queue Model](#7-queue-model)
8. [Concurrency Model](#8-concurrency-model)
9. [Server Changes](#9-server-changes)
10. [Removals](#10-removals)
11. [Dependencies](#11-dependencies)
12. [File Changes](#12-file-changes)
13. [Migration Guide](#13-migration-guide)
14. [Implementation Order](#14-implementation-order)
15. [Future Considerations](#15-future-considerations)

---

## 1. Design Decisions

| Area                  | Decision                           | Rationale                                       |
| --------------------- | ---------------------------------- | ----------------------------------------------- |
| Workflow registration | Builder-time `.workflows([...])`   | All workflows defined at startup, cleaner API   |
| Duration parsing      | `humantime` crate                  | Standard, well-maintained, handles edge cases   |
| Step input            | Closure capture                    | Simpler API, output memoization is sufficient   |
| Sleep names           | Not unique, observability only     | Names are for debugging, not identity           |
| Middleware            | Skip                               | YAGNI, current SQL transitions are atomic       |
| Queue model           | Hidden, derived from workflow type | Reduces cognitive load, fewer misconfigurations |
| Concurrency           | Worker-level only                  | Simple, covers most use cases                   |
| Per-key concurrency   | Skip for v1                        | Complex, add later if requested                 |
| Throttling            | Skip for v1                        | Concurrency limits cover most cases             |
| Namespace             | Strict tenant isolation            | Always required, never cross-pollinated         |
| Trigger model         | Explicit `start("workflow-type")`  | Simpler than event-based routing                |
| Input types           | Runtime checking                   | Worker-side type safety is sufficient           |

---

## 2. Worker API

### Before

```rust
let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
    .namespace("production")
    .max_concurrent(50)
    .queue_concurrency_limit(100)
    .workflow_type_concurrency("process-order", 10)
    .default_step_retry(RetryPolicy {
        maximum_attempts: Some(3),
        initial_interval: Some(Duration::from_secs(1)),
        backoff_coefficient: Some(2.0),
        maximum_interval: Some(Duration::from_secs(60)),
        non_retryable_errors: vec![],
    })
    .build()
    .await?;

worker.register("process-order", process_order);
worker.register("send-notification", send_notification);
worker.run().await?;
```

### After

```rust
let worker = Worker::new("http://localhost:50051")
    .namespace("production")
    .max_concurrent(50)
    .retries(3)
    .workflows([
        ("process-order", process_order),
        ("send-notification", send_notification),
    ])
    .build()
    .await?;

worker.run().await?;
```

### Implementation

```rust
pub struct WorkerBuilder {
    addr: String,
    namespace_id: String,
    max_concurrent: usize,
    default_retry: Option<Retry>,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    workflows: Vec<(String, WorkflowFn)>,
}

impl WorkerBuilder {
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
            namespace_id: "default".to_string(),
            max_concurrent: 100,
            default_retry: None,
            hostname: None,
            version: None,
            labels: HashMap::new(),
            workflows: Vec::new(),
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

    /// Simple retry count with default exponential backoff
    pub fn retries(mut self, n: u32) -> Self {
        self.default_retry = Some(Retry::exponential(n));
        self
    }

    /// Full retry configuration
    pub fn retry(mut self, r: Retry) -> Self {
        self.default_retry = Some(r);
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

    pub fn workflows<I, F, Fut, In, Out>(mut self, workflows: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, F)>,
        F: Fn(Context, In) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<Out>> + Send + 'static,
        In: DeserializeOwned + Send + 'static,
        Out: Serialize + Send + 'static,
    {
        for (name, handler) in workflows {
            self.workflows.push((name.to_string(), wrap_handler(handler)));
        }
        self
    }

    pub async fn build(self) -> anyhow::Result<Worker> {
        if self.workflows.is_empty() {
            anyhow::bail!("At least one workflow must be registered");
        }
        // ... build worker
    }
}

pub struct Worker;

impl Worker {
    pub fn new(addr: &str) -> WorkerBuilder {
        WorkerBuilder::new(addr)
    }
}
```

### Key Changes

- `Worker::new(addr)` - no queue parameter
- `.workflows([...])` - register all workflows in builder
- `.retries(n)` - simple default, or `.retry(Retry::...)` for full control
- Remove `.register()` method
- Remove `queue_concurrency_limit`
- Remove `workflow_type_concurrency`

---

## 3. Retry API

### Before

```rust
RetryPolicy {
    maximum_attempts: Some(3),
    initial_interval: Some(Duration::from_secs(1)),
    backoff_coefficient: Some(2.0),
    maximum_interval: Some(Duration::from_secs(60)),
    non_retryable_errors: vec!["InvalidArgument".to_string()],
}
```

### After

```rust
// Simple - just attempts with defaults
Retry::attempts(3)

// Exponential backoff (most common)
Retry::exponential(5)
    .initial("1s")
    .max("60s")
    .non_retryable(["InvalidArgument", "NotFound"])

// Linear (fixed interval)
Retry::linear(5, "2s")

// Presets
Retry::none()      // No retries (attempts = 0)
Retry::forever()   // Infinite retries with backoff
```

### Implementation

```rust
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Retry {
    pub attempts: u32,
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub backoff_coefficient: f64,
    pub non_retryable_errors: Vec<String>,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            attempts: 3,
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            backoff_coefficient: 2.0,
            non_retryable_errors: Vec::new(),
        }
    }
}

impl Retry {
    /// Create retry with N attempts and default exponential backoff
    pub fn attempts(n: u32) -> Self {
        Self {
            attempts: n,
            ..Default::default()
        }
    }

    /// Create retry with exponential backoff
    /// Default: initial=1s, max=60s, coefficient=2.0
    pub fn exponential(attempts: u32) -> Self {
        Self::attempts(attempts)
    }

    /// Create retry with fixed interval (no backoff)
    pub fn linear(attempts: u32, interval: &str) -> Self {
        let d = humantime::parse_duration(interval)
            .expect("Invalid duration format");
        Self {
            attempts,
            initial_interval: d,
            max_interval: d,
            backoff_coefficient: 1.0,
            non_retryable_errors: Vec::new(),
        }
    }

    /// No retries - fail immediately
    pub fn none() -> Self {
        Self {
            attempts: 0,
            ..Default::default()
        }
    }

    /// Retry forever with exponential backoff
    pub fn forever() -> Self {
        Self {
            attempts: u32::MAX,
            ..Default::default()
        }
    }

    /// Set initial retry interval (e.g., "1s", "500ms", "1m")
    pub fn initial(mut self, duration: &str) -> Self {
        self.initial_interval = humantime::parse_duration(duration)
            .expect("Invalid duration format");
        self
    }

    /// Set maximum retry interval (e.g., "60s", "5m")
    pub fn max(mut self, duration: &str) -> Self {
        self.max_interval = humantime::parse_duration(duration)
            .expect("Invalid duration format");
        self
    }

    /// Set backoff coefficient (default: 2.0)
    pub fn backoff(mut self, coefficient: f64) -> Self {
        self.backoff_coefficient = coefficient;
        self
    }

    /// Set error types that should not be retried
    pub fn non_retryable<I, S>(mut self, errors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.non_retryable_errors = errors.into_iter().map(Into::into).collect();
        self
    }
}

// Convert to proto
impl From<Retry> for kagzi_proto::kagzi::RetryPolicy {
    fn from(r: Retry) -> Self {
        Self {
            maximum_attempts: r.attempts as i32,
            initial_interval_ms: r.initial_interval.as_millis() as i64,
            backoff_coefficient: r.backoff_coefficient,
            maximum_interval_ms: r.max_interval.as_millis() as i64,
            non_retryable_errors: r.non_retryable_errors,
        }
    }
}
```

---

## 4. Step API

### Before

```rust
// Three separate methods with increasing complexity
let result = ctx.run("fetch-user", async {
    fetch_user(user_id).await
}).await?;

let result = ctx.run_with_input("transform", &data, async {
    transform(data).await
}).await?;

let result = ctx.run_with_input_with_retry("call-api", &req, Some(policy), async {
    call_api(req).await
}).await?;
```

### After

```rust
// Single builder pattern - extensible without API breakage
let user = ctx.step("fetch-user")
    .run(|| fetch_user(user_id))
    .await?;

let result = ctx.step("transform-data")
    .run(|| transform(&data))
    .await?;

let response = ctx.step("call-external-api")
    .retry(Retry::exponential(10).initial("500ms"))
    .timeout("30s")
    .run(|| call_api(&request))
    .await?;
```

### Implementation

```rust
impl Context {
    /// Start building a step
    pub fn step(&mut self, name: &str) -> StepBuilder<'_> {
        StepBuilder {
            ctx: self,
            name: name.to_string(),
            retry: None,
            timeout: None,
        }
    }
}

pub struct StepBuilder<'a> {
    ctx: &'a mut Context,
    name: String,
    retry: Option<Retry>,
    timeout: Option<Duration>,
}

impl<'a> StepBuilder<'a> {
    /// Override retry policy for this step
    pub fn retry(mut self, r: Retry) -> Self {
        self.retry = Some(r);
        self
    }

    /// Set timeout for this step (e.g., "30s", "5m")
    pub fn timeout(mut self, duration: &str) -> Self {
        self.timeout = Some(
            humantime::parse_duration(duration)
                .expect("Invalid duration format")
        );
        self
    }

    /// Execute the step
    pub async fn run<F, Fut, R>(self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<R>> + Send,
        R: Serialize + DeserializeOwned + Send + 'static,
    {
        // Determine effective retry policy
        let retry = self.retry
            .or_else(|| self.ctx.default_retry.clone());

        // Call BeginStep
        let begin_resp = self.ctx.client
            .begin_step(BeginStepRequest {
                run_id: self.ctx.run_id.clone(),
                step_name: self.name.clone(),
                kind: StepKind::Function as i32,
                input: None,  // No longer serializing input
                retry_policy: retry.map(Into::into),
                timeout_ms: self.timeout.map(|d| d.as_millis() as i64),
            })
            .await?
            .into_inner();

        // Return cached result if step already completed
        if !begin_resp.should_execute {
            let cached = begin_resp.cached_output.unwrap_or_default();
            return Ok(serde_json::from_slice(&cached.data)?);
        }

        // Execute the step
        let result = f().await;

        // Report result
        match result {
            Ok(value) => {
                let output = serde_json::to_vec(&value)?;
                self.ctx.client
                    .complete_step(CompleteStepRequest {
                        run_id: self.ctx.run_id.clone(),
                        step_id: self.name,
                        output: Some(Payload { data: output, metadata: HashMap::new() }),
                    })
                    .await?;
                Ok(value)
            }
            Err(e) => {
                self.ctx.client
                    .fail_step(FailStepRequest {
                        run_id: self.ctx.run_id.clone(),
                        step_id: self.name,
                        error: e.to_string(),
                        non_retryable: false,  // Could be configurable
                        retry_after_ms: None,
                    })
                    .await?;
                Err(e)
            }
        }
    }
}
```

### Why Closure Capture Works

In durable execution, we only memoize **outputs**, not inputs:

1. Workflow input is serialized once at workflow start
2. Each step output is serialized after completion
3. On replay, completed steps return cached output without re-executing
4. The closure captures variables from scope, which are deterministically derived from (1) and (2)

```rust
async fn order_workflow(ctx: Context, input: OrderInput) -> Result<()> {
    // `input` comes from workflow input (serialized at start)
    let user = ctx.step("fetch-user")
        .run(|| fetch_user(input.user_id))  // captures input.user_id
        .await?;

    // `user` comes from previous step output (serialized after step completed)
    let charge = ctx.step("charge-card")
        .run(|| charge_card(&user))  // captures user
        .await?;

    Ok(())
}
```

---

## 5. Sleep API

### Before

```rust
ctx.sleep(Duration::from_secs(30)).await?;
ctx.sleep(Duration::from_secs(86400)).await?;  // What is this? 1 day
```

### After

```rust
ctx.sleep("wait-for-cooldown", "30s").await?;
ctx.sleep("wait-next-day", "1 day").await?;
ctx.sleep("retry-delay", "5m 30s").await?;

// Sleep until specific time
ctx.sleep_until("deadline", deadline_timestamp).await?;
```

### Implementation

```rust
impl Context {
    /// Sleep for a duration with a descriptive name
    ///
    /// The name is used for observability (logs, UI) but does not need to be unique.
    ///
    /// Duration formats: "30s", "5m", "1h", "1 day", "2 weeks"
    pub async fn sleep(&mut self, name: &str, duration: &str) -> anyhow::Result<()> {
        let d = humantime::parse_duration(duration)
            .map_err(|e| anyhow::anyhow!("Invalid duration '{}': {}", duration, e))?;
        self.sleep_internal(name, d).await
    }

    /// Sleep until a specific timestamp
    pub async fn sleep_until(&mut self, name: &str, until: DateTime<Utc>) -> anyhow::Result<()> {
        let now = Utc::now();
        if until <= now {
            return Ok(());  // Already past, no sleep needed
        }
        let duration = (until - now).to_std().unwrap_or(Duration::ZERO);
        self.sleep_internal(name, duration).await
    }

    async fn sleep_internal(&mut self, name: &str, duration: Duration) -> anyhow::Result<()> {
        self.client
            .sleep(SleepRequest {
                run_id: self.run_id.clone(),
                step_name: name.to_string(),
                duration: Some(prost_types::Duration {
                    seconds: duration.as_secs() as i64,
                    nanos: duration.subsec_nanos() as i32,
                }),
            })
            .await
            .map_err(map_grpc_error)?;

        // Signal that workflow should pause
        Err(WorkflowPaused.into())
    }
}
```

### Supported Duration Formats (via humantime)

- Seconds: `"30s"`, `"30sec"`, `"30 seconds"`
- Minutes: `"5m"`, `"5min"`, `"5 minutes"`
- Hours: `"2h"`, `"2hr"`, `"2 hours"`
- Days: `"1d"`, `"1 day"`, `"1 days"`
- Weeks: `"1w"`, `"1 week"`
- Combined: `"1h 30m"`, `"1 day 12 hours"`

---

## 6. Client API

### Before

```rust
let mut client = KagziClient::connect("http://localhost:50051").await?;
let run_id = client.workflow("process-order", "orders", input).await?;

// Schedule
client.create_workflow_schedule(CreateWorkflowScheduleRequest {
    schedule_id: "daily-report".into(),
    workflow_type: "generate-report".into(),
    task_queue: "reports".into(),
    namespace_id: "prod".into(),
    cron_expr: "0 6 * * *".into(),
    enabled: true,
    max_catchup: 7,
    version: "1".into(),
    input: Some(Payload { ... }),
}).await?;
```

### After

```rust
let client = Kagzi::connect("http://localhost:50051").await?;

// Start a workflow
let run = client
    .start("process-order")
    .namespace("production")
    .input(OrderInput { user_id: 123, items: vec![...] })
    .id("order-123")  // Optional idempotency key
    .await?;

println!("Started workflow: {}", run.id);

// Schedule a recurring workflow
client
    .schedule("daily-report")
    .namespace("production")
    .workflow("generate-report")
    .cron("0 6 * * *")
    .input(ReportConfig { ... })
    .catchup(7)
    .await?;

// Query workflow status
let status = client.get(&run.id).namespace("production").await?;

// Cancel a workflow
client.cancel(&run.id).namespace("production").await?;
```

### Implementation

```rust
pub struct Kagzi {
    client: AdminServiceClient<Channel>,
}

impl Kagzi {
    pub async fn connect(addr: &str) -> anyhow::Result<Self> {
        let client = AdminServiceClient::connect(addr.to_string()).await?;
        Ok(Self { client })
    }

    pub fn start(&self, workflow_type: &str) -> StartWorkflowBuilder<'_> {
        StartWorkflowBuilder {
            client: &self.client,
            workflow_type: workflow_type.to_string(),
            namespace: "default".to_string(),
            input: None,
            idempotency_key: None,
        }
    }

    pub fn schedule(&self, schedule_id: &str) -> ScheduleBuilder<'_> {
        ScheduleBuilder {
            client: &self.client,
            schedule_id: schedule_id.to_string(),
            namespace: "default".to_string(),
            workflow_type: None,
            cron: None,
            input: None,
            max_catchup: 100,
            enabled: true,
        }
    }

    pub fn get(&self, run_id: &str) -> GetWorkflowBuilder<'_> {
        GetWorkflowBuilder {
            client: &self.client,
            run_id: run_id.to_string(),
            namespace: "default".to_string(),
        }
    }

    pub fn cancel(&self, run_id: &str) -> CancelWorkflowBuilder<'_> {
        CancelWorkflowBuilder {
            client: &self.client,
            run_id: run_id.to_string(),
            namespace: "default".to_string(),
        }
    }
}

pub struct StartWorkflowBuilder<'a> {
    client: &'a AdminServiceClient<Channel>,
    workflow_type: String,
    namespace: String,
    input: Option<Vec<u8>>,
    idempotency_key: Option<String>,
}

impl<'a> StartWorkflowBuilder<'a> {
    pub fn namespace(mut self, ns: &str) -> Self {
        self.namespace = ns.to_string();
        self
    }

    pub fn input<T: Serialize>(mut self, input: T) -> Self {
        self.input = Some(serde_json::to_vec(&input).expect("Failed to serialize input"));
        self
    }

    /// Set idempotency key to prevent duplicate workflows
    pub fn id(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    pub async fn await(self) -> anyhow::Result<WorkflowRun> {
        let mut client = self.client.clone();
        let resp = client
            .start_workflow(StartWorkflowRequest {
                workflow_type: self.workflow_type.clone(),
                task_queue: self.workflow_type,  // Queue = workflow type
                namespace_id: self.namespace,
                input: self.input.map(|data| Payload { data, metadata: HashMap::new() }),
                external_id: self.idempotency_key.unwrap_or_else(|| Uuid::new_v4().to_string()),
                ..Default::default()
            })
            .await?
            .into_inner();

        Ok(WorkflowRun { id: resp.run_id })
    }
}

#[derive(Debug)]
pub struct WorkflowRun {
    pub id: String,
}
```

---

## 7. Queue Model

### Design

Queues are **hidden** from users. Internally, `task_queue = workflow_type`.

```
User sees:                    Internal:
─────────────────────────────────────────────────
workflow_type="process-order" → task_queue="process-order"
workflow_type="send-email"    → task_queue="send-email"
```

### Why Hide Queues

1. **Reduces configuration errors** - no mismatch between queue and workflow type
2. **Simpler mental model** - users think in terms of workflows, not queues
3. **Matches Inngest/Hatchet** - proven simpler for users
4. **Queue is an implementation detail** - routing is what users care about

### Multi-Workflow Workers

A worker can handle multiple workflow types. Internally, it polls all corresponding queues:

```rust
Worker::new(addr)
    .workflows([
        ("process-order", process_order),   // Polls queue "process-order"
        ("send-notification", send_notif),  // Polls queue "send-notification"
    ])
```

The server routes each workflow to the appropriate internal queue. Workers poll all queues they're registered for.

---

## 8. Concurrency Model

### Single Level: Worker Concurrency

```rust
Worker::new(addr)
    .max_concurrent(50)  // Max 50 workflows running on this worker
    .workflows([...])
```

Enforced client-side with a semaphore (already implemented).

### What Gets Removed

| Removed                     | Why                                           |
| --------------------------- | --------------------------------------------- |
| `queue_concurrency_limit`   | Not enforced, unclear purpose                 |
| `workflow_type_concurrency` | Adds complexity, use separate workers instead |
| Per-key concurrency         | Complex, skip for v1                          |
| Throttling                  | Concurrency limits suffice for v1             |

### If Per-Type Limits Are Needed

Run separate workers:

```rust
// High-priority order processing - more capacity
Worker::new(addr)
    .max_concurrent(100)
    .workflows([("process-order", process_order)])
    .build().await?;

// Low-priority notifications - limited capacity
Worker::new(addr)
    .max_concurrent(10)
    .workflows([("send-notification", send_notif)])
    .build().await?;
```

---

## 9. Server Changes

### Worker Registration

When a worker registers, the server records each workflow type:

```rust
// RegisterRequest now includes workflow_types but no task_queue
// Server internally sets task_queue = workflow_type for routing

for workflow_type in request.workflow_types {
    // Store mapping: this worker handles this workflow type
    // When polling, worker specifies which types it handles
}
```

### Start Workflow

When client starts a workflow, server derives queue:

```rust
// StartWorkflowRequest
// - workflow_type: "process-order"
// - task_queue: not provided (or ignored)

// Server logic:
let task_queue = request.workflow_type.clone();  // Queue = workflow type
```

### Poll Logic

Unchanged. Worker polls by namespace + queue + types:

```sql
WHERE namespace_id = $1
  AND task_queue = $2
  AND workflow_type = ANY($3)
```

With hidden queues, `task_queue` always equals one of the `workflow_types`, so the query still works correctly.

### Proto Changes (Optional)

Could simplify `RegisterRequest` and `StartWorkflowRequest` by removing `task_queue` field, but can also just ignore it server-side for backward compatibility.

---

## 10. Removals

| What                                 | Where                 | Reason                                  |
| ------------------------------------ | --------------------- | --------------------------------------- |
| `task_queue` parameter               | Worker builder        | Hidden, derived from workflow types     |
| `queue` parameter                    | Client start workflow | Hidden, derived from workflow type      |
| `queue_concurrency_limit`            | Worker builder        | Unused, not enforced                    |
| `workflow_type_concurrency`          | Worker builder        | Complexity, use separate workers        |
| `run()` method                       | Context               | Replaced by `step().run()`              |
| `run_with_input()` method            | Context               | Replaced by `step().run()`              |
| `run_with_input_with_retry()` method | Context               | Replaced by `step().run().retry()`      |
| `register()` method                  | Worker                | Replaced by `.workflows([])` in builder |
| `sleep_counter` field                | Context               | Named sleeps replace auto-increment     |
| `RetryPolicy` struct                 | SDK                   | Replaced by `Retry` builder             |

---

## 11. Dependencies

### Add

```toml
[dependencies]
humantime = "2.1" # Duration parsing
```

### Already Present (no changes)

- `serde` / `serde_json` - serialization
- `tokio` - async runtime
- `tonic` - gRPC
- `anyhow` - error handling
- `uuid` - ID generation

---

## 12. File Changes

| File                                        | Changes                                                               |
| ------------------------------------------- | --------------------------------------------------------------------- |
| `sdk/kagzi-rs/src/lib.rs`                   | Re-export `Retry`, `Context`, `Worker`, `Kagzi`                       |
| `sdk/kagzi-rs/src/retry.rs`                 | **Rewrite**: New `Retry` builder with presets                         |
| `sdk/kagzi-rs/src/worker.rs`                | **Refactor**: New builder API, remove queue param, add `.workflows()` |
| `sdk/kagzi-rs/src/context.rs`               | **New file**: Extract from `workflow_context.rs`, add `StepBuilder`   |
| `sdk/kagzi-rs/src/workflow_context.rs`      | **Delete** or rename to `context.rs`                                  |
| `sdk/kagzi-rs/src/client.rs`                | **Refactor**: New fluent API with builders                            |
| `sdk/kagzi-rs/src/step.rs`                  | **New file**: `StepBuilder` implementation                            |
| `sdk/kagzi-rs/Cargo.toml`                   | Add `humantime` dependency                                            |
| `crates/kagzi-server/src/worker_service.rs` | Auto-derive queue from workflow type                                  |
| `crates/kagzi-server/src/admin_service.rs`  | Auto-derive queue from workflow type                                  |
| `proto/worker.proto`                        | Optional: deprecate `task_queue` field                                |
| `proto/admin.proto`                         | Optional: deprecate `task_queue` field                                |

---

## 13. Migration Guide

For existing users migrating from current API:

### Worker

```rust
// Before
let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
    .namespace("prod")
    .max_concurrent(50)
    .build()
    .await?;
worker.register("process-order", handler);
worker.run().await?;

// After
let worker = Worker::new("http://localhost:50051")
    .namespace("prod")
    .max_concurrent(50)
    .workflows([("process-order", handler)])
    .build()
    .await?;
worker.run().await?;
```

### Steps

```rust
// Before
let result = ctx.run("step-name", async { do_work().await }).await?;
let result = ctx.run_with_input("step", &input, async { work(input).await }).await?;

// After
let result = ctx.step("step-name").run(|| do_work()).await?;
let result = ctx.step("step").run(|| work(&input)).await?;
```

### Sleep

```rust
// Before
ctx.sleep(Duration::from_secs(30)).await?;

// After
ctx.sleep("wait-30s", "30s").await?;
```

### Client

```rust
// Before
let run_id = client.workflow("process-order", "orders", input).await?;

// After
let run = client.start("process-order").namespace("prod").input(input).await?;
```

### Retry Policy

```rust
// Before
RetryPolicy {
    maximum_attempts: Some(3),
    initial_interval: Some(Duration::from_secs(1)),
    backoff_coefficient: Some(2.0),
    maximum_interval: Some(Duration::from_secs(60)),
    non_retryable_errors: vec![],
}

// After
Retry::exponential(3).initial("1s").max("60s")
```

---

## 14. Implementation Order

Recommended order to minimize disruption:

### Phase 1: New Types (Non-Breaking)

1. **`Retry` builder** - new file, doesn't break existing code
2. **`StepBuilder`** - new type, add alongside existing methods
3. **Add `humantime` dependency**

### Phase 2: Context Changes

4. **Add `step()` method** to Context (coexists with old methods)
5. **Add new `sleep(name, duration)` method** (coexists with old)
6. **Deprecate old methods** with `#[deprecated]`

### Phase 3: Worker Changes

7. **Add `.workflows()` to builder**
8. **Make queue optional** (derive from workflow types)
9. **Deprecate `.register()` and queue param**

### Phase 4: Client Changes

10. **Add new fluent client API**
11. **Deprecate old methods**

### Phase 5: Cleanup

12. **Remove deprecated methods**
13. **Update examples and docs**
14. **Update server to ignore queue if workflow_type provided**

---

## 15. Future Considerations

Features explicitly deferred to later versions:

| Feature                  | Why Deferred                              | When to Add                            |
| ------------------------ | ----------------------------------------- | -------------------------------------- |
| Per-key concurrency      | Complex, requires server-side enforcement | When users request fairness controls   |
| Throttling               | Concurrency limits suffice                | When users hit rate limit use cases    |
| Event-based triggers     | Bigger architectural change               | If users want pub/sub patterns         |
| Typed workflow handles   | Adds macro/codegen complexity             | If type safety becomes pain point      |
| Step timeout enforcement | Requires server-side tracking             | When long-running steps need killing   |
| Workflow versioning      | Complex migration semantics               | When breaking changes need coexistence |

---

## Appendix: Complete Example

```rust
use kagzi::{Context, Kagzi, Retry, Worker};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct OrderInput {
    user_id: u64,
    items: Vec<String>,
    total: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderOutput {
    order_id: String,
    status: String,
}

async fn process_order(mut ctx: Context, input: OrderInput) -> anyhow::Result<OrderOutput> {
    // Fetch user with default retry
    let user = ctx.step("fetch-user")
        .run(|| fetch_user(input.user_id))
        .await?;

    // Charge card with custom retry and timeout
    let payment = ctx.step("charge-card")
        .retry(Retry::exponential(5).initial("1s").max("30s"))
        .timeout("2m")
        .run(|| charge_card(&user, input.total))
        .await?;

    // Wait for payment confirmation
    ctx.sleep("wait-confirmation", "30s").await?;

    // Create shipment
    let shipment = ctx.step("create-shipment")
        .run(|| create_shipment(&input.items))
        .await?;

    // Send confirmation email (no retry on failure)
    ctx.step("send-email")
        .retry(Retry::none())
        .run(|| send_confirmation(&user.email, &shipment))
        .await?;

    Ok(OrderOutput {
        order_id: payment.transaction_id,
        status: "completed".into(),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Start worker
    let worker = Worker::new("http://localhost:50051")
        .namespace("production")
        .max_concurrent(50)
        .retries(3)
        .workflows([
            ("process-order", process_order),
        ])
        .build()
        .await?;

    // In another process: trigger workflow
    let client = Kagzi::connect("http://localhost:50051").await?;
    let run = client
        .start("process-order")
        .namespace("production")
        .input(OrderInput {
            user_id: 123,
            items: vec!["item-1".into(), "item-2".into()],
            total: 99.99,
        })
        .id("order-123")
        .await?;

    println!("Started: {}", run.id);

    // Run worker (blocks)
    worker.run().await?;

    Ok(())
}
```
