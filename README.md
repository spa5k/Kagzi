# Kagzi

**Reliable workflow orchestration built for simplicity.**

Kagzi is a workflow engine that enables you to write durable, fault-tolerant workflows in Rust. Your workflows survive process crashes, server restarts, and code deploys through deterministic replay and PostgreSQL-backed state management.

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org)

## Why Kagzi?

- **🎯 Simple**: Single server binary, PostgreSQL only dependency
- **🔄 Durable**: Workflows survive crashes through step-level checkpointing
- **⚡ Fast**: Event-driven work distribution via PostgreSQL LISTEN/NOTIFY
- **🛠️ Ergonomic**: Rust SDK with type-safe workflow definitions
- **📦 Batteries Included**: Scheduling, retries, timeouts, and sleep built-in

## Quick Start

### Prerequisites

- Rust 1.80+ (edition 2024)
- PostgreSQL 14+
- Docker (optional, for quick setup)

### 1. Start PostgreSQL

```bash
docker-compose up -d
```

Or use your existing PostgreSQL instance and set the connection string:

```bash
export KAGZI_DB_URL="postgresql://postgres:postgres@localhost:5432/kagzi"
```

### 2. Run Migrations

```bash
cargo install sqlx-cli --no-default-features --features postgres
sqlx migrate run
```

### 3. Start the Server

```bash
cargo run -p kagzi-server
```

The server will start on `0.0.0.0:50051` by default.

### 4. Run Your First Workflow

```rust
use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct HelloInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelloOutput {
    message: String,
}

async fn hello_workflow(
    _ctx: Context,
    input: HelloInput,
) -> anyhow::Result<HelloOutput> {
    Ok(HelloOutput {
        message: format!("Hello, {}!", input.name),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Start a worker
    let mut worker = Worker::new("http://localhost:50051")
        .namespace("default")
        .workflows([("hello_workflow", hello_workflow)])
        .build()
        .await?;

    tokio::spawn(async move {
        worker.run().await.unwrap();
    });

    // Create a client and start the workflow
    let client = Kagzi::connect("http://localhost:50051").await?;
    let run = client
        .start("hello_workflow")
        .namespace("default")
        .input(HelloInput {
            name: "World".to_string(),
        })
        .send()
        .await?;

    println!("Started workflow: {}", run.id);

    Ok(())
}
```

See [examples/](examples/) for more patterns including error handling, scheduling, fan-out/fan-in, and saga patterns.

## Core Concepts

### Workflows

Workflows are durable functions that orchestrate multiple steps. They're deterministic and resumable—if a worker crashes, another worker can pick up where it left off.

```rust
async fn order_workflow(mut ctx: Context, input: OrderInput) -> anyhow::Result<OrderOutput> {
    // Each step is checkpointed to the database
    let payment = ctx.step("process-payment")
        .run(|| process_payment(input.payment_details))
        .await?;

    let inventory = ctx.step("reserve-inventory")
        .run(|| reserve_inventory(input.items))
        .await?;

    // Sleep for durable delays
    ctx.sleep("await-fulfillment", "24h").await?;

    let shipment = ctx.step("ship-order")
        .run(|| ship_order(inventory))
        .await?;

    Ok(OrderOutput { shipment })
}
```

### Steps

Steps are the checkpoints within a workflow. When a step completes, its output is stored. On replay (e.g., after a crash), completed steps return cached outputs without re-execution.

### Workers

Workers are long-running processes that poll for workflows and execute them. Register your workflow definitions with a worker:

```rust
let worker = Worker::new("http://localhost:50051")
    .namespace("production")
    .task_queue("orders")
    .workflows([
        ("order_workflow", order_workflow),
        ("refund_workflow", refund_workflow),
    ])
    .build()
    .await?;

worker.run().await?;
```

### Scheduling

Create cron-based schedules for recurring workflows:

```rust
use kagzi::Kagzi;

let client = Kagzi::connect("http://localhost:50051").await?;

client
    .create_schedule("daily-report")
    .namespace("analytics")
    .workflow("generate_report")
    .cron("0 9 * * *") // Every day at 9 AM
    .send()
    .await?;
```

## Features

### Durability

- **Step-level checkpointing**: Each step's output is persisted
- **Deterministic replay**: Workflows always execute from the beginning, using cached step outputs
- **Crash recovery**: Workers can fail at any time—workflows resume on other workers

### Scheduling

- **Cron expressions**: Standard cron syntax for recurring workflows
- **Durable sleep**: Pause workflows for hours, days, or weeks with `ctx.sleep()`
- **Backfill control**: Limit catchup runs for schedules that were offline

### Error Handling

- **Automatic retries**: Configure retry policies with exponential backoff
- **Per-step retries**: Override retry behavior at the step level
- **Error propagation**: Failed steps bubble up, triggering workflow retries

### Observability

- **Workflow history**: Query execution history, step outputs, and errors
- **Worker registration**: Track active workers and their health
- **OpenTelemetry support**: Distributed tracing integration

### Multi-tenancy

- **Namespaces**: Isolate workflows by environment or tenant
- **Task queues**: Route workflows to specific worker pools
- **Worker discovery**: Query available workers and workflow types by namespace

## Architecture

Kagzi follows a worker-driven architecture where PostgreSQL serves as the single source of truth:

```
┌──────────────┐      gRPC       ┌──────────────┐
│ Application  │ ───────────────▶ │ Kagzi Server │
│  (Client)    │                  └──────┬───────┘
└──────────────┘                         │
                                         ▼
                              ┌──────────────────┐
                              │   PostgreSQL     │
                              │  • workflow_runs │
                              │  • step_runs     │
                              │  • workers       │
                              └────────┬─────────┘
                                       │ LISTEN/NOTIFY
                                       ▼
                              ┌──────────────────┐
                              │ Workers (N)      │
                              │ • Poll for work  │
                              │ • Execute steps  │
                              │ • Send heartbeat │
                              └──────────────────┘
```

For detailed architecture documentation, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Use Cases

- **Data Pipelines**: Orchestrate multi-step ETL processes with retries and error handling
- **Saga Patterns**: Implement distributed transactions with compensating actions
- **Background Jobs**: Process tasks that need reliability and observability
- **Scheduled Tasks**: Run recurring jobs with cron-like scheduling
- **Microservice Orchestration**: Coordinate calls across multiple services with fault tolerance
- **Long-Running Processes**: Handle workflows that span hours or days with durable sleep

## Comparison

### vs. Temporal

Kagzi prioritizes **operational simplicity** over horizontal scalability:

- **Single binary**: No separate frontend, history, or matching services
- **PostgreSQL only**: No additional datastores required
- **Simpler mental model**: Workers poll directly from the database
- **Rust-first**: Designed for Rust applications (though gRPC enables polyglot clients)

Temporal excels at massive scale and multi-language support. Kagzi excels at getting started quickly and keeping operations simple.

### vs. AWS Step Functions

- **Self-hosted**: Run on your own infrastructure
- **No vendor lock-in**: Open source, Apache 2.0
- **Richer SDK**: Native Rust workflows vs. JSON state machines
- **Lower latency**: Direct database access vs. API calls

## Examples

The [examples/](examples/) directory contains workflow patterns:

- **01_basics** - Hello world and step chaining
- **02_error_handling** - Retry policies and error propagation
- **03_scheduling** - Cron-based recurring workflows
- **04_concurrency** - Parallel step execution
- **05_fan_out_in** - Fan-out/fan-in patterns
- **06_long_running** - Workflows with durable sleep
- **07_idempotency** - Preventing duplicate workflow creation
- **08_saga_pattern** - Distributed transactions with compensation
- **09_data_pipeline** - Multi-stage data processing
- **10_multi_queue** - Worker pools and task routing

Run any example:

```bash
cargo run --example 01_basics
```

## Configuration

Configure the server via environment variables:

```bash
# Database
export KAGZI_DB_URL="postgresql://user:pass@localhost:5432/kagzi"

# Server
export KAGZI_SERVER_HOST="0.0.0.0"
export KAGZI_SERVER_PORT="50051"
export KAGZI_SERVER_DB_MAX_CONNECTIONS="50"

# Coordinator
export KAGZI_COORDINATOR_INTERVAL_SECS="5"
export KAGZI_COORDINATOR_BATCH_SIZE="100"

# Worker defaults
export KAGZI_WORKER_POLL_TIMEOUT_SECS="60"
export KAGZI_WORKER_HEARTBEAT_INTERVAL_SECS="10"
export KAGZI_WORKER_VISIBILITY_TIMEOUT_SECS="60"

# Telemetry
export KAGZI_TELEMETRY_ENABLED="false"
export KAGZI_TELEMETRY_LOG_LEVEL="info"
```

See [crates/kagzi-server/src/config.rs](crates/kagzi-server/src/config.rs) for all options.

## Development

### Prerequisites

- Rust 1.80+ with edition 2024
- PostgreSQL 14+
- Protocol Buffers compiler (for proto generation)

### Build

```bash
cargo build --workspace
```

### Run Tests

```bash
# Start PostgreSQL
docker-compose up -d

# Run tests
cargo test --workspace
```

### Generate Proto Files

```bash
cd proto
./generate.sh
```

## Contributing

We welcome contributions! Please see:

- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community guidelines
- [ARCHITECTURE.md](ARCHITECTURE.md) for technical documentation
- [CHANGELOG.md](CHANGELOG.md) for recent changes

### Reporting Issues

Found a bug or have a feature request? Please [open an issue](https://github.com/spa5k/kagzi/issues) with:

- Clear description of the problem or feature
- Steps to reproduce (for bugs)
- Expected vs. actual behavior
- Environment details (OS, Rust version, PostgreSQL version)

## Roadmap

- [ ] Python SDK
- [ ] TypeScript SDK
- [ ] Web UI for workflow visualization
- [ ] Workflow versioning
- [ ] Parallel execution optimization
- [ ] Enhanced metrics and dashboards
- [ ] Workflow cancellation propagation to child workflows

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

## Acknowledgments

Kagzi draws inspiration from:

- [Temporal](https://temporal.io) - Durable execution model
- [Hatchet](https://hatchet.run) - PostgreSQL-backed task queue
- [AWS Step Functions](https://aws.amazon.com/step-functions/) - Workflow orchestration patterns

---

**Built with ❤️ in Rust**
