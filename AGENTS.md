# Kagzi Development Guide

Kagzi is a workflow orchestration engine built in Rust 2024.

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
just build              # Build entire workspace (includes proto generation)
just build-proto        # Build only proto crate (generates Rust from .proto files)
just tidy               # Format all code (rustfmt + dprint + buf)
just lint               # Lint with clippy (fails on warnings)
just dev                # Run the development server
```

## Database Commands

```bash
just db-up              # Start PostgreSQL container
just db-down            # Stop PostgreSQL container
just db-reset           # Reset database (destructive!)
just migrate            # Run pending migrations
just migrate-add <name> # Create new migration
```

## Test Commands

```bash
just test                    # Run all tests (unit + integration)
just test-unit               # Run unit tests only (fast, no Docker required)
just test-integration        # Run integration tests (requires Docker)
just test-one <test_name>    # Run specific integration test by name
just test-integration-verbose # Run integration tests with output
just test-e2e                # Run end-to-end tests
```

## Code Style Guidelines

### Naming Conventions

- **Functions/variables:** `snake_case`
- **Types/structs/enums:** `PascalCase`
- **Constants:** `SCREAMING_SNAKE_CASE`
- **Module names:** `snake_case`
- **Workflow/step names:** `snake_case` (used in database and API)

### Error Handling

- Use `anyhow::Result<T>` for application-level errors
- Use `thiserror` to define custom error types for library/public APIs
- Propagate errors with `?` operator
- Wrap errors with `context()` for debugging

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

## Development Workflow

1. Start database: `just db-up`
2. Run migrations: `just migrate`
3. Start dev server: `just dev` (listens on 0.0.0.0:50051)
4. Make changes and test
5. Run lint before committing: `just lint`

## Environment Variables

- `DATABASE_URL`: PostgreSQL connection string
- `KAGZI_POLL_TIMEOUT_SECS`: Poll timeout for integration tests
- `KAGZI_SERVER_URL`: Server URL for examples (default: http://localhost:50051)
- `KAGZI_NAMESPACE`: Workflow namespace for examples

## Beads (Issue Tracking)

Issues tracked in `.beads/issues.jsonl`:

```bash
bd create "Feature description"
bd list
bd show <issue-id>
bd update <issue-id> --status in_progress
bd update <issue-id> --status done
bd sync
```
