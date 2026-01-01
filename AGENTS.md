# Kagzi Development Guide

This document provides guidelines for agents working on the Kagzi codebase. Kagzi is a workflow orchestration engine built in Rust 2024.

## Project Structure

- `crates/kagzi-proto`: Protobuf definitions and generated code
- `crates/kagzi-queue`: Queue abstraction layer
- `crates/kagzi-server`: Core gRPC server implementation
- `crates/kagzi-store`: Database models and repository layer
- `crates/tests`: End-to-end tests using testcontainers
- `sdk/kagzi-rs`: Public SDK for client applications
- `examples/`: Example applications demonstrating usage
- `migrations/`: SQLx database migrations
- `proto/`: Protobuf definition files

## Build Commands

```bash
# Build entire workspace (includes proto generation)
just build

# Build only proto crate (generates Rust from .proto files)
just build-proto

# Format all code (rustfmt + dprint + buf)
just tidy

# Lint with clippy (fails on warnings)
just lint

# Run the development server
just dev
```

## Database Commands

```bash
# Start PostgreSQL container
just db-up

# Stop PostgreSQL container
just db-down

# Reset database (destructive!)
just db-reset

# Run pending migrations
just migrate

# Create new migration
just migrate-add <name>
```

## Test Commands

```bash
# Run all tests (unit + integration)
just test

# Run unit tests only (fast, no Docker required)
just test-unit

# Run integration tests (requires Docker)
just test-integration

# Run specific integration test by name
just test-one <test_name>

# Run integration tests with output
just test-integration-verbose

# Run end-to-end tests
just test-e2e
```

## Code Style Guidelines

### Imports and Formatting

Imports are organized by group with `rustfmt` configured for:
- `imports_granularity = "Module"`: Each module gets its own import
- `group_imports = "StdExternalCrate"`: Std, external crates, then local modules

```rust
// Correct import ordering
use std::env;

use serde::{Deserialize, Serialize};
use tokio::time::Duration;

use kagzi::{Context, Worker};
```

### Naming Conventions

- **Functions/variables:** `snake_case`
- **Types/structs/enums:** `PascalCase`
- **Constants:** `SCREAMING_SNAKE_CASE`
- **Module names:** `snake_case`
- **Workflow/step names:** `snake_case` (used in database and API)

### Error Handling

- Use `anyhow::Result<T>` for application-level errors that need context
- Use `thiserror` to define custom error types for library/public APIs
- Propagate errors with `?` operator
- Wrap errors with `context()` or `with_context()` for debugging

```rust
// Application error
async fn example() -> anyhow::Result<Output> {
    something().await.context("failed to do thing")?
}

// Custom error type
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

### Async and Tokio

- Use `tokio` runtime with `#[tokio::main]` for binaries
- Prefer `async fn` over `Box<dyn Future>`
- Use `tokio::spawn` for background tasks
- Handle errors from spawned tasks

### Database (sqlx)

- Migrations live in `migrations/` directory
- Use `sqlx::query_as!` and `sqlx::query!` for type-safe queries
- Prepare offline queries: `just sqlx-prepare`
- UUIDs use `uuid::Uuid` with v4 or v7 features
- Timestamps use `chrono::DateTime<Utc>`

### Serialization

- Use `serde` with JSON for payload serialization
- Derive `Serialize`, `Deserialize` for input/output types
- Use `serde_json::Value` for dynamic/untyped payloads
- Derive `Debug` for all public structs

### Protobuf Files

- Located in `proto/` directory
- Run `just build-proto` to regenerate Rust code
- Follow tonic-prost conventions
- Message names use `PascalCase`

### Testing

- Unit tests: No external dependencies (no Docker)
- Integration tests: Require PostgreSQL via Docker
- Set `KAGZI_POLL_TIMEOUT_SECS=2` for faster test iterations
- Use `--test-threads=1` for integration tests to avoid race conditions

## Development Workflow

1. Start database: `just db-up`
2. Run migrations: `just migrate`
3. Start dev server: `just dev` (listens on 0.0.0.0:50051)
4. Make changes and test
5. Run lint before committing: `just lint`

## Proto File Conventions

```protobuf
// Service names use PascalCase
service WorkflowService {
    rpc StartWorkflow(StartWorkflowRequest) returns (WorkflowRun);
}

// Message names use PascalCase
message StartWorkflowRequest {
    string workflow_id = 1;
    bytes input = 2;
}
```

## Common Patterns

### Workflow Definition

```rust
async fn my_workflow(ctx: Context, input: Input) -> anyhow::Result<Output> {
    let step1 = ctx.step("step_name").run(|| async {
        // Step logic
        Ok(result)
    }).await?;
    Ok(output)
}
```

### Worker Registration

```rust
let mut worker = Worker::new(server_url)
    .namespace("my_namespace")
    .workflows([("workflow_name", my_workflow)])
    .build()
    .await?;

tokio::spawn(async move {
    if let Err(e) = worker.run().await {
        eprintln!("Worker error: {:?}", e);
    }
});
```

## Beads (Issue Tracking)

Issues are tracked with Beads in `.beads/issues.jsonl`:

```bash
bd create "Feature description"
bd list
bd show <issue-id>
bd update <issue-id> --status in_progress
bd update <issue-id> --status done
bd sync
```

## Environment Variables

- `DATABASE_URL`: PostgreSQL connection string
- `KAGZI_POLL_TIMEOUT_SECS`: Poll timeout for integration tests
- `KAGZI_SERVER_URL`: Server URL for examples (default: http://localhost:50051)
- `KAGZI_NAMESPACE`: Workflow namespace for examples
