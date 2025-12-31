# Kagzi

<div align="center">

![Kagzi Logo](https://img.shields.io/badge/Kagzi-Workflow%20Engine-blue?style=for-the-badge)
![Rust](https://img.shields.io/badge/rust-1.75+-orange?style=for-the-badge&logo=rust)
![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=for-the-badge)
![Status](https://img.shields.io/badge/status-Alpha-orange?style=for-the-badge)

_A modern, distributed workflow engine built in Rust with gRPC_

</div>

> [!WARNING]
> Kagzi is currently in alpha stage and not recommended for production use.

Kagzi is a durable workflow engine for Rust. Think Temporal or Inngest, but simpler.

## Why Kagzi?

- **Reliable execution**: Workflows survive server crashes and restarts
- **Built for Rust**: Native async/await with full type safety
- **Simple API**: Focus on your business logic, not infrastructure
- **Durable**: State persisted in PostgreSQL
- **Scheduled workflows**: Built-in cron support
- **Horizontally scalable**: Add more workers as you grow
- **Multi-language support**: Will support multiple languages in the future through gRPC

## Architecture

```text
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

```bash
# 1. Start PostgreSQL
docker-compose up -d

# 2. Start the Kagzi server
cargo run -p kagzi-server

# 3. Run a workflow
cargo run -p kagzi --example simple
```

### Quick Example

```rust
use kagzi::{Client, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmailInput {
    user_id: String,
    user_email: String,
}

async fn send_welcome_email_workflow(
    mut ctx: WorkflowContext,
    input: SendWelcomeEmailInput,
) -> anyhow::Result<()> {
    // Step 1: Fetch user (memoized - runs once)
    let user = ctx
        .run("fetch-user", async {
            fetch_user_from_db(&input.user_id).await
        })
        .await?;

    // Step 2: Send email (runs only if first step succeeded)
    ctx.run("send-email", async {
        send_welcome_email(&user.email).await
    })
        .await?;

    // Step 3: Update user record
    ctx.run("mark-welcome-sent", async {
        mark_welcome_email_sent(&input.user_id).await
    })
        .await?;

    Ok(())
}
```

**Separate your workflow logic from your application:**

```rust
// worker.rs - Run this in the background
let mut worker = Worker::builder("http://localhost:50051", "email")
    .build()
    .await?;
worker.register("send-welcome-email", send_welcome_email_workflow);
worker.run().await?;

// main.rs - Trigger from your API
let mut client = Client::connect("http://localhost:50051").await?;
client.workflow(
    "send-welcome-email",
    "email",
    SendWelcomeEmailInput {
        user_id: "123",
        user_email: "alice@example.com",
    },
)
.retries(3)
.await?;
```

**Run the complete example:**

```bash
# Terminal 1: Start Kagzi server
cargo run -p kagzi-server

# Terminal 2: Start the worker
cargo run -p kagzi --example separate_files_example worker

# Terminal 3: Trigger a workflow
cargo run -p kagzi --example separate_files_example trigger
```

## API

### Client

```rust
let mut client = Client::connect("http://localhost:50051").await?;

// Start a workflow
let run_id = client
    .workflow("process_payment", "default", payment_data)
    .retries(3)
    .await?;
```

### Worker

```rust
let mut worker = Worker::builder("http://localhost:50051", "default")
    .max_concurrent(50)
    .build()
    .await?;

worker.register("process_payment", process_payment_workflow);
worker.run().await?;
```

### Workflow Context

```rust
async fn process_payment_workflow(
    mut ctx: WorkflowContext,
    payment: Payment,
) -> anyhow::Result<Result> {
    // Charge payment - retries on failure
    let charge = ctx
        .run("charge", async { charge_card(&payment).await })
        .await?;

    // Send receipt - skipped if workflow is replayed
    let receipt = ctx
        .run("send_receipt", async { send_email(&payment).await })
        .await?;

    Ok(charge)
}
```

### Cron Schedules

```rust
// Run daily at 9 AM UTC
client.schedule(
    "daily_report",
    "reports",
    "0 0 9 * * *",
    report_data,
)
.create()
.await?;
```

## Examples

```bash
cargo run -p kagzi --example simple              # Basic workflow
cargo run -p kagzi --example scheduling          # Cron jobs
cargo run -p kagzi --example saga                # Compensation pattern
cargo run -p kagzi --example fanout              # Parallel processing
```

## Configuration

Set environment variables:

```bash
export KAGZI_DB_URL="postgresql://user:pass@host:5432/db"
export KAGZI_SERVER_PORT=50051
export RUST_LOG=info
```

**Note**: Payloads over 2MB are rejected. Store large data externally.

### Database

```bash
# Start PostgreSQL
docker-compose up -d

# Run migrations
sqlx migrate run --database-url "postgresql://postgres:postgres@localhost:54122/postgres"
```

## Development

```bash
# Setup
just setup

# Run server
just dev

# Run tests
just test

# Run example
just example simple
```

## Deployment

### Docker

```bash
# Pull image
docker pull ghcr.io/spa5k/kagzi/server:latest

# Run
docker run -d \
  --name kagzi \
  -p 50051:50051 \
  -e KAGZI_DB_URL="$DATABASE_URL" \
  ghcr.io/spa5k/kagzi/server:latest
```

### From Source

```bash
cargo build --release
./target/release/kagzi-server
```

## Observability

Kagzi includes structured logging for workflow executions. Full metrics and tracing integration are planned for future releases.

## Roadmap

### ✅ Current Release

| Feature                    | Status  |
| -------------------------- | ------- |
| PostgreSQL backend         | ✅ Done |
| Durable workflow execution | ✅ Done |
| Step memoization & retries | ✅ Done |
| Cron-based scheduling      | ✅ Done |
| Concurrent workers         | ✅ Done |
| gRPC protocol              | ✅ Done |
| Saga pattern support       | ✅ Done |

### 🚧 Upcoming Features

| Feature                      | Version | Status        |
| ---------------------------- | ------- | ------------- |
| Web Dashboard UI             | v0.2    | 🏗️ In Progress |
| OpenTelemetry integration    | v0.3    | 📋 Planned    |
| REST API Gateway             | v0.3    | 📋 Planned    |
| Workflow versioning          | v0.4    | 📋 Planned    |
| Additional database backends | v0.4    | 📋 Planned    |
| Python SDK                   | v0.5    | 📋 Planned    |
| CLI tools                    | v0.5    | 📋 Planned    |
| Workflow templates           | v0.6    | 📋 Planned    |
| Signals & events             | v0.6    | 📋 Planned    |

### 🔮 Future Roadmap

| Feature                   | Target  | Description                          |
| ------------------------- | ------- | ------------------------------------ |
| Multi-tenant support      | v1.0    | Isolate workflows per tenant         |
| Workflow hooks            | v1.0    | Trigger workflows on external events |
| Advanced retry policies   | v1.1    | Custom retry strategies              |
| Kubernetes operator       | v1.2    | Native K8s deployment                |
| Workflow visualization    | v1.2    | Visual workflow designer             |
| Performance optimizations | Ongoing | Continuous improvements              |

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

Apache 2.0 - see [LICENSE](LICENSE) file.

## Links

- [GitHub](https://github.com/spa5k/kagzi)
- [Issues](https://github.com/spa5k/kagzi/issues)
- [Discussions](https://github.com/spa5k/kagzi/discussions)
