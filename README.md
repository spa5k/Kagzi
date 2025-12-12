# Kagzi

<div align="center">

![Kagzi Logo](https://img.shields.io/badge/Kagzi-Workflow%20Engine-blue?style=for-the-badge)
![Rust](https://img.shields.io/badge/rust-1.75+-orange?style=for-the-badge&logo=rust)
![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=for-the-badge)
![Status](https://img.shields.io/badge/status-Alpha-orange?style=for-the-badge)

_A modern, distributed workflow engine built in Rust with gRPC_

</div>

## Overview

Kagzi is a high-performance, distributed workflow engine designed for building reliable, scalable asynchronous systems. Built with Rust and gRPC, it provides a robust foundation for orchestrating complex business processes with built-in fault tolerance, observability, and enterprise-grade features.

> [!WARNING]  
> Kagzi is currently in alpha stage and not recommended for production use. APIs may change.

### Key Features

- **High Performance**: Built with Rust 2024 edition and Tokio for maximum throughput
- **Distributed Architecture**: Scale horizontally with database-backed work distribution
- **Fault Tolerant**: Automatic retries, error handling, and workflow recovery
- **Observability**: Comprehensive tracing, correlation IDs, and structured logging
- **Type Safe**: Full Rust type safety with compile-time guarantees
- **Async First**: Native async/await support throughout
- **gRPC Protocol**: High-performance, language-agnostic communication
- **PostgreSQL Backend**: Reliable, ACID-compliant persistence with connection pooling
- **Composable**: Build complex workflows from simple, reusable steps
- **Advanced Scheduling**: Cron-based workflow scheduling with catchup support
- **Saga Pattern**: Built-in support for distributed transactions with compensation
- **Concurrency Control**: Per-queue and per-workflow type concurrency limits
- **Idempotency**: External IDs for idempotent workflow executions

## Architecture

```
┌─────────────────┐      gRPC      ┌─────────────────┐      SQL      ┌─────────────────┐
│   Clients       │ ◄─────────────► │  Kagzi Server   │ ◄─────────────► │  PostgreSQL     │
│                 │                │                 │                │                 │
│ • Web Services  │                │ • Scheduler     │                │ • Workflows     │
│ • CLI Tools     │                │ • Queue Manager │                │ • Steps         │
│ • External APIs │                │ • Workflow      │                │ • Queue         │
│                 │                │   Engine        │                │ • Schedules     │
└─────────────────┘                │ • Watchdog      │                │ • Workers       │
                                   └─────────────────┘                └─────────────────┘
                                             ▲
                                             │ gRPC
                                             ▼
                                   ┌─────────────────┐
                                   │  Worker Nodes   │
                                   │                 │
                                   │ • Execute Steps │
                                   │ • Heartbeats    │
                                   │ • Retry Logic   │
                                   │                 │
                                   │ (Multiple)      │
                                   └─────────────────┘
```

## Quick Start

### Prerequisites

- Rust 1.75+ with Cargo (2024 edition)
- PostgreSQL 14+ (or Docker)
- Just (task runner) - optional but recommended

### Installation

1. **Clone the repository**

   ```bash
   git clone https://github.com/spa5k/kagzi.git
   cd kagzi
   ```

2. **Set up the development environment**

   ```bash
   # Using Just (recommended)
   just setup

   # Or manually:
   docker-compose up -d
   cargo run -p kagzi-server
   ```

3. **Run your first workflow**
   ```bash
   cargo run -p kagzi --example simple
   ```

### Basic Usage

Here's a simple workflow to get you started:

```rust
use kagzi::{Client, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Input {
    name: String,
}

#[derive(Serialize, Deserialize)]
struct Output {
    message: String,
}

// Define step functions as separate async functions
async fn greet(name: String) -> anyhow::Result<String> {
    Ok(format!("Hello, {}!", name))
}

// Workflow orchestrates steps
async fn greet_workflow(mut ctx: WorkflowContext, input: Input) -> anyhow::Result<Output> {
    // Run step - if replayed, returns cached result
    let greeting = ctx.run("greet", greet(input.name)).await?;

    Ok(Output { message: greeting })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Start worker
    let mut worker = Worker::builder("http://localhost:50051", "default")
        .build()
        .await?;
    worker.register("greet", greet_workflow);
    tokio::spawn(async move { worker.run().await });

    // Start workflow
    let mut client = Client::connect("http://localhost:50051").await?;
    let run_id = client.workflow("greet", "default", Input {
        name: "World".to_string(),
    }).await?;

    println!("Started workflow: {}", run_id);
    Ok(())
}
```

## Documentation

### Core Concepts

#### Workflows

Workflows are the main unit of execution in Kagzi. They consist of multiple steps that can be executed sequentially, in parallel, or with complex dependencies.

#### Steps

Steps are individual units of work within a workflow. Each step can:

- Execute async functions
- Call other workflows (sub-workflows)
- Sleep for specified durations
- Handle failures and retries

#### Workers

Workers are processes that execute workflow steps. They poll the server for available work and execute steps using registered workflow functions.

#### Context

The `WorkflowContext` provides access to workflow operations like:

- Running sub-steps
- Sleeping
- Accessing workflow metadata
- Error handling

### API Reference

#### Client API

```rust
let mut client = Client::connect("http://localhost:50051").await?;

// Start a simple workflow
let run_id = client.workflow("workflow_name", "task_queue", input).await?;

// Start with options
let run_id = client
    .workflow("workflow_name", "task_queue", input)
    .id("unique-external-id")
    .version("1.0.0")
    .namespace("custom-namespace")
    .deadline(chrono::Utc::now() + chrono::Duration::hours(1))
    .retry_policy(retry_policy)
    .retries(3)
    .await?;

// Get workflow schedule
let schedule = client.get_workflow_schedule("schedule_id", Some("namespace")).await?;

// List workflow schedules
let schedules = client.list_workflow_schedules("namespace", None).await?;
```

#### Worker API

```rust
let mut worker = Worker::builder("http://localhost:50051", "task_queue")
    // Optional: namespace for the worker
    .namespace("production")
    // Optional: max concurrent workflows for this worker
    .max_concurrent(50)
    // Optional: worker hostname for identification
    .hostname("worker-01.example.com")
    // Optional: worker version
    .version("1.2.3")
    // Optional: custom labels
    .label("region", "us-west")
    .label("env", "prod")
    // Optional: cap total concurrent workflows for this queue
    .queue_concurrency_limit(32)
    // Optional: cap a specific workflow type inside the queue
    .workflow_type_concurrency("payment_capture", 16)
    // Optional: default step-level retry if a step does not provide one
    .default_step_retry(kagzi::RetryPolicy {
        maximum_attempts: Some(3),
        initial_interval: Some(std::time::Duration::from_millis(500)),
        backoff_coefficient: Some(2.0),
        maximum_interval: Some(std::time::Duration::from_secs(10)),
        non_retryable_errors: vec![],
    })
    .build()
    .await?;

// Register workflow
worker.register("workflow_name", workflow_function);

// Start polling for work
worker.run().await?;
```

#### Schedules (Cron)

- Cron expressions follow `cron::Schedule` format in UTC: `sec min hour dom month dow [year]`. Missed ticks are replayed on restart using `(from, to]` bounds, capped by `max_catchup` (default 100).
- Idempotency per fire: `schedule:{schedule_id}:{fire_at_rfc3339}` prevents duplicates across restarts.
- Scheduler config: `KAGZI_SCHEDULER_INTERVAL_SECS` (default 5s), `KAGZI_SCHEDULER_BATCH_SIZE` (default 100), `KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK` (default 1000).
- Disable stops future fires; re-enable catches up from the last fired time within `max_catchup`.

```rust
use kagzi::{Client, ScheduleUpdate};

let mut client = Client::connect("http://localhost:50051").await?;

// Create a cron schedule
let schedule = client
    .schedule("daily_report", "reports", "0 0 6 * * *") // 06:00:00 UTC daily
    .input(serde_json::json!({"region": "us-east"}))
    .version("1.0.0")
    .create()
    .await?;

// Update cron expression
let updated = client
    .update_schedule(
        &schedule.schedule_id,
        "default",
        ScheduleUpdate::default().cron_expr("0 0 7 * * *"),
    )
    .await?;

// Delete the schedule
client.delete_schedule(&schedule.schedule_id, "default").await?;
```

#### Workflow Context API

```rust
async fn workflow_function(mut ctx: WorkflowContext, input: Input) -> anyhow::Result<Output> {
    // Run a step - results are memoized for replay
    let result = ctx.run("step_name", my_async_function(input.value)).await?;

    // Run a step with explicit input tracking (for observability)
    let result2 = ctx.run_with_input("step_name", &input, my_async_function(input.value)).await?;

    // Run a step with an explicit retry policy
    let result_with_retry = ctx
        .run_with_input_with_retry(
            "step_with_retry",
            &input,
            Some(kagzi::RetryPolicy {
                maximum_attempts: Some(5),
                initial_interval: None,
                backoff_coefficient: None,
                maximum_interval: None,
                non_retryable_errors: vec![],
            }),
            my_async_function(input.value),
        )
        .await?;

    // Durable sleep - survives server restarts
    ctx.sleep(Duration::from_secs(10)).await?;

    Ok(Output { result })
}
```

## Examples

Run any example with `cargo run -p kagzi --example <name> -- <variant>`.

- 01_basics: hello world, multi-step chain, context passing (`hello|chain|context`)
- 02_error_handling: retries vs non-retryable, per-step override (`flaky|fatal|override`)
- 03_scheduling: cron create/delete, durable sleep, catchup (`cron|sleep|catchup`)
- 04_concurrency: local cap, shared queue cap, workflow-type limits (`local|queue|workflow`)
- 05_fan_out_in: parallel static fetch + dynamic map-reduce (`static|mapreduce`)
- 06_long_running: polling loop and timeout guard (`poll|timeout`)
- 07_idempotency: external IDs and memoized steps (`external|memo`)
- 08_saga_pattern: trip booking with compensation (`saga|partial`)
- 09_data_pipeline: JSON transform and large-payload offload (`transform|large`)
- 10_multi_queue: priority queues and tenant isolation (`priority|namespace`)

## Configuration

### Environment Variables

| Variable                                 | Description                    | Default                                                  |
| ---------------------------------------- | ------------------------------ | -------------------------------------------------------- |
| `KAGZI_DB_URL`                           | PostgreSQL connection string   | `postgresql://postgres:postgres@localhost:5432/postgres` |
| `KAGZI_SERVER_HOST`                      | Server bind host               | `0.0.0.0`                                                |
| `KAGZI_SERVER_PORT`                      | Server bind port               | `50051`                                                  |
| `KAGZI_DB_MAX_CONNECTIONS`               | Maximum database connections   | `50`                                                     |
| `KAGZI_SCHEDULER_INTERVAL_SECS`          | Scheduler tick interval (secs) | `5`                                                      |
| `KAGZI_SCHEDULER_BATCH_SIZE`             | Scheduler batch size           | `100`                                                    |
| `KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK` | Max workflows per tick         | `1000`                                                   |
| `KAGZI_WATCHDOG_INTERVAL_SECS`           | Watchdog interval (secs)       | `1`                                                      |
| `KAGZI_WORKER_STALE_THRESHOLD_SECS`      | Worker stale threshold (secs)  | `30`                                                     |
| `KAGZI_POLL_TIMEOUT_SECS`                | Worker poll timeout (secs)     | `60`                                                     |
| `KAGZI_HEARTBEAT_INTERVAL_SECS`          | Worker heartbeat interval      | `10`                                                     |
| `RUST_LOG`                               | Log level filter               | `info`                                                   |

### Payload Limits

Payloads over 1MB log a warning; payloads over 2MB are rejected to avoid bloating the database. Store large data externally and pass references instead.

### Database Setup

Using Docker (recommended):

```bash
docker-compose up -d
```

Manual setup:

```bash
# Create database
createdb kagzi

# Run migrations
sqlx migrate run --database-url "postgresql://user:pass@localhost/kagzi"
```

Note: The default Docker Compose setup exposes PostgreSQL on port `54122` to avoid conflicts with local PostgreSQL instances. The justfile is configured to use this port by default.

## Development

### Project Structure

```
kagzi/
├── crates/
│   ├── kagzi/              # Client library
│   ├── kagzi-proto/        # Generated gRPC code
│   ├── kagzi-server/       # Server implementation
│   ├── kagzi-store/        # Database repository layer
│   └── tests/              # Integration and end-to-end tests
├── examples/               # Usage examples (10 comprehensive examples)
├── migrations/             # Database migrations
├── proto/                  # Protocol definitions
├── scripts/                # Helper scripts
├── justfile               # Task runner commands
└── docker-compose.yml     # Development database (port 54122)
```

### Building

```bash
# Build entire workspace
cargo build

# Build specific crate
cargo build -p kagzi-server

# Build with optimizations
cargo build --release
```

### Testing

```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p kagzi

# Run with coverage
cargo tarpaulin --out Html

# Run integration tests (requires Docker)
cargo test -p kagzi-server --test integration_tests

# Run end-to-end tests with testcontainers
cargo test -p tests
```

### Development Commands (using Just)

```bash
# Start development environment
just setup

# Start database
just db-up

# Run server
just dev

# Run migrations
just migrate

# Lint code
just lint

# Launch gRPC UI
just grpcui

# Run specific examples
just example hello
just example chain "variant args"

# Run all examples
just examples-all

# Run tests
just test
just test-unit
just test-integration
just test-e2e
```

## Deployment

Kagzi can be deployed in various environments:

### Using Docker Images

Pre-built Docker images are available on GitHub Container Registry (GHCR):

```bash
# Pull the server image
docker pull ghcr.io/spa5k/kagzi/server:latest
docker pull ghcr.io/spa5k/kagzi/server:v0.1.0

# Run the server
docker run -d \
  --name kagzi-server \
  -p 50051:50051 \
  -e KAGZI_DB_URL="postgresql://user:pass@host:5432/dbname" \
  ghcr.io/spa5k/kagzi/server:latest
```

### Local Development

```bash
# Using Docker Compose (recommended for development)
docker-compose up -d

# Or build and run locally
cargo build --release
./target/release/kagzi-server
```

### Building Docker Images Locally

```bash
# Build all components
docker build -t kagzi:local .

# Build only the server
docker build -f Dockerfile.server -t kagzi-server:local .
```

**Note:** Kubernetes deployment guides will be provided as the project approaches beta stability.

## Observability

Kagzi includes structured logging and tracing support for workflow executions. Full metrics and OpenTelemetry integration are planned for future releases.

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

### Development Workflow

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Run `just lint` and `cargo test`
6. Submit a pull request

## License

This project is licensed under the Apache License 2.0 - see the [LICENSE](LICENSE) file for details.

## Support

- 📖 [Documentation](https://docs.kagzi.dev)
- 🐛 [Issue Tracker](https://github.com/spa5k/kagzi/issues)
- 💬 [Discussions](https://github.com/spa5k/kagzi/discussions)
- 📧 [Email](mailto:support@kagzi.dev)

## Roadmap

### Upcoming Features

- [ ] OpenTelemetry Integration (Metrics, Tracing)
- [ ] Web Dashboard for workflow visualization
- [ ] REST API Gateway alongside gRPC
- [ ] Workflow Templates and reusable patterns
- [ ] Additional Database Backends (MySQL, SQLite)
- [ ] Docker images for easy deployment
- [ ] Kubernetes operators and Helm charts
- [ ] Workflow Versioning and Migration tools
- [ ] Multi-tenant Support
- [ ] Workflow Hooks and Webhooks

---

<div align="center">

**Built with ❤️ by the open-source community**

[GitHub](https://github.com/spa5k/kagzi) • [Discussions](https://github.com/spa5k/kagzi/discussions) • [Issues](https://github.com/spa5k/kagzi/issues)

</div>
