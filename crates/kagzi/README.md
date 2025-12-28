# Kagzi Client Library

The Kagzi client library provides a Rust interface for defining workflows, managing workers, and interacting with the Kagzi workflow orchestration system via gRPC.

## Overview

Kagzi is a durable workflow orchestration system that guarantees at-least-once execution of workflow steps. The client library enables you to:

- Define workflows as async functions with typed inputs and outputs
- Start workflows programmatically or on schedules (cron expressions)
- Register workers to poll and execute workflows
- Configure retry policies and concurrency limits
- Leverage built-in distributed tracing and error handling

## Features

- **Type-safe workflows**: Define workflows with typed inputs/outputs using standard Rust futures
- **Durable execution**: Automatic step replay and idempotency guarantees
- **Retry policies**: Configurable exponential backoff with non-retryable error types
- **Concurrency control**: Per-queue and per-workflow-type limits
- **Scheduled workflows**: Cron-based workflow execution with catchup support
- **Distributed tracing**: Automatic correlation and trace ID propagation
- **Graceful shutdown**: Draining of active workflows and clean deregistration
- **Rich error handling**: Structured errors with retry hints and metadata

## Architecture

```
┌─────────────────┐         gRPC          ┌──────────────────┐
│   Client App    │◄─────────────────────►│   Kagzi Server   │
│                 │                        │                  │
│  - Start WF     │                        │  - Schedule DB  │
│  - List Schedules│                     │  - Queue Mgmt    │
└─────────────────┘                        │  - State Machine │
                                           └──────────────────┘
                                                   │
                                                   │ poll & execute
                                                   │
┌─────────────────┐         gRPC          ┌──────────────┐
│   Worker App    │◄─────────────────────►│   Worker     │
│                 │                        │  - Run WF    │
│  - Register WF  │                        │  - Heartbeat │
│  - Poll Tasks   │                        │  - Complete  │
└─────────────────┘                        └──────────────┘
```

### Key Components

1. **Client**: Starts workflows and manages schedules via gRPC
2. **Worker**: Polls for tasks, executes workflows, sends heartbeats
3. **WorkflowContext**: Passed to workflow functions, manages step execution and state
4. **RetryPolicy**: Controls how failed steps are retried
5. **KagziError**: Rich error type with retry semantics

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
kagzi = { version = "0.1.0" }
kagzi-macros = { version = "0.1.0" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
```

## Quick Start

### 1. Define a Workflow

```rust
use kagzi::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct ProcessOrderInput {
    order_id: String,
    customer_id: String,
    amount: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProcessOrderOutput {
    order_id: String,
    status: String,
    invoice_id: String,
}

async fn process_order(
    mut ctx: WorkflowContext,
    input: ProcessOrderInput,
) -> anyhow::Result<ProcessOrderOutput> {
    // Step 1: Validate order
    let validation_result = ctx
        .run_with_input(
            "validate-order",
            &input,
            async move {
                // Validation logic
                if input.amount <= 0.0 {
                    return Err(KagziError::non_retryable("Invalid amount"));
                }
                Ok(true)
            },
        )
        .await?;

    // Step 2: Process payment
    let payment_result = ctx
        .run("process-payment", async move {
            // Payment processing logic
            Ok("payment-123".to_string())
        })
        .await?;

    // Step 3: Generate invoice
    let invoice_id = ctx
        .run("generate-invoice", async move {
            // Invoice generation
            Ok("inv-456".to_string())
        })
        .await?;

    Ok(ProcessOrderOutput {
        order_id: input.order_id,
        status: "completed".to_string(),
        invoice_id,
    })
}
```

### 2. Start a Workflow (Client)

```rust
use kagzi::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Connect to Kagzi server
    let mut client = Client::connect("http://localhost:50051").await?;

    // Start a workflow
    let run_id = client
        .workflow(
            "process-order",
            "orders",
            ProcessOrderInput {
                order_id: "order-123".to_string(),
                customer_id: "customer-456".to_string(),
                amount: 99.99,
            },
        )
        .id("business-order-id-123")
        .namespace("production")
        .retries(5)
        .await?;

    println!("Workflow started with run_id: {}", run_id);
    Ok(())
}
```

### 3. Run a Worker

```rust
use kagzi::{Worker, WorkerBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Build and register worker
    let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
        .namespace("production")
        .max_concurrent(50)
        .queue_concurrency_limit(100)
        .default_step_retry(RetryPolicy {
            maximum_attempts: Some(3),
            initial_interval: Some(std::time::Duration::from_secs(1)),
            backoff_coefficient: Some(2.0),
            maximum_interval: Some(std::time::Duration::from_secs(60)),
            non_retryable_errors: vec![
                "InvalidArgument".to_string(),
                "PreconditionFailed".to_string(),
            ],
        })
        .build()
        .await?;

    // Register workflows
    worker.register("process-order", process_order);

    // Run worker (blocks until shutdown)
    worker.run().await?;

    Ok(())
}
```

## Usage Examples

### Scheduled Workflows

```rust
use kagzi::Client;
use chrono::{Utc, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = Client::connect("http://localhost:50051").await?;

    // Create a scheduled workflow
    let schedule = client
        .workflow_schedule(
            "daily-report",
            "reports",
            "0 6 * * *", // Cron: 6 AM daily
            ReportInput { report_type: "daily".to_string() },
        )
        .namespace("production")
        .enabled(true)
        .max_catchup(7) // Catch up to 7 missed executions
        .await?;

    println!("Schedule created: {}", schedule.schedule_id);
    Ok(())
}

#[derive(Serialize)]
struct ReportInput {
    report_type: String,
}
```

### Workflow with Sleep

```rust
use kagzi::WorkflowContext;
use std::time::Duration;

async fn delayed_workflow(
    mut ctx: WorkflowContext,
    input: String,
) -> anyhow::Result<String> {
    // Execute first step
    let result = ctx.run("step1", async move {
        Ok(format!("Processed: {}", input))
    }).await?;

    // Sleep for 30 seconds (workflow pauses, worker can process other tasks)
    ctx.sleep(Duration::from_secs(30)).await?;

    // Execute after sleep
    let final_result = ctx.run("step2", async move {
        Ok(format!("Final: {}", result))
    }).await?;

    Ok(final_result)
}
```

### Using Macros

The `kagzi-macros` crate provides convenience macros for defining workflows:

```rust
use kagzi::prelude::*;

#[kagzi_workflow]
async fn my_workflow(
    ctx: WorkflowContext,
    input: MyInput,
) -> anyhow::Result<MyOutput> {
    // Your workflow logic
    Ok(MyOutput {})
}
```

## API Reference

### Client

#### Connection

```rust
pub async fn connect(addr: &str) -> anyhow::Result<Client>
```

Connects to the Kagzi server at the specified address.

**Example:**

```rust
let mut client = Client::connect("http://localhost:50051").await?;
```

#### Starting Workflows

```rust
pub fn workflow<I: Serialize>(
    &mut self,
    workflow_type: &str,
    task_queue: &str,
    input: I,
) -> WorkflowBuilder<'_, I>
```

Creates a builder for starting a workflow.

**WorkflowBuilder Methods:**

- `id(external_id: impl Into<String>)` - Set business identifier
- `namespace(ns: impl Into<String>)` - Set namespace (default: "default")
- `version(version: impl Into<String>)` - Set workflow version
- `retry_policy(policy: RetryPolicy)` - Set retry policy
- `retries(max_attempts: i32)` - Shortcut to set max retry attempts

**Returns:** `anyhow::Result<String>` - The run_id of the started workflow

#### Managing Schedules

```rust
pub fn workflow_schedule<I: Serialize>(
    &mut self,
    workflow_type: &str,
    task_queue: &str,
    cron_expr: &str,
    input: I,
) -> WorkflowScheduleBuilder<'_, I>
```

Creates a builder for scheduling a workflow.

**WorkflowScheduleBuilder Methods:**

- `namespace(ns: impl Into<String>)` - Set namespace
- `enabled(enabled: bool)` - Enable/disable schedule
- `max_catchup(max_catchup: i32)` - Maximum missed executions to catch up
- `version(version: impl Into<String>)` - Set workflow version

**Returns:** `anyhow::Result<WorkflowSchedule>`

#### Querying Schedules

```rust
pub async fn get_workflow_schedule(
    &mut self,
    schedule_id: &str,
    namespace_id: Option<&str>,
) -> anyhow::Result<Option<WorkflowSchedule>>

pub async fn list_workflow_schedules(
    &mut self,
    namespace_id: &str,
    page: Option<PageRequest>,
) -> anyhow::Result<Vec<WorkflowSchedule>>

pub async fn delete_workflow_schedule(
    &mut self,
    schedule_id: &str,
    namespace_id: Option<&str>,
) -> anyhow::Result<()>
```

### Worker

#### WorkerBuilder

```rust
pub fn builder(addr: &str, task_queue: &str) -> WorkerBuilder
```

Creates a builder for configuring a worker.

**WorkerBuilder Methods:**

- `namespace(ns: &str)` - Set namespace (default: "default")
- `max_concurrent(n: usize)` - Max concurrent workflows (default: 100)
- `hostname(h: &str)` - Set hostname (default: auto-detected)
- `version(v: &str)` - Set worker version
- `label(key: &str, value: &str)` - Add worker label
- `queue_concurrency_limit(limit: i32)` - Set queue-wide concurrency limit
- `workflow_type_concurrency(workflow_type: &str, limit: i32)` - Per-workflow-type limit
- `default_step_retry(policy: RetryPolicy)` - Default retry policy for steps

#### Registering Workflows

```rust
pub fn register<F, Fut, I, O>(&mut self, name: &str, func: F)
where
    F: Fn(WorkflowContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<O>> + Send + 'static,
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
```

Registers a workflow function with the worker.

**Parameters:**

- `name`: Workflow type identifier
- `func`: Async function that takes `WorkflowContext` and input, returns output

#### Running the Worker

```rust
pub async fn run(&mut self) -> anyhow::Result<()>
```

Starts the worker polling loop. Blocks until shutdown signal is received.

**Note:** Must call `register()` before `run()` for at least one workflow.

#### Worker Methods

```rust
pub fn worker_id(&self) -> Option<Uuid>
pub fn is_registered(&self) -> bool
pub fn active_count(&self) -> usize
pub fn shutdown(&self)
pub fn shutdown_token(&self) -> CancellationToken
```

### WorkflowContext

Passed to workflow functions, provides methods for step execution and workflow control.

#### Execute Steps

```rust
pub async fn run<R, Fut>(&mut self, step_id: &str, fut: Fut) -> anyhow::Result<R>
where
    R: Serialize + DeserializeOwned + Send + 'static,
    Fut: Future<Output = anyhow::Result<R>> + Send,
```

Executes a step without input. Automatically retries on failure if step execution wasn't completed before.

**Example:**

```rust
let result = ctx.run("fetch-user", async {
    fetch_user_from_db(user_id).await
}).await?;
```

```rust
pub async fn run_with_input<I, R, Fut>(
    &mut self,
    step_id: &str,
    input: &I,
    fut: Fut,
) -> anyhow::Result<R>
where
    I: Serialize + Send + 'static,
    R: Serialize + DeserializeOwned + Send + 'static,
    Fut: Future<Output = anyhow::Result<R>> + Send,
```

Executes a step with typed input. Input is serialized and cached for replay.

**Example:**

```rust
let output = ctx
    .run_with_input(
        "transform-data",
        &input_data,
        async { transform(input_data).await },
    )
    .await?;
```

```rust
pub async fn run_with_input_with_retry<I, R, Fut>(
    &mut self,
    step_id: &str,
    input: &I,
    retry_policy: Option<RetryPolicy>,
    fut: Fut,
) -> anyhow::Result<R>
```

Same as `run_with_input` but allows specifying a custom retry policy for this step.

#### Sleep

```rust
pub async fn sleep(&mut self, duration: Duration) -> anyhow::Result<()>
```

Pauses workflow execution for the specified duration. Returns `WorkflowPaused` error.

**Important:** The workflow is paused and the worker can process other tasks. When the sleep duration elapses, the workflow resumes from the sleep step.

**Example:**

```rust
ctx.sleep(Duration::from_secs(30)).await?;
// Execution continues after 30 seconds
```

### RetryPolicy

Controls retry behavior for failed steps.

```rust
#[derive(Clone, Default)]
pub struct RetryPolicy {
    pub maximum_attempts: Option<i32>,
    pub initial_interval: Option<Duration>,
    pub backoff_coefficient: Option<f64>,
    pub maximum_interval: Option<Duration>,
    pub non_retryable_errors: Vec<String>,
}
```

**Fields:**

- `maximum_attempts`: Maximum number of retry attempts (0 = unlimited)
- `initial_interval`: Initial delay before first retry
- `backoff_coefficient`: Multiplier for exponential backoff (e.g., 2.0 doubles each time)
- `maximum_interval`: Maximum delay between retries
- `non_retryable_errors`: Error codes that should not be retried

**Example:**

```rust
let policy = RetryPolicy {
    maximum_attempts: Some(5),
    initial_interval: Some(Duration::from_secs(1)),
    backoff_coefficient: Some(2.0),
    maximum_interval: Some(Duration::from_secs(60)),
    non_retryable_errors: vec![
        "InvalidArgument".to_string(),
        "PreconditionFailed".to_string(),
    ],
};
```

### KagziError

Rich error type that communicates retry semantics to the server.

```rust
#[derive(Debug, Clone)]
pub struct KagziError {
    pub code: ErrorCode,
    pub message: String,
    pub non_retryable: bool,
    pub retry_after: Option<Duration>,
    pub subject: Option<String>,
    pub subject_id: Option<String>,
    pub metadata: HashMap<String, String>,
}
```

**Constructors:**

```rust
// Generic error with code
KagziError::new(code, message)

// Mark as non-retryable
KagziError::non_retryable(message)

// Retry with delay
KagziError::retry_after(message, Duration::from_secs(60))
```

**Example:**

```rust
return Err(KagziError::non_retryable("Invalid order amount"));

return Err(KagziError::retry_after(
    "Service temporarily unavailable",
    Duration::from_secs(30),
));
```

### WorkflowPaused

Marker error returned when a workflow is paused (sleep or scheduled retry).

```rust
pub struct WorkflowPaused;
```

You typically don't need to construct this manually. It's returned by `WorkflowContext::sleep()` and when a step is scheduled for retry.

### ErrorCode

Available error codes (from `kagzi_proto::kagzi::ErrorCode`):

- `Internal` - Internal server error
- `InvalidArgument` - Invalid input (non-retryable)
- `NotFound` - Resource not found
- `PreconditionFailed` - Precondition violated (non-retryable)
- `Conflict` - Resource conflict (non-retryable)
- `Unauthorized` - Authorization failed (non-retryable)
- `Unavailable` - Service temporarily unavailable (retryable)

## Configuration

### Environment Variables

The client library uses standard Rust environment variables for gRPC:

- `KAGZI_SERVER_ADDRESS`: Default server address
- `RUST_LOG`: Logging level (e.g., `info`, `debug`, `kagzi=trace`)

### Tracing Setup

```rust
use tracing_subscriber::{EnvFilter, fmt};

fn init_tracing() {
    let filter = EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
```

## Error Handling

### Best Practices

1. **Use specific error codes:** Choose appropriate `ErrorCode` values to communicate retry semantics
2. **Mark non-retryable errors:** Use `KagziError::non_retryable()` for validation failures
3. **Provide context:** Include meaningful messages and metadata in errors
4. **Handle `WorkflowPaused`:** This is expected for sleep operations, not an actual error

### Example Error Handling

```rust
async fn process_payment(
    ctx: WorkflowContext,
    input: PaymentInput,
) -> anyhow::Result<PaymentOutput> {
    let result = ctx.run_with_input("charge-card", &input, async move {
        match charge_card(&input).await {
            Ok(output) => Ok(output),
            Err(PaymentError::InvalidCard) => {
                // Non-retryable - user error
                Err(KagziError::non_retryable("Invalid card details"))
            }
            Err(PaymentError::InsufficientFunds) => {
                // Non-retryable - user error
                Err(KagziError::non_retryable("Insufficient funds"))
            }
            Err(PaymentError::GatewayTimeout) => {
                // Retryable with delay
                Err(KagziError::retry_after(
                    "Payment gateway timeout",
                    Duration::from_secs(30),
                ))
            }
            Err(e) => {
                // Unknown error - let default retry policy decide
                Err(anyhow::anyhow!(e))
            }
        }
    }).await?;

    Ok(result)
}
```

## Retry Policies

### Worker-Level Defaults

Set default retry policy for all steps:

```rust
let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
    .default_step_retry(RetryPolicy {
        maximum_attempts: Some(3),
        initial_interval: Some(Duration::from_secs(1)),
        backoff_coefficient: Some(2.0),
        maximum_interval: Some(Duration::from_secs(60)),
        non_retryable_errors: vec![
            "InvalidArgument".to_string(),
            "PreconditionFailed".to_string(),
        ],
    })
    .build()
    .await?;
```

### Step-Level Overrides

Override retry policy for specific steps:

```rust
let result = ctx
    .run_with_input_with_retry(
        "critical-operation",
        &input,
        Some(RetryPolicy {
            maximum_attempts: Some(10),
            initial_interval: Some(Duration::from_millis(500)),
            backoff_coefficient: Some(1.5),
            maximum_interval: Some(Duration::from_secs(30)),
            non_retryable_errors: vec![],
        }),
        async { critical_operation(input).await },
    )
    .await?;
```

### Workflow-Level Retry

Set retry policy when starting a workflow:

```rust
let run_id = client
    .workflow("my-workflow", "queue", input)
    .retry_policy(RetryPolicy {
        maximum_attempts: Some(5),
        initial_interval: Some(Duration::from_secs(2)),
        backoff_coefficient: Some(2.0),
        maximum_interval: Some(Duration::from_secs(300)),
        non_retryable_errors: vec![],
    })
    .await?;
```

## Tracing

Kagzi automatically propagates correlation and trace IDs through the workflow execution. These are available in structured logs.

### Viewing Traced Workflows

Logs include correlation and trace IDs:

```
2025-12-28T10:30:45.123Z INFO process_order:run correlation_id=0190abc1-2345-6789-abcd-ef0123456789 trace_id=0190def2-3456-7890-bcde-f01234567890 run_id=run-123 step_id=validate-order Executing step
```

### Custom Tracing

You can add custom spans and events:

```rust
use tracing::{info_span, instrument};

async fn my_workflow(
    mut ctx: WorkflowContext,
    input: MyInput,
) -> anyhow::Result<MyOutput> {
    let span = info_span!("my_workflow", order_id = %input.order_id);
    let _enter = span.enter();

    let result = ctx.run("process", async move {
        info!("Processing order");
        Ok(MyOutput {})
    }).await?;

    Ok(result)
}
```

## Best Practices

### 1. Idempotent Steps

Design steps to be idempotent since they may be executed multiple times:

```rust
// Bad: Non-idempotent counter increment
ctx.run("increment", async {
    increment_counter().await // Will double-count on retry
}).await?;

// Good: Use upserts or idempotent operations
ctx.run("upsert-record", async {
    upsert_record_with_id(id, data).await // Safe on retry
}).await?;
```

### 2. Small, Focused Steps

Keep steps small and focused:

```rust
// Good: Separate steps
let user = ctx.run("fetch-user", async { fetch_user(id).await }).await?;
let orders = ctx.run("fetch-orders", async { fetch_orders(id).await }).await?;
let total = ctx.run("calculate-total", async { calculate_total(&orders).await }).await?;

// Avoid: Large monolithic steps
let result = ctx.run("do-everything", async {
    let user = fetch_user(id).await?;
    let orders = fetch_orders(id).await?;
    let total = calculate_total(&orders)?;
    Ok((user, orders, total))
}).await?;
```

### 3. Proper Error Classification

Classify errors appropriately for retry behavior:

```rust
match operation().await {
    Ok(result) => Ok(result),
    Err(Error::Validation(msg)) => Err(KagziError::non_retryable(msg)),
    Err(Error::NotFound) => Err(KagziError::new(ErrorCode::NotFound, msg)),
    Err(Error::Timeout) => Err(KagziError::retry_after(msg, Duration::from_secs(30))),
    Err(e) => Err(anyhow::anyhow!(e)), // Let default policy handle
}
```

### 4. Timeout Management

Use async timeouts instead of blocking:

```rust
use tokio::time::{timeout, Duration};

let result = ctx.run("api-call", async move {
    timeout(Duration::from_secs(10), api_call())
        .await
        .map_err(|_| KagziError::new(ErrorCode::Unavailable, "API timeout"))?
}).await?;
```

### 5. Worker Configuration

Configure workers based on workload:

```rust
// I/O-bound tasks (more concurrency)
let mut worker = WorkerBuilder::new("http://localhost:50051", "api-calls")
    .max_concurrent(200)
    .queue_concurrency_limit(500)
    .build()
    .await?;

// CPU-bound tasks (less concurrency)
let mut worker = WorkerBuilder::new("http://localhost:50051", "processing")
    .max_concurrent(4)
    .queue_concurrency_limit(10)
    .build()
    .await?;
```

### 6. Graceful Shutdown

Handle shutdown signals properly:

```rust
use tokio::signal;

async fn run_worker() -> anyhow::Result<()> {
    let mut worker = Worker::new("http://localhost:50051", "queue").await?;
    worker.register("workflow", my_workflow);

    let shutdown_token = worker.shutdown_token();

    tokio::select! {
        result = worker.run() => result,
        _ = signal::ctrl_c() => {
            println!("Received Ctrl+C, shutting down...");
            worker.shutdown();
            Ok(())
        }
    }
}
```

## Common Patterns

### Fan-Out/Fan-In

Process multiple items in parallel:

```rust
async fn process_batch(
    mut ctx: WorkflowContext,
    input: BatchInput,
) -> anyhow::Result<BatchOutput> {
    let mut tasks = Vec::new();

    for item in input.items {
        let item = item.clone();
        tasks.push(ctx.run_with_input("process-item", &item, async move {
            process_single_item(item).await
        }));
    }

    // Execute in parallel
    let results = futures::future::try_join_all(tasks).await?;

    Ok(BatchOutput { results })
}
```

### Compensation (Saga Pattern)

Implement compensating actions:

```rust
async fn transaction_workflow(
    mut ctx: WorkflowContext,
    input: TransactionInput,
) -> anyhow::Result<()> {
    // Step 1: Reserve inventory
    let reservation = ctx
        .run("reserve-inventory", async {
            reserve_inventory(&input).await
        })
        .await?;

    // Step 2: Process payment
    let payment_result = ctx
        .run("process-payment", async {
            process_payment(&input).await
        })
        .await;

    match payment_result {
        Ok(_) => Ok(()),
        Err(e) => {
            // Compensate: release inventory
            ctx.run_with_input(
                "release-inventory",
                &reservation,
                async {
                    release_inventory(&reservation).await
                },
            ).await?;
            Err(e)
        }
    }
}
```

### Conditional Execution

Skip steps based on conditions:

```rust
async fn conditional_workflow(
    mut ctx: WorkflowContext,
    input: WorkflowInput,
) -> anyhow::Result<Output> {
    let validated = ctx.run("validate", async {
        validate(&input).await
    }).await?;

    if validated.needs_approval {
        let approval = ctx.run("await-approval", async {
            wait_for_approval(&validated).await
        }).await?;

        if !approval.approved {
            return Err(KagziError::non_retryable("Not approved"));
        }
    }

    let result = ctx.run("execute", async {
        execute(&validated).await
    }).await?;

    Ok(result)
}
```

### Long-Running Workflows

Handle workflows that take hours or days:

```rust
async fn long_running_workflow(
    mut ctx: WorkflowContext,
    input: Input,
) -> anyhow::Result<Output> {
    // Step 1: Start process
    let process_id = ctx.run("start", async {
        start_long_process(input).await
    }).await?;

    // Step 2: Wait for completion (check every hour)
    loop {
        ctx.sleep(Duration::from_secs(3600)).await?;

        let status = ctx.run_with_input(
            "check-status",
            &process_id,
            async { check_process_status(process_id).await },
        ).await?;

        if status.is_complete {
            break;
        }
    }

    // Step 3: Get final result
    let result = ctx.run_with_input(
        "get-result",
        &process_id,
        async { get_process_result(process_id).await },
    ).await?;

    Ok(result)
}
```

## Testing

### Unit Testing Workflows

Test workflow logic without a server:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_order_validation() {
        // Test validation logic directly
        let result = validate_amount(-10.0);
        assert!(result.is_err());
    }
}
```

### Integration Testing

Use `just test-integration` to test with a running server:

```bash
# Start server
just dev

# Run integration tests
KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests -- --test-threads=1
```

## Troubleshooting

### Workflow Not Picked Up

Check:

1. Worker registered the workflow type
2. Task queue matches
3. Namespace matches
4. Worker is running and not maxed out on concurrency

### Steps Repeated Unexpectedly

This is normal for idempotency. Ensure steps are idempotent:

- Use upserts instead of inserts
- Check existence before creating
- Use idempotency keys for external APIs

### Workflow Paused Unexpectedly

Check:

1. Did you call `ctx.sleep()`?
2. Did a step return a retryable error?
3. Check logs for `WorkflowPaused` messages

### Worker Not Responding

Check:

1. Server is reachable
2. Heartbeats are being sent (check logs)
3. Network connectivity
4. Server logs for errors

## Performance Considerations

### Concurrency Tuning

- **I/O-bound workflows**: Higher concurrency (100-500)
- **CPU-bound workflows**: Lower concurrency (2-8)
- **Queue limits**: Prevent overwhelming downstream services

### Step Size

- **Small steps**: Better visibility, easier retries, more network round trips
- **Large steps**: Fewer round trips, harder to reason about failures
- **Guideline**: Aim for 1-10 seconds per step

### Payload Size

- Keep payloads under 1MB for optimal performance
- Large payloads can be stored externally with references

### Worker Scaling

- Multiple workers can share the same task queue
- Use queue_concurrency_limit to distribute load
- Different queues for different priority levels

## License

Licensed under the same terms as the Kagzi project. See LICENSE file for details.
