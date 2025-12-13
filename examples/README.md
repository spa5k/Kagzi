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

## Macro Examples

Kagzi provides procedural macros to reduce boilerplate and improve observability:

### 11_macros - Overview

Demonstrates both `#[kagzi_step]` and `kagzi_workflow!` macros.

### 12_kagzi_step_macro - Step Attribute Macro

Shows how the `#[kagzi_step]` attribute macro enhances individual workflow step functions with:

- Automatic tracing span creation
- Input/output logging at debug level
- Error context enrichment
- Structured logging

```bash
cargo run --example 12_kagzi_step_macro
```

### 13_kagzi_workflow_macro - Workflow Macro

Demonstrates the `kagzi_workflow!` procedural macro that provides:

- Clean block-based syntax
- Built-in `run!` helper macro
- Automatic workflow-level tracing
- Reduced boilerplate

```bash
cargo run --example 13_kagzi_workflow_macro
```

### 14_macros_comprehensive - Complete E-commerce Example

A comprehensive example showing both macros working together in a real-world e-commerce order processing workflow:

- Order validation
- Pricing calculation
- Inventory reservation
- Payment processing
- Shipping scheduling
- Email notifications

```bash
cargo run --example 14_macros_comprehensive
```

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

# Run all macro examples
for ex in 11_macros 12_kagzi_step_macro 13_kagzi_workflow_macro 14_macros_comprehensive; do
    echo "Running $ex"
    cargo run --example $ex
done
```

## Macro Usage

### Step Macro

```rust
use kagzi_macros::kagzi_step;

#[kagzi_step]
async fn validate_order(
    ctx: WorkflowContext,
    input: OrderRequest,
) -> anyhow::Result<ValidationResult> {
    // Your implementation
    // Automatic tracing, logging, and error context added
}
```

### Workflow Macro

```rust
use kagzi_macros::kagzi_workflow;

kagzi_workflow! {
    pub async fn process_order(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        let validated = run!("validate", validate_order(&input));
        let payment = run!("pay", process_payment(&validated));
        Ok(result)
    }
}
```

## Benefits

- **50-70% less boilerplate code**
- **Automatic observability** without manual setup
- **Type-safe** workflow definitions
- **Excellent IDE support** with rust-analyzer
- **Zero runtime overhead** - all expansion at compile time
