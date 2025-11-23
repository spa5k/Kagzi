# Kagzi

A durable workflow engine for Rust, inspired by Temporal, Inngest, and OpenWorkflow.

Kagzi allows you to build resilient, long-running workflows that can survive crashes, restarts, and even months of sleep - all backed by PostgreSQL.

## Features

### Core Features
- ✅ **Durable Execution**: Workflows survive crashes and restarts
- ✅ **Step Memoization**: Completed steps are cached and never re-executed
- ✅ **Durable Sleep**: Sleep for seconds or months without holding resources
- ✅ **Type Safe**: Full Rust type safety with serde serialization
- ✅ **PostgreSQL Backend**: Simple, reliable persistence
- ✅ **Automatic Migrations**: Database schema managed automatically
- ✅ **Worker Pool**: Scale horizontally with multiple workers

### V2 Features (New!)
- ✅ **Retry Policies**: Exponential backoff and fixed interval retries with configurable error handling
- ✅ **Parallel Execution**: Execute multiple workflow steps concurrently with different error strategies
- ✅ **Workflow Versioning**: Manage multiple versions of workflows simultaneously
- ✅ **Production-Ready Workers**: Graceful shutdown, heartbeat monitoring, and concurrent workflow limiting
- ✅ **Health Monitoring**: Worker health checks and system observability
- ✅ **Advanced Error Handling**: Rich error classification and retry predicates

## Quick Start

### Prerequisites

- Rust 1.70+
- PostgreSQL (via Docker Compose provided)

### 1. Start PostgreSQL

Using Docker Compose (recommended):
```bash
docker compose up -d
# or older docker-compose syntax
docker-compose up -d
```

Or install PostgreSQL locally and create a database:
```bash
createdb kagzi
# Update DATABASE_URL in .env to match your setup
```

### 2. Run the Simple Example

```bash
cargo run --example simple
```

This will:
1. Connect to PostgreSQL
2. Run migrations automatically
3. Register a workflow
4. Start a workflow run
5. Execute it with a worker
6. Wait for and print the result

### 3. Try the Welcome Email Example

This demonstrates a more realistic workflow with multiple steps and sleep.

**Terminal 1 - Start Worker:**
```bash
cargo run --example welcome_email -- worker
```

**Terminal 2 - Trigger Workflow:**
```bash
cargo run --example welcome_email -- trigger Alice
```

Watch the worker execute the workflow step by step!

## Usage

### Define a Workflow

```rust
use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct MyInput {
    user_id: String,
}

async fn my_workflow(
    ctx: WorkflowContext,
    input: MyInput,
) -> anyhow::Result<String> {
    // Step 1: Fetch user (memoized)
    let user = ctx.step("fetch-user", async {
        // This only runs once - subsequent calls return cached result
        Ok::<_, anyhow::Error>("user data".to_string())
    }).await?;

    // Step 2: Sleep for an hour (durable - doesn't hold worker)
    ctx.sleep("wait", std::time::Duration::from_secs(3600)).await?;

    // Step 3: Send email (memoized)
    ctx.step("send-email", async {
        // This also only runs once
        Ok::<_, anyhow::Error>(())
    }).await?;

    Ok("completed".to_string())
}
```

### Start Workflows and Workers

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Connect (automatically runs migrations)
    let kagzi = Kagzi::connect("postgres://localhost/kagzi").await?;

    // Register workflows
    kagzi.register_workflow("my-workflow", my_workflow).await;

    // Start a workflow
    let handle = kagzi.start_workflow("my-workflow", MyInput {
        user_id: "user_123".to_string(),
    }).await?;

    println!("Started workflow: {}", handle.run_id());

    // Start a worker (in a separate process or thread)
    let worker = kagzi.create_worker();
    worker.start().await?;

    Ok(())
}
```

## V2 Features Guide

### Retry Policies

Configure automatic retries for workflow steps with exponential backoff or fixed intervals:

```rust
use kagzi::{StepBuilder, RetryPolicy, RetryPredicate};

// Exponential backoff with jitter
StepBuilder::new("api-call")
    .retry_policy(RetryPolicy::exponential_with(
        1000,    // 1s initial delay
        30000,   // 30s max delay
        2.0,     // Double each retry
        5,       // Max 5 attempts
        true,    // Use jitter
    ))
    .retry_predicate(RetryPredicate::OnRetryableError)
    .execute(&ctx, async {
        // Your step logic here
        Ok::<_, anyhow::Error>(())
    })
    .await?;

// Fixed interval retries
StepBuilder::new("db-operation")
    .retry_policy(RetryPolicy::fixed_with(2000, 3)) // 2s delay, 3 attempts
    .execute(&ctx, async { Ok::<_, anyhow::Error>(()) })
    .await?;
```

See `examples/step_retry_policies.rs` for a complete example.

### Parallel Execution

Execute multiple workflow steps concurrently with memoization support:

```rust
use kagzi::{ParallelExecutor, ParallelErrorStrategy};
use uuid::Uuid;

// Create a parallel executor
let parallel_group = Uuid::new_v4();
let executor = ParallelExecutor::new(
    &ctx,
    parallel_group,
    None,
    ParallelErrorStrategy::CollectAll, // Or FailFast
);

// Execute steps in parallel
let handles = vec![
    tokio::spawn(executor.execute_step("step-1", async { /* ... */ })),
    tokio::spawn(executor.execute_step("step-2", async { /* ... */ })),
    tokio::spawn(executor.execute_step("step-3", async { /* ... */ })),
];

let results = futures::future::join_all(handles).await;
```

See `examples/advanced_parallel.rs` and `examples/parallel_execution.rs` for complete examples.

### Workflow Versioning

Manage multiple versions of workflows and roll out changes gradually:

```rust
// Register multiple versions
kagzi.register_workflow_version("process-order", 1, process_order_v1).await?;
kagzi.register_workflow_version("process-order", 2, process_order_v2).await?;
kagzi.register_workflow_version("process-order", 3, process_order_v3).await?;

// Set default version for new workflows
kagzi.set_default_workflow_version("process-order", 3).await?;

// Start a workflow with a specific version
let handle = kagzi.start_workflow_version("process-order", 2, input).await?;

// Or use the default version
let handle = kagzi.start_workflow("process-order", input).await?;
```

See `examples/workflow_versioning.rs` for a complete example.

### Production-Ready Workers

Configure workers with custom settings for production deployments:

```rust
use kagzi::WorkerBuilder;

let worker = kagzi
    .create_worker_builder()
    .worker_id("my-worker-1".to_string())
    .max_concurrent_workflows(10)     // Process up to 10 workflows concurrently
    .heartbeat_interval_secs(30)      // Send heartbeat every 30 seconds
    .poll_interval_ms(500)            // Poll for new workflows every 500ms
    .graceful_shutdown_timeout_secs(300) // Wait up to 5 minutes on shutdown
    .build();

worker.start().await?;
```

Workers automatically handle:
- Graceful shutdown (SIGTERM/SIGINT)
- Heartbeat monitoring
- Concurrent workflow limiting
- Automatic worker registration

See `examples/worker_management.rs` for a complete example.

### Health Monitoring

Monitor worker health and system status:

```rust
use kagzi::{HealthChecker, HealthCheckConfig};

let config = HealthCheckConfig {
    heartbeat_timeout_secs: 120,  // Unhealthy after 2 minutes
    heartbeat_warning_secs: 60,   // Warn after 1 minute
};

let health_checker = HealthChecker::with_config(kagzi.db_handle(), config);

// Check database health
health_checker.check_database_health().await?;

// Check worker health
let workers = kagzi_core::queries::get_active_workers(kagzi.db_handle().pool()).await?;
for worker in workers {
    let health = health_checker.check_worker_health(&worker, 0).await;
    println!("Worker {} status: {}", worker.worker_name, health.status);
}

// Comprehensive health check
let result = health_checker.comprehensive_check().await;
if result.is_healthy() {
    println!("System is healthy!");
}
```

See `examples/health_monitoring.rs` for a complete example.

## Architecture

```
┌─────────────────────────────────────────────────┐
│         Your Application                        │
│  - Define workflows                             │
│  - Start workflow runs                          │
│  - Query status                                 │
└────────────┬────────────────────────────────────┘
             │ uses
             ▼
┌─────────────────────────────────────────────────┐
│           Kagzi SDK (Rust)                      │
│  - WorkflowContext & StepBuilder                │
│  - Step memoization & retry policies            │
│  - Parallel execution                           │
│  - Workflow versioning                          │
│  - Client & Worker (with health monitoring)     │
└────────────┬────────────────────────────────────┘
             │ talks to
             ▼
┌─────────────────────────────────────────────────┐
│          PostgreSQL                             │
│  - workflow_runs                                │
│  - step_runs (memoization + parallel tracking)  │
│  - worker_leases (coordination)                 │
│  - workers (heartbeat + health)                 │
└─────────────────────────────────────────────────┘
```

## How It Works

1. **Workflow Definition**: You define workflows as async Rust functions
2. **Workflow Start**: Client creates a row in `workflow_runs` table with status `PENDING`
3. **Worker Polling**: Workers poll for pending workflows using `FOR UPDATE SKIP LOCKED`
4. **Step Execution**: Each step checks the database for cached results before executing
5. **Durable Sleep**: Sleep updates status to `SLEEPING` and releases worker
6. **Resume**: Workers pick up sleeping workflows after their sleep time expires
7. **Completion**: Final output is stored and status updated to `COMPLETED`

## Project Structure

```
Kagzi/
├── crates/
│   ├── kagzi-core/     # Database models, queries, migrations
│   ├── kagzi/          # User-facing SDK
│   └── kagzi-cli/      # CLI tool (optional)
├── examples/           # Example workflows
├── migrations/         # SQL migrations
└── docker-compose.yml  # PostgreSQL setup
```

## Examples

Kagzi includes comprehensive examples demonstrating all features:

### Basic Examples
- `simple.rs` - Minimal workflow example
- `welcome_email.rs` - Multi-step workflow with sleep
- `data_pipeline.rs` - Data processing workflow
- `scheduled_reminder.rs` - Scheduled task example

### V2 Feature Examples
- `retry_pattern.rs` - Retry patterns and error handling
- `step_retry_policies.rs` - Step-level retry configuration
- `parallel_execution.rs` - Basic parallel execution
- `advanced_parallel.rs` - Advanced parallel patterns with error strategies
- `workflow_versioning.rs` - Managing multiple workflow versions
- `worker_management.rs` - Production worker configuration
- `health_monitoring.rs` - Worker health and system monitoring
- `batch_processing.rs` - Batch processing with parallel steps
- `order_fulfillment.rs` - Complex order processing workflow

Run any example with:
```bash
cargo run --example <example_name>
```

## Roadmap

### Phase 1 ✅
- [x] PostgreSQL backend
- [x] Basic workflow execution
- [x] Step memoization
- [x] Durable sleep
- [x] Worker polling
- [x] Automatic migrations

### Phase 2 (V2) ✅
- [x] Retry policies and exponential backoff
- [x] Parallel step execution
- [x] Workflow versioning
- [x] Production-ready workers
- [x] Health monitoring
- [x] Advanced error handling

### Phase 3
- [ ] Observability and metrics (Prometheus/OpenTelemetry)
- [ ] Web dashboard for monitoring
- [ ] SQLite backend
- [ ] Redis backend
- [ ] Workflow cancellation and timeouts
- [ ] Child workflows and signals

### Phase 4
- [ ] gRPC server for multi-language support
- [ ] Python SDK
- [ ] TypeScript SDK
- [ ] Go SDK

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT
