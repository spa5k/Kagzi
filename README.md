# Kagzi

**The Rust workflow engine.**

> [!WARNING]
> Kagzi is currently in alpha stage and not recommended for production use.

Kagzi is a durable workflow engine for Rust. Think Temporal or Inngest, but simpler and built for Rust developers.

## Why Kagzi?

- **Reliable execution**: Workflows survive server crashes and restarts
- **Built for Rust**: Native async/await with full type safety
- **Simple API**: Focus on your business logic, not infrastructure
- **Durable**: State persisted in PostgreSQL
- **Scheduled workflows**: Built-in cron support
- **Horizontally scalable**: Add more workers as you grow

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

```bash
# 1. Start PostgreSQL
docker-compose up -d

# 2. Start the Kagzi server
cargo run -p kagzi-server

# 3. Run a workflow
cargo run -p kagzi --example simple
```

### Basic Example

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

async fn greet_workflow(
    mut ctx: WorkflowContext,
    input: Input,
) -> anyhow::Result<Output> {
    // This step survives server restarts
    let greeting = ctx
        .run("greet", async {
            Ok(format!("Hello, {}!", input.name))
        })
        .await?;

    Ok(Output { message: greeting })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Start worker in background
    let mut worker = Worker::builder("http://localhost:50051", "default")
        .build()
        .await?;
    worker.register("greet", greet_workflow);
    tokio::spawn(async move { worker.run().await });

    // Start workflow
    let mut client = Client::connect("http://localhost:50051").await?;
    let run_id = client
        .workflow("greet", "default", Input {
            name: "World".to_string(),
        })
        .await?;

    println!("Started workflow: {}", run_id);
    Ok(())
}
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
cargo run -p kagzi --example simple      # Basic workflow
cargo run -p kagzi --example scheduling  # Cron jobs
cargo run -p kagzi --example saga        # Compensation pattern
cargo run -p kagzi --example fanout      # Parallel processing
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

Kagzi includes structured logging and tracing support for workflow executions. Full metrics and OpenTelemetry integration are planned for future releases.

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

Apache 2.0 - see [LICENSE](LICENSE) file.

## Links

- [GitHub](https://github.com/spa5k/kagzi)
- [Issues](https://github.com/spa5k/kagzi/issues)
- [Discussions](https://github.com/spa5k/kagzi/discussions)
