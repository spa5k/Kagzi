# Kagzi Examples

This directory contains examples demonstrating various features of the Kagzi workflow engine.

## Core Examples

- **01_basics** - Basic workflow execution
- **02_error_handling** - Error handling patterns
- **03_scheduling** - Cron-based workflow scheduling
- **04_concurrency** - Running workflows concurrently
- **05_fan_out_in** - Fan-out/fan-in patterns
- **06_long_running** - Long-running workflows
- **07_idempotency** - Ensuring idempotent operations
- **08_saga_pattern** - Compensation-based transactions
- **09_data_pipeline** - Data processing pipeline
- **10_multi_queue** - Multiple queue management

## Running Examples

### Prerequisites

1. Start PostgreSQL:

```bash
docker-compose up -d
```

2. Start the Kagzi server:

```bash
cargo run -p kagzi-server
```

### Running Individual Examples

```bash
# Run a specific example
cargo run --example <example_name>

# Run with debug logging
RUST_LOG=debug cargo run --example <example_name>

# Run all core examples
for ex in 01_basics 02_error_handling 03_scheduling; do
    echo "Running $ex"
    cargo run --example $ex
done
```
