# Kagzi Macros

Procedural macros for the Kagzi workflow engine that reduce boilerplate, enhance observability, and provide a clean syntax for defining workflows and steps.

## Overview

The `kagzi-macros` crate provides three macros that work together to create a declarative, observable workflow system:

- **`#[kagzi_step]`** - Attribute macro that enhances workflow step functions with automatic tracing, logging, and error context enrichment
- **`kagzi_workflow!`** - Macro that creates workflow functions with a built-in `run!` helper for executing steps
- **`run!`** - Helper macro (automatically defined inside workflows) that executes steps with proper observability

These macros enable you to write workflows that are:

- **Observable** - Automatic tracing spans, structured logging, and error context
- **Concise** - Minimal boilerplate while maintaining type safety
- **Maintainable** - Clear separation between workflow orchestration and step implementation
- **Production-ready** - Built-in error handling and observability hooks

## Macros Reference

### `#[kagzi_step]`

Attribute macro that enhances an async function with workflow step observability features.

#### Function Signature Requirements

The macro validates that the decorated function meets these requirements:

- Must be `async`
- Must have 1 or 2 parameters (optionally `WorkflowContext` as first parameter, then input)
- Must return `Result<T, E>` (any error type)

#### What It Adds

The macro automatically wraps your function body with:

1. **Tracing Span** - Creates a span with the step name under the `"step"` target
2. **Debug Logging** - Logs step start with input and completion with output
3. **Error Enrichment** - Adds context to errors using `anyhow::Context`
4. **Instrumentation** - Uses `tracing::Instrument` to attach the span to the async block

#### Generated Code Pattern

```rust
#[kagzi_step]
async fn my_step(input: InputType) -> Result<OutputType, ErrorType> {
    // Your implementation
}
```

Expands to:

```rust
async fn my_step(input: InputType) -> anyhow::Result<OutputType> {
    use anyhow::Context;
    use tracing::Instrument;

    let span = tracing::info_span!("step", step = "my_step");

    async {
        tracing::debug!("Starting step: my_step");

        let result: anyhow::Result<OutputType> = {
            // Your implementation
        };

        match result {
            Ok(output) => {
                tracing::debug!(output = ?output, "Step completed successfully: my_step");
                Ok(output)
            }
            Err(e) => {
                tracing::error!(error = %e, "Step failed: my_step");
                Err(e.context(format!("Step 'my_step' failed")))
            }
        }
    }.instrument(span).await
}
```

### `kagzi_workflow!`

Macro that creates a workflow function with a built-in `run!` helper macro for executing steps.

#### What It Adds

1. **Workflow-Level Tracing** - Creates a span with the workflow name under the `"workflow"` target
2. **`run!` Macro Definition** - Defines a local macro that executes steps via `ctx.run()`
3. **Async Instrumentation** - Wraps the workflow body in an instrumented async block
4. **Info Logging** - Logs workflow start

#### Usage Pattern

```rust
kagzi_workflow! {
    pub async fn my_workflow(
        mut ctx: WorkflowContext,
        input: InputType,
    ) -> Result<OutputType, Error> {
        let step1_result = run!("step1", step1_function(&input));
        let step2_result = run!("step2", step2_function(&step1_result));
        Ok(OutputType { /* ... */ })
    }
}
```

#### Generated Code Pattern

```rust
pub async fn my_workflow(
    mut ctx: WorkflowContext,
    input: InputType,
) -> Result<OutputType, Error> {
    use tracing::Instrument;

    macro_rules! run {
        ($step_name:expr, $step_expr:expr) => {
            {
                ctx.run($step_name, $step_expr).await?
            }
        };
    }

    let span = tracing::info_span!("workflow", workflow = "my_workflow");

    async move {
        tracing::info!("Starting workflow: my_workflow");

        let step1_result = run!("step1", step1_function(&input));
        let step2_result = run!("step2", step2_function(&step1_result));
        Ok(OutputType { /* ... */ })
    }.instrument(span).await
}
```

### `run!`

Helper macro automatically defined within `kagzi_workflow!` blocks.

#### Syntax

```rust
run!("step_name", expression)
```

#### Parameters

- `"step_name"` - String literal used for tracing and metrics
- `expression` - Step function call or any async expression returning `Result<T, E>`

#### Behavior

- Executes the expression via `ctx.run(step_name, expression).await?`
- The `?` propagates errors automatically
- The step name is used for observability (tracing, metrics)

**Note:** The `run!` macro is only available inside functions created with `kagzi_workflow!`. It is not available for direct use outside of workflow definitions.

## Usage Examples

### Basic Step with `#[kagzi_step]`

```rust
use kagzi_macros::kagzi_step;

#[kagzi_step]
async fn validate_order(input: OrderRequest) -> anyhow::Result<ValidationResult> {
    if input.items.is_empty() {
        return Err(anyhow::anyhow!("Order cannot be empty"));
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

### Step with Context Parameter

```rust
use kagzi::WorkflowContext;
use kagzi_macros::kagzi_step;

#[kagzi_step]
async fn reserve_inventory(
    ctx: WorkflowContext,
    input: ValidationResult,
) -> anyhow::Result<InventoryReservation> {
    // Access workflow context for correlation IDs, etc.
    let correlation_id = ctx.correlation_id();

    tracing::info!(correlation_id, "Reserving inventory");

    Ok(InventoryReservation {
        reservation_id: "res_123".to_string(),
        items: input.items.clone(),
    })
}
```

### Complete Workflow

```rust
use kagzi::WorkflowContext;
use kagzi_macros::{kagzi_step, kagzi_workflow};

#[kagzi_step]
async fn validate_order(input: OrderRequest) -> anyhow::Result<ValidationResult> {
    if input.items.is_empty() {
        return Err(anyhow::anyhow!("Order cannot be empty"));
    }
    Ok(ValidationResult { is_valid: true })
}

#[kagzi_step]
async fn reserve_inventory(input: ValidationResult) -> anyhow::Result<Inventory> {
    Ok(Inventory { reserved: true })
}

#[kagzi_step]
async fn process_payment(input: Inventory) -> anyhow::Result<Payment> {
    Ok(Payment { status: "paid".to_string() })
}

kagzi_workflow! {
    pub async fn process_order(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        // Execute steps in sequence
        let validated = run!("validate", validate_order(input));
        let inventory = run!("reserve", reserve_inventory(validated));
        let payment = run!("pay", process_payment(inventory));

        Ok(OrderResult {
            order_id: "order_123".to_string(),
            payment_status: payment.status,
        })
    }
}
```

### Parallel Execution Pattern

```rust
kagzi_workflow! {
    async fn parallel_workflow(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<Output> {
        // Run independent steps in parallel
        let (result1, result2) = tokio::try_join!(
            async { run!("step1", step1(&input)) },
            async { run!("step2", step2(&input)) }
        )?;

        // Use results
        let final_result = run!("combine", combine_steps(result1, result2));

        Ok(final_result)
    }
}
```

### Conditional Execution

```rust
kagzi_workflow! {
    async fn conditional_workflow(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<Output> {
        let validated = run!("validate", validate(&input))?;

        if input.requires_verification {
            let verified = run!("verify", verify(validated))?;
            Ok(verified)
        } else {
            let processed = run!("process", process(validated))?;
            Ok(processed)
        }
    }
}
```

### Error Handling with Context

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OrderError {
    #[error("Invalid order: {0}")]
    Invalid(String),
    #[error("Inventory unavailable")]
    OutOfStock,
}

#[kagzi_step]
async fn check_inventory(input: OrderRequest) -> Result<Inventory, OrderError> {
    if input.quantity > 100 {
        return Err(OrderError::OutOfStock);
    }
    Ok(Inventory { available: true })
}

kagzi_workflow! {
    async fn workflow_with_errors(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        // The run! macro propagates errors with the ? operator
        let inventory = run!("inventory_check", check_inventory(input))?;

        // Errors are automatically enriched with step name
        // Error message will include: "Step 'inventory_check' failed: caused by Inventory unavailable"

        Ok(OrderResult { success: true })
    }
}
```

## Generated Code

### Before and After: `#[kagzi_step]`

**Input:**

```rust
#[kagzi_step]
async fn process_payment(input: PaymentRequest) -> anyhow::Result<PaymentResult> {
    if input.amount <= 0.0 {
        return Err(anyhow::anyhow!("Invalid amount"));
    }

    Ok(PaymentResult {
        transaction_id: "tx_123".to_string(),
        status: "success".to_string(),
    })
}
```

**Generated Output:**

```rust
async fn process_payment(input: PaymentRequest) -> anyhow::Result<PaymentResult> {
    use anyhow::Context;
    use tracing::Instrument;

    let span = tracing::info_span!(
        "step",
        step = "process_payment",
    );

    async {
        tracing::debug!(
            "Starting step: {}",
            stringify!(process_payment)
        );

        let result: anyhow::Result<PaymentResult> = {
            if input.amount <= 0.0 {
                return Err(anyhow::anyhow!("Invalid amount"));
            }

            Ok(PaymentResult {
                transaction_id: "tx_123".to_string(),
                status: "success".to_string(),
            })
        };

        match result {
            Ok(output) => {
                tracing::debug!(
                    output = ?output,
                    "Step completed successfully: {}",
                    stringify!(process_payment)
                );
                Ok(output)
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Step failed: {}",
                    stringify!(process_payment)
                );
                Err(e.context(format!("Step '{}' failed", stringify!(process_payment))))
            }
        }
    }.instrument(span).await
}
```

### Before and After: `kagzi_workflow!`

**Input:**

```rust
kagzi_workflow! {
    pub async fn process_order(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        let validated = run!("validate", validate_order(input));
        let inventory = run!("reserve", reserve_inventory(validated));

        Ok(OrderResult {
            order_id: "123".to_string(),
            success: true,
        })
    }
}
```

**Generated Output:**

```rust
pub async fn process_order(
    mut ctx: WorkflowContext,
    input: OrderRequest,
) -> anyhow::Result<OrderResult> {
    use tracing::Instrument;

    macro_rules! run {
        ($step_name:expr, $step_expr:expr) => {
            {
                ctx.run($step_name, $step_expr).await?
            }
        };
    }

    let span = tracing::info_span!(
        "workflow",
        workflow = "process_order",
    );

    async move {
        tracing::info!(
            "Starting workflow: {}",
            stringify!(process_order)
        );

        let validated = run!("validate", validate_order(input));
        let inventory = run!("reserve", reserve_inventory(validated));

        Ok(OrderResult {
            order_id: "123".to_string(),
            success: true,
        })
    }.instrument(span).await
}
```

## How It Works

### Compilation Flow

1. **Macro Expansion Phase** (during compilation):
   - `#[kagzi_step]` parses the function signature and validates it
   - Generates observability wrapper around your function body
   - Preserves original function signature (no breaking changes)

2. **Workflow Generation Phase**:
   - `kagzi_workflow!` parses the function definition
   - Generates `run!` macro definition inside the function
   - Wraps function body with workflow-level tracing

3. **Runtime Behavior**:
   - Spans are created when workflows/steps are executed
   - Tracing subscribers collect log events and span data
   - Errors are automatically enriched with step/workflow context

### Tracing Span Hierarchy

```
workflow:process_order
├── step:validate_order
│   ├── debug: Starting step: validate_order
│   └── debug: Step completed successfully: validate_order
├── step:reserve_inventory
│   ├── debug: Starting step: reserve_inventory
│   └── debug: Step completed successfully: reserve_inventory
└── step:process_payment
    ├── debug: Starting step: process_payment
    └── error: Step failed: process_payment
```

### Error Enrichment

When an error occurs in a step, the macro adds context:

```rust
// Original error
Err(anyhow::anyhow!("Invalid amount"))

// After macro enrichment
Err(anyhow::anyhow!("Invalid amount")
    .context("Step 'process_payment' failed"))
```

This creates a chain of context that helps with debugging:

```
Error: Step 'process_payment' failed

Caused by:
  0: Invalid amount

Location:
  process_payment at src/workflow.rs:25
```

## Observability Features

### Automatic Tracing

The macros integrate with `tracing` to provide:

1. **Structured Spans** - Each step and workflow creates a span with metadata
   - Step span: `{ target: "step", step: "<step_name>" }`
   - Workflow span: `{ target: "workflow", workflow: "<workflow_name>" }`

2. **Log Levels** - Appropriate log levels for different events
   - `INFO`: Workflow start
   - `DEBUG`: Step start/completion with input/output
   - `ERROR`: Step failures with error details

3. **Field Logging** - Structured fields for easy filtering and querying
   ```rust
   tracing::debug!(output = ?output, "Step completed successfully: {name}")
   // Logs: { output: Output { ... }, message: "Step completed successfully: my_step" }
   ```

### Correlation IDs

When using `WorkflowContext`, correlation IDs flow through the trace:

```rust
#[kagzi_step]
async fn my_step(ctx: WorkflowContext, input: Input) -> Result<Output> {
    // Access correlation_id from context
    let correlation_id = ctx.correlation_id();

    tracing::info!(correlation_id, "Processing");
    Ok(Output {})
}
```

The tracing span automatically inherits correlation IDs from parent spans.

### Metrics Integration

The `run!` macro calls `ctx.run(step_name, expr)`, which can be extended in the `WorkflowContext` implementation to emit metrics:

```rust
// In your WorkflowContext implementation
pub async fn run<T, E>(
    &mut self,
    step_name: &'static str,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let start = std::time::Instant::now();

    let result = future.await;

    let duration = start.elapsed();
    self.emit_step_metric(step_name, duration, result.is_ok());

    result
}
```

## Macro Attributes and Parameters

### `#[kagzi_step]` Attributes

Currently, the macro does not accept any custom attributes. All behavior is automatic.

**Planned Extensions:**

- `#[kagzi_step(name = "custom_name")]` - Override step name for tracing
- `#[kagzi_step(level = "info")]` - Set custom log level
- `#[kagzi_step(retries = 3)]` - Add retry logic

### `kagzi_workflow!` Parameters

The macro accepts a complete function definition including:

- Visibility (`pub`, `pub(crate)`, or private)
- Function name
- Parameters (must include `WorkflowContext` for `run!` to work)
- Return type (must be `Result<T, E>`)
- Function body

## Advanced Usage Patterns

### Fan-Out / Fan-In Pattern

Execute multiple steps in parallel and combine results:

```rust
kagzi_workflow! {
    async fn fan_out_fan_in(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<FinalResult> {
        // Fan-out: Execute independent steps
        let tasks = vec![
            run!("task1", task1(&input)),
            run!("task2", task2(&input)),
            run!("task3", task3(&input)),
        ];

        // Wait for all to complete
        let results: Vec<TaskResult> = futures::future::join_all(tasks).await
            .into_iter()
            .collect::<anyhow::Result<_>>()?;

        // Fan-in: Combine results
        let final_result = run!("combine", combine_results(results));

        Ok(final_result)
    }
}
```

### Saga Pattern (Compensating Transactions)

Implement a saga with compensating actions:

```rust
#[kagzi_step]
async fn reserve_seat(input: BookingRequest) -> anyhow::Result<Reservation> {
    // Reserve seat
    Ok(Reservation { seat_id: "A1".to_string() })
}

#[kagzi_step]
async fn process_payment(input: Reservation) -> anyhow::Result<Payment> {
    // Process payment
    Ok(Payment { status: "paid".to_string() })
}

#[kagzi_step]
async fn cancel_reservation(input: Reservation) -> anyhow::Result<()> {
    // Compensating action: release seat
    tracing::warn!(seat_id = input.seat_id, "Cancelling reservation");
    Ok(())
}

kagzi_workflow! {
    async fn booking_saga(
        mut ctx: WorkflowContext,
        input: BookingRequest,
    ) -> anyhow::Result<BookingConfirmation> {
        // Step 1: Reserve seat
        let reservation = match run!("reserve", reserve_seat(input.clone())) {
            Ok(r) => r,
            Err(e) => return Err(e), // Can't compensate first step
        };

        // Step 2: Process payment
        let payment = match run!("payment", process_payment(reservation.clone())) {
            Ok(p) => p,
            Err(e) => {
                // Compensate: cancel reservation
                let _ = run!("cancel_reservation", cancel_reservation(reservation));
                return Err(e);
            }
        };

        // Step 3: Send confirmation
        let confirmation = run!("confirm", send_confirmation(reservation, payment))?;

        Ok(confirmation)
    }
}
```

### Circuit Breaker Pattern

Add circuit breaker logic to step execution:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

static CIRCUIT_BREAKER: AtomicBool = AtomicBool::new(false);

#[kagzi_step]
async fn external_api_call(input: Request) -> anyhow::Result<Response> {
    if CIRCUIT_BREAKER.load(Ordering::Relaxed) {
        return Err(anyhow::anyhow!("Circuit breaker is open"));
    }

    // Make API call
    match call_api().await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            // Open circuit on consecutive failures
            CIRCUIT_BREAKER.store(true, Ordering::Relaxed);
            Err(e)
        }
    }
}
```

### Step Composition

Reuse steps across multiple workflows:

```rust
// Reusable steps
#[kagzi_step]
async fn validate_input<T>(input: T) -> anyhow::Result<T> {
    Ok(input)
}

#[kagzi_step]
async fn log_result<T>(input: T) -> anyhow::Result<T> {
    tracing::info!(result = ?input, "Step completed");
    Ok(input)
}

// Workflow 1
kagzi_workflow! {
    async fn workflow1(
        mut ctx: WorkflowContext,
        input: Input1,
    ) -> anyhow::Result<Output1> {
        let validated = run!("validate", validate_input(input));
        let logged = run!("log", log_result(validated));
        run!("process", process1(logged))
    }
}

// Workflow 2
kagzi_workflow! {
    async fn workflow2(
        mut ctx: WorkflowContext,
        input: Input2,
    ) -> anyhow::Result<Output2> {
        let validated = run!("validate", validate_input(input));
        let logged = run!("log", log_result(validated));
        run!("process", process2(logged))
    }
}
```

### Conditional Step Execution

Skip steps based on business logic:

```rust
kagzi_workflow! {
    async fn conditional_steps(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        let validated = run!("validate", validate_order(input.clone()))?;

        // Conditionally run verification
        let verified = if input.requires_verification {
            Some(run!("verify", verify_order(validified))?)
        } else {
            None
        };

        // Conditionally run premium features
        if input.is_premium {
            let upgraded = run!("upgrade", apply_premium_upgrade(validated))?;
            Ok(OrderResult { upgraded: true })
        } else {
            Ok(OrderResult { upgraded: false })
        }
    }
}
```

## Edge Cases and Limitations

### Type Inference Issues

In some cases, you may need to add type annotations:

```rust
// Might fail type inference
let result = run!("step", some_step(&input))?;

// Explicit type annotation
let result: OutputType = run!("step", some_step(&input))?;
```

### Complex Expressions in `run!`

For complex expressions, use block syntax:

```rust
let result = run!("complex_step", {
    let intermediate = preprocess(&input);
    complex_step(intermediate)
})?;
```

### Generic Steps

Steps can be generic:

```rust
#[kagzi_step]
async fn generic_step<T>(input: T) -> anyhow::Result<T> {
    Ok(input)
}

kagzi_workflow! {
    async fn generic_workflow(
        mut ctx: WorkflowContext,
        input: String,
    ) -> anyhow::Result<String> {
        // Type inference works here
        let validated = run!("validate", generic_step(input))?;
        Ok(validated)
    }
}
```

### Async Blocks

You can use async blocks with `run!`:

```rust
kagzi_workflow! {
    async fn with_async_block(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<Output> {
        let result = run!("custom", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            process(input)
        })?;

        Ok(result)
    }
}
```

### Macro Scope Limitations

The `run!` macro is only available within the same `kagzi_workflow!` block where it's defined:

```rust
// This works
kagzi_workflow! {
    async fn outer_workflow(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<Output> {
        let result = run!("inner", inner_function(&input))?;
        Ok(result)
    }
}

// This does NOT work - run! is not available here
async fn standalone_function(input: Input) -> anyhow::Result<Output> {
    // Error: cannot find macro `run!` in this scope
    let result = run!("step", some_step(input))?;
    Ok(result)
}
```

### Error Type Compatibility

Steps can return different error types, but they must be compatible:

```rust
#[kagzi_step]
async fn step1(input: Input) -> anyhow::Result<Output> {
    Ok(Output {})
}

#[kagzi_step]
async fn step2(input: Input) -> Result<Output, CustomError> {
    Ok(Output {})
}

kagzi_workflow! {
    async fn mixed_errors(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<FinalOutput> {
        // Works - anyhow::Result is compatible with Result<T, CustomError>
        let r1 = run!("step1", step1(input.clone()))?;
        let r2 = run!("step2", step2(input.clone()))?;
        Ok(FinalOutput {})
    }
}
```

## Testing Strategies

### Unit Testing Steps

Steps can be tested independently since they're just async functions:

```rust
#[tokio::test]
async fn test_validate_order() {
    let input = OrderRequest {
        items: vec![],
    };

    let result = validate_order(input).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));
}
```

### Integration Testing Workflows

Test the entire workflow with mock dependencies:

```rust
#[tokio::test]
async fn test_order_workflow() {
    let mut ctx = WorkflowContext::test_new();
    let input = OrderRequest::test_data();

    let result = process_order(ctx, input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.success, true);
}
```

### Spying on Logs

Use `tracing_subscriber` to capture logs in tests:

```rust
#[tokio::test]
async fn test_step_logs() {
    let (subscriber, handle) = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .finish();

    tracing::subscriber::with_default(subscriber, || async {
        validate_order(test_input()).await.unwrap();
    }).await;

    let logs = handle.logs();
    assert!(logs.iter().any(|log| log.contains("validate_order")));
}
```

## Performance Considerations

### Zero-Cost Abstractions

The macros generate code with minimal runtime overhead:

- Span creation is lightweight (single allocation per span)
- No dynamic dispatch
- No reflection

### Async Overhead

Each step's async block adds minimal overhead (~1 pointer for the future).
The `run!` macro uses `.await?` which is the standard pattern in async Rust.

### Memory Usage

- Spans are allocated on stack when possible
- Log fields use `tracing::field::Display` or `Debug` implementations
- No heap allocations unless explicitly done in your code

## Best Practices

### 1. Keep Steps Focused

Each step should do one thing well:

```rust
// Good: Single responsibility
#[kagzi_step]
async fn validate_email(input: UserInput) -> anyhow::Result<ValidatedEmail> {
    validate_email_format(&input.email)?;
    Ok(ValidatedEmail { email: input.email })
}

// Avoid: Multiple responsibilities
#[kagzi_step]
async fn validate_and_save_user(input: UserInput) -> anyhow::Result<User> {
    validate_email(&input.email)?;
    save_to_db(input)?; // Should be separate step
    Ok(User {})
}
```

### 2. Use Descriptive Step Names

The step name is used in logs and metrics:

```rust
// Good: Descriptive
run!("validate_payment_method", validate_payment(&input))

// Avoid: Vague
run!("step1", validate_payment(&input))
```

### 3. Leverage Type System

Use strongly typed inputs and outputs:

```rust
#[derive(Debug)]
pub struct ValidatedOrder {
    pub order_id: String,
    pub total: f64,
}

#[kagzi_step]
async fn validate_order(input: OrderRequest) -> anyhow::Result<ValidatedOrder> {
    // ...
}

// Next step receives typed input
#[kagzi_step]
async fn reserve_inventory(input: ValidatedOrder) -> anyhow::Result<Inventory> {
    // ...
}
```

### 4. Handle Errors Appropriately

Return specific error types and let the macro enrich them:

```rust
#[derive(Error, Debug)]
pub enum OrderError {
    #[error("Invalid order: {0}")]
    Validation(String),
    #[error("Payment failed: {0}")]
    Payment(String),
}

#[kagzi_step]
async fn process_payment(input: PaymentRequest) -> Result<PaymentResult, OrderError> {
    // Macro will add: "Step 'process_payment' failed: caused by ..."
}
```

### 5. Document Step Contracts

Add doc comments explaining input/output contracts:

```rust
#[kagzi_step]
/// Validates an order request.
///
/// # Input
/// - `order_id`: Must be non-empty
/// - `items`: Must have at least one item
///
/// # Output
/// Returns `ValidatedOrder` with calculated totals.
///
/// # Errors
/// Returns `OrderError` if validation fails.
async fn validate_order(input: OrderRequest) -> anyhow::Result<ValidatedOrder> {
    // ...
}
```

## Troubleshooting

### "kagzi_step functions must be async"

The function is missing the `async` keyword:

```rust
// Error
#[kagzi_step]
fn my_step(input: Input) -> Result<Output, Error> {
    // ...
}

// Fix
#[kagzi_step]
async fn my_step(input: Input) -> Result<Output, Error> {
    // ...
}
```

### "kagzi_step functions must return a Result<T, E>"

The function doesn't return a `Result` type:

```rust
// Error
#[kagzi_step]
async fn my_step(input: Input) -> Output {
    // ...
}

// Fix
#[kagzi_step]
async fn my_step(input: Input) -> Result<Output, Error> {
    // ...
}
```

### "cannot find macro `run!` in this scope"

The `run!` macro is used outside a `kagzi_workflow!` block:

```rust
// Error
async fn my_function() -> anyhow::Result<Output> {
    let result = run!("step", some_step())?;
    Ok(result)
}

// Fix
kagzi_workflow! {
    async fn my_workflow(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<Output> {
        let result = run!("step", some_step())?;
        Ok(result)
    }
}
```

### Type Inference Failures

Complex expressions may need explicit types:

```rust
// Error: Type inference failed
let result = run!("step", complex_transform(input))?;

// Fix: Add type annotation
let result: OutputType = run!("step", complex_transform(input))?;
```

## Migration Guide

### From Manual Tracing to `#[kagzi_step]`

**Before:**

```rust
async fn validate_order(input: OrderRequest) -> anyhow::Result<ValidationResult> {
    let span = tracing::info_span!("validate_order");
    let _enter = span.enter();

    tracing::debug!("Starting validation");

    let result = validate_internal(input)?;

    tracing::debug!(result = ?result, "Validation complete");
    Ok(result)
}
```

**After:**

```rust
#[kagzi_step]
async fn validate_order(input: OrderRequest) -> anyhow::Result<ValidationResult> {
    validate_internal(input)
}
```

### From Manual Workflow to `kagzi_workflow!`

**Before:**

```rust
pub async fn process_order(
    mut ctx: WorkflowContext,
    input: OrderRequest,
) -> anyhow::Result<OrderResult> {
    let span = tracing::info_span!("process_order");
    let _enter = span.enter();

    tracing::info!("Starting workflow");

    let validated = ctx.run("validate", validate_order(input)).await?;
    let inventory = ctx.run("reserve", reserve_inventory(validated)).await?;

    Ok(OrderResult { success: true })
}
```

**After:**

```rust
kagzi_workflow! {
    pub async fn process_order(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderResult> {
        let validated = run!("validate", validate_order(input));
        let inventory = run!("reserve", reserve_inventory(validated));

        Ok(OrderResult { success: true })
    }
}
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
kagzi-macros = { workspace = true }
kagzi = { workspace = true }
```

## Usage

1. Import the macros:

```rust
use kagzi_macros::{kagzi_step, kagzi_workflow};
use kagzi::prelude::*;
```

2. Apply `#[kagzi_step]` to step functions
3. Use `kagzi_workflow!` to define workflow blocks

## Examples

See the examples directory for complete working examples:

- `examples/11_macros/main.rs` - Basic macro usage
- `examples/12_kagzi_step_macro/main.rs` - Step macro examples
- `examples/13_kagzi_workflow_macro/main.rs` - Workflow macro examples
- `examples/14_macros_comprehensive/main.rs` - Advanced patterns

## Contributing

When contributing to the `kagzi-macros` crate:

1. Add tests for new features in `src/tests.rs`
2. Update examples in the main README
3. Ensure macro expansion is tested with `cargo expand`
4. Document any new attributes or parameters

## Development Notes

The macros are implemented using the procedural macro system with:

- `proc_macro2` for better token stream handling
- `syn` for parsing Rust code
- `quote` for code generation
- No runtime overhead - all expansion happens at compile time

## Benefits

- **Reduced Boilerplate**: Automatic tracing and logging
- **Better Observability**: Structured logging with context
- **Cleaner Syntax**: Concise workflow definitions
- **Type Safety**: Compile-time checking of function signatures
- **IDE Support**: Excellent rust-analyzer compatibility

## License

This crate is part of the Kagzi project and follows the same license.

## See Also

- [Main Kagzi Documentation](../../README.md)
- [WorkflowContext API](../kagzi/src/lib.rs)
- [Integration Tests](../../tests/)
- [Examples](../../examples/)
