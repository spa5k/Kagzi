# Kagzi Macros

Procedural macros for the Kagzi workflow engine to reduce boilerplate and improve observability.

## Available Macros

### `#[kagzi_step]` Attribute Macro

Enhances workflow step functions with automatic observability features.

```rust
use kagzi_macros::kagzi_step;
use kagzi::WorkflowContext;

// With WorkflowContext parameter
#[kagzi_step]
/// Validates an order and calculates total
async fn validate_order(
    ctx: WorkflowContext,
    input: OrderRequest,
) -> anyhow::Result<ValidationResult> {
    if input.items.is_empty() {
        return Err(OrderError::EmptyOrder.into());
    }

    let total: f64 = input.items.iter()
        .map(|i| i.price * i.quantity as f64)
        .sum();

    Ok(ValidationResult {
        is_valid: true,
        total_amount: total,
    })
}

// Without WorkflowContext parameter
#[kagzi_step]
/// Validates an order and calculates total
async fn validate_order_simple(
    input: OrderRequest,
) -> anyhow::Result<ValidationResult> {
    if input.items.is_empty() {
        return Err(OrderError::EmptyOrder.into());
    }

    let total: f64 = input.items.iter()
        .map(|i| i.price * i.quantity as f64)
        .sum();

    Ok(ValidationResult {
        is_valid: true,
        total_amount: total,
    })
}
```

The macro expands to include:

- Automatic tracing span creation with step name
- Input/output logging at debug level
- Error context enrichment with step name
- Structured logging for better observability

#### Requirements

Functions using `#[kagzi_step]` must:

1. Be `async`
2. Have 1 or 2 parameters:
   - Optionally: `WorkflowContext` (or similar context type) as first parameter
   - Required: The input parameter
3. Return a `Result<T, E>` type (typically `anyhow::Result<T>`)

### `kagzi_workflow!` Macro

Provides a block-based syntax for defining workflows with a `run!` helper macro.

```rust
use kagzi_macros::kagzi_workflow;
use kagzi::WorkflowContext;

kagzi_workflow! {
    pub async fn process_order(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        // Run validation step
        let validated = run!("validate", validate_order(&input));

        // Run inventory reservation
        let inventory = run!("reserve", reserve_inventory(&validated));

        // Run payment processing
        let payment = run!("pay", process_payment(&validated));

        // Return final result
        Ok(OrderResult {
            order_id: input.order_id,
            inventory,
            payment,
            status: "completed".to_string(),
        })
    }
}
```

The macro:

- Defines a `run!` macro for step execution
- Adds workflow-level tracing
- Simplifies step chaining with automatic error propagation

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
kagzi-macros = "0.0.1-alpha.1"
kagzi = "0.0.1-alpha.1"
```

## Usage

1. Import the macros:

```rust
use kagzi_macros::{kagzi_step, kagzi_workflow};
use kagzi::prelude::*;
```

2. Apply `#[kagzi_step]` to step functions
3. Use `kagzi_workflow!` to define workflow blocks

## Example

See `examples/11_macros/main.rs` for a complete working example demonstrating the macros.

## Benefits

- **Reduced Boilerplate**: Automatic tracing and logging
- **Better Observability**: Structured logging with context
- **Cleaner Syntax**: Concise workflow definitions
- **Type Safety**: Compile-time checking of function signatures
- **IDE Support**: Excellent rust-analyzer compatibility

## Development Notes

The macros are implemented using procedural macro system with:

- `proc_macro2` for better token stream handling
- `syn` for parsing Rust code
- `quote` for code generation
- No runtime overhead - all expansion happens at compile time
