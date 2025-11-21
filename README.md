# Kagzi

A durable workflow engine for Rust, inspired by Temporal, Inngest, and OpenWorkflow.

Kagzi allows you to build resilient, long-running workflows that can survive crashes, restarts, and even months of sleep - all backed by PostgreSQL.

## Features

- ✅ **Durable Execution**: Workflows survive crashes and restarts
- ✅ **Step Memoization**: Completed steps are cached and never re-executed
- ✅ **Durable Sleep**: Sleep for seconds or months without holding resources
- ✅ **Type Safe**: Full Rust type safety with serde serialization
- ✅ **PostgreSQL Backend**: Simple, reliable persistence
- ✅ **Automatic Migrations**: Database schema managed automatically
- ✅ **Worker Pool**: Scale horizontally with multiple workers

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
│  - WorkflowContext                              │
│  - Step memoization                             │
│  - Client & Worker                              │
└────────────┬────────────────────────────────────┘
             │ talks to
             ▼
┌─────────────────────────────────────────────────┐
│          PostgreSQL                             │
│  - workflow_runs                                │
│  - step_runs (memoization)                      │
│  - worker_leases (coordination)                 │
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

## Roadmap

### Phase 1 (Current) ✅
- [x] PostgreSQL backend
- [x] Basic workflow execution
- [x] Step memoization
- [x] Durable sleep
- [x] Worker polling
- [x] Automatic migrations

### Phase 2
- [ ] Retry policies and exponential backoff
- [ ] Parallel step execution
- [ ] Workflow versioning
- [ ] Observability and metrics
- [ ] Web dashboard
- [ ] SQLite backend
- [ ] Redis backend

### Phase 3
- [ ] gRPC server for multi-language support
- [ ] Python SDK
- [ ] TypeScript SDK
- [ ] Go SDK

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT
