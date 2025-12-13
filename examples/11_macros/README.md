# Kagzi Macros Example

This example demonstrates the procedural macros provided by the Kagzi SDK to reduce boilerplate when defining workflows and steps.

## Macros Implemented

### 1. `#[kagzi_step]` Attribute Macro

Enhances async workflow step functions with:

- Automatic tracing span creation with step name
- Input/output logging at debug level
- Error context enrichment
- Structured logging for better observability

```rust
#[kagzi_step]
async fn validate_order(
    _ctx: WorkflowContext,
    input: ProcessOrderInput,
) -> anyhow::Result<ProcessOrderInput> {
    // Your implementation
}
```

### 2. `kagzi_workflow!` Macro

Provides a block-based syntax for defining workflows with a `run!` helper macro:

- Defines a local `run!` macro for step execution
- Adds workflow-level tracing
- Simplifies step chaining with automatic error propagation

```rust
kagzi_workflow! {
    pub async fn process_order(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        let validated = run!("validate", validate_order(&input));
        // ...
    }
}
```

## Running the Example

```bash
# Run the example
cargo run --example 11_macros

# Run with debug logging to see macro-generated traces
RUST_LOG=debug cargo run --example 11_macros
```

## Key Benefits

1. **Reduced Boilerplate**: No manual tracing setup needed
2. **Better Observability**: Automatic structured logging
3. **Error Context**: Errors are automatically enriched with step information
4. **Type Safety**: Compile-time checking of function signatures
5. **Zero Runtime Overhead**: All expansion happens at compile time

## Implementation Notes

- Macros use the `proc_macro2`, `syn`, and `quote` crates
- No changes to the core Kagzi runtime needed
- Existing code continues to work without modification
- IDE support is excellent with rust-analyzer
