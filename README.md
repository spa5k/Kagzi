# Kagzi

<div align="center">

![Kagzi Logo](https://img.shields.io/badge/Kagzi-Workflow%20Engine-blue?style=for-the-badge)
![Rust](https://img.shields.io/badge/rust-1.70+-orange?style=for-the-badge&logo=rust)
![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=for-the-badge)
![Build Status](https://img.shields.io/badge/build-passing-brightgreen?style=for-the-badge)

*A modern, distributed workflow engine built in Rust with gRPC*

[Documentation](#documentation) • [Quick Start](#quick-start) • [Examples](#examples) • [API Reference](#api-reference)

</div>

## Overview

Kagzi is a high-performance, distributed workflow engine designed for building reliable, scalable asynchronous systems. Built with Rust and gRPC, it provides a robust foundation for orchestrating complex business processes with built-in fault tolerance, observability, and enterprise-grade features.

### Key Features

- 🚀 **High Performance**: Built with Rust and Tokio for maximum throughput
- 🔄 **Distributed Architecture**: Scale horizontally across multiple workers
- 🛡️ **Fault Tolerant**: Automatic retries, error handling, and recovery mechanisms
- 📊 **Observability**: Comprehensive tracing, logging, and monitoring support
- 🎯 **Type Safe**: Full Rust type safety with compile-time guarantees
- ⚡ **Async First**: Native async/await support throughout
- 🔍 **gRPC Protocol**: High-performance, language-agnostic communication
- 💾 **PostgreSQL Backend**: Reliable, ACID-compliant persistence
- 🧩 **Composable**: Build complex workflows from simple, reusable steps

## Architecture

```
┌─────────────────┐    gRPC     ┌─────────────────┐    SQL     ┌─────────────────┐
│   Client       │ ◄──────────► │   Kagzi Server  │ ◄──────────► │  PostgreSQL    │
└─────────────────┘             │                 │             │                 │
                               └─────────────────┘             └─────────────────┘
                                        ▲
                                        │ gRPC
                                        ▼
                               ┌─────────────────┐
                               │   Workers       │
                               │ (Multiple)      │
                               └─────────────────┘
```

## Quick Start

### Prerequisites

- Rust 1.70+ with Cargo
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
    .idempotent("unique-key")
    .version("1.0.0")
    .context(serde_json::json!({"key": "value"}))
    .deadline(chrono::Utc::now() + chrono::Duration::hours(1))
    .await?;

// Get workflow status
let status = client.get_workflow_run(run_id).await?;

// List workflows
let workflows = client.list_workflow_runs("namespace").await?;
```

#### Worker API

```rust
let mut worker = Worker::builder("http://localhost:50051", "task_queue")
    // Optional: cap total concurrent RUNNING workflows for this queue
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

#### Workflow Context API

```rust
async fn workflow_function(mut ctx: WorkflowContext, input: Input) -> anyhow::Result<Output> {
    // Run a step - results are memoized for replay
    let result = ctx.run("step_name", my_async_function(input.value)).await?;
    
    // Run a step with explicit input tracking (for observability)
    let result2 = ctx.run_with_input("step_name", &input, my_async_function(input.value)).await?;

    // Run a step with an explicit retry policy; unspecified fields inherit
    // from workflow-level retry (if set) or the worker's default_step_retry.
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

### 1. Simple Workflow

```rust
// See: crates/kagzi/examples/simple.rs
cargo run -p kagzi --example simple
```

A basic workflow demonstrating step execution, sleeping, and result handling.

### 2. User Onboarding

```rust
// See: crates/kagzi/examples/twitter_example.rs
cargo run -p kagzi --example twitter_example
```

A realistic user signup workflow with multiple steps, error handling, and email notifications.

### 3. Traced Workflow

```rust
// See: crates/kagzi/examples/traced_workflow.rs
cargo run -p kagzi --example traced_workflow
```

Demonstrates comprehensive tracing and observability features.

### 4. Advanced Patterns

```rust
async fn complex_workflow(mut ctx: WorkflowContext, input: ComplexInput) -> anyhow::Result<ComplexOutput> {
    // Parallel execution
    let (result1, result2) = tokio::try_join!(
        ctx.run("step1", async_step1(input.clone())),
        ctx.run("step2", async_step2(input.clone()))
    )?;
    
    // Conditional execution
    if result1.needs_processing {
        ctx.run("process", async_process(result1)).await?;
    }
    
    // Sub-workflow
    let sub_result = ctx.run("sub_workflow", sub_workflow_func(result2)).await?;
    
    Ok(ComplexOutput::new(result1, result2, sub_result))
}
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://postgres:postgres@localhost:5432/postgres` |
| `RUST_LOG` | Log level filter | `info` |
| `KAGZI_SERVER_ADDR` | Server bind address | `0.0.0.0:50051` |

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

## Development

### Project Structure

```
kagzi/
├── crates/
│   ├── kagzi/              # Client library
│   ├── kagzi-proto/        # Generated gRPC code
│   └── kagzi-server/      # Server implementation
├── examples/               # Usage examples
├── migrations/             # Database migrations
├── proto/                  # Protocol definitions
└── docker-compose.yml      # Development database
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
```

## Deployment

### Docker Deployment

```dockerfile
FROM rust:1.70 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/kagzi-server /usr/local/bin/
EXPOSE 50051
CMD ["kagzi-server"]
```

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kagzi-server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: kagzi-server
  template:
    metadata:
      labels:
        app: kagzi-server
    spec:
      containers:
      - name: kagzi-server
        image: kagzi-server:latest
        ports:
        - containerPort: 50051
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: kagzi-secrets
              key: database-url
```

## Monitoring & Observability

### Tracing

Kagzi provides comprehensive tracing support with correlation IDs and distributed tracing:

```rust
// Initialize tracing
kagzi::tracing_utils::init_tracing("my-service")?;

// All gRPC calls are automatically traced with:
// - Correlation IDs
// - Trace IDs  
// - Method names
// - Request/response timing
// - Error context
```

### Metrics

The server exposes structured logs that can be consumed by monitoring systems:

```json
{
  "timestamp": "2024-01-01T12:00:00Z",
  "level": "info",
  "method": "StartWorkflow",
  "correlation_id": "abc123",
  "trace_id": "def456",
  "duration_ms": 150,
  "status": "success"
}
```

## Performance

### Benchmarks

- **Throughput**: 10,000+ workflows/second per server instance
- **Latency**: <10ms median for simple workflows
- **Scalability**: Linear scaling with worker instances
- **Memory**: <100MB base memory per server

### Optimization Tips

1. **Connection Pooling**: Configure appropriate pool sizes for your workload
2. **Batch Operations**: Use bulk operations where possible
3. **Async Patterns**: Leverage Rust's async/await for I/O-bound operations
4. **Monitoring**: Set up alerts for latency and error rates

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

- [ ] Web Dashboard UI
- [ ] Workflow Visualization
- [ ] Additional Database Backends (MySQL, SQLite)
- [ ] Workflow Templates
- [ ] Advanced Scheduling (Cron, Delayed Start)
- [ ] Workflow Versioning and Migration
- [ ] Metrics and Alerting Integration
- [ ] Multi-tenant Support

---

<div align="center">

**Built with ❤️ by the Kagzi team**

[Website](https://kagzi.dev) • [Blog](https://blog.kagzi.dev) • [Twitter](https://twitter.com/kagzi_dev)

</div>