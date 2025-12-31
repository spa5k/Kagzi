## workflow_service.rs

In the worker.

```rust
use kagzi::{Worker, WorkerBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Build and register worker
    let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
        .namespace("production")
        .max_concurrent(50)
        .queue_concurrency_limit(100)
        .default_step_retry(RetryPolicy {
            maximum_attempts: Some(3),
            initial_interval: Some(std::time::Duration::from_secs(1)),
            backoff_coefficient: Some(2.0),
            maximum_interval: Some(std::time::Duration::from_secs(60)),
            non_retryable_errors: vec![
                "InvalidArgument".to_string(),
                "PreconditionFailed".to_string(),
            ],
        })
        .build()
        .await?;

    // Register workflows
    worker.register("process-order", process_order);

    // Run worker (blocks until shutdown)
    worker.run().await?;

    Ok(())
}
```

`let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")`

the name should be define below .name or something.

Similarly in this -

```rust
.default_step_retry(RetryPolicy {
    maximum_attempts: Some(3),
    initial_interval: Some(std::time::Duration::from_secs(1)),
    backoff_coefficient: Some(2.0),
    maximum_interval: Some(std::time::Duration::from_secs(60)),
    non_retryable_errors: vec![
        "InvalidArgument".to_string(),
        "PreconditionFailed".to_string(),
    ],
})
```

this should be simplified,

```rust
let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
    .namespace("production")
    .max_concurrent(50)
    .queue_concurrency_limit(100)
```

two concurrency thing is complex, the worker semaphore internally, should be just one I guess?
why are there two, need to check.

```rust
use kagzi::Client;
use chrono::{Utc, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = Client::connect("http://localhost:50051").await?;

    // Create a scheduled workflow
    let schedule = client
        .workflow_schedule(
            "daily-report",
            "reports",
            "0 6 * * *", // Cron: 6 AM daily
            ReportInput { report_type: "daily".to_string() },
        )
        .namespace("production")
        .enabled(true)
        .max_catchup(7) // Catch up to 7 missed executions
        .await?;

    println!("Schedule created: {}", schedule.schedule_id);
    Ok(())
}

#[derive(Serialize)]
struct ReportInput {
    report_type: String,
}
```

## sleep

```rust
use kagzi::WorkflowContext;
use std::time::Duration;

async fn delayed_workflow(
    mut ctx: WorkflowContext,
    input: String,
) -> anyhow::Result<String> {
    // Execute first step
    let result = ctx.run("step1", async move {
        Ok(format!("Processed: {}", input))
    }).await?;

    // Sleep for 30 seconds (workflow pauses, worker can process other tasks)
    ctx.sleep(Duration::from_secs(30)).await?;

    // Execute after sleep
    let final_result = ctx.run("step2", async move {
        Ok(format!("Final: {}", result))
    }).await?;

    Ok(final_result)
}
```

similarly need to clarify the sleep thing, see how its done in vercel workflow/temporal etc, this feels weird to me maybe.

Lets delete the macros for now.

```rust
// Step 1: Validate order
    let validation_result = ctx
        .run_with_input(
            "validate-order",
            &input,
            async move {
                // Validation logic
                if input.amount <= 0.0 {
                    return Err(KagziError::non_retryable("Invalid amount"));
                }
                Ok(true)
            },
        )
        .await?;

    // Step 2: Process payment
    let payment_result = ctx
        .run("process-payment", async move {
            // Payment processing logic
            Ok("payment-123".to_string())
        })
        .await?;
```

why is there two run way, run_with_input and normal run?

```rust
    // Build and register worker
    let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
        .namespace("production")
        .max_concurrent(50)
        .queue_concurrency_limit(100)
        .default_step_retry(RetryPolicy {
            maximum_attempts: Some(3),
            initial_interval: Some(std::time::Duration::from_secs(1)),
            backoff_coefficient: Some(2.0),
            maximum_interval: Some(std::time::Duration::from_secs(60)),
            non_retryable_errors: vec![
                "InvalidArgument".to_string(),
                "PreconditionFailed".to_string(),
            ],
        })
        .build()
        .await?;

    // Register workflows
    worker.register("process-order", process_order);
```

the workflow registrration should be like this imo

```rust
let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
        .namespace("production")
        .max_concurrent(50)
        .workflows([process_order, cancel_order]),
```

something like this

### retry -

````rust
```rust
pub fn workflow<I: Serialize>(
    &mut self,
    workflow_type: &str,
    task_queue: &str,
    input: I,
) -> WorkflowBuilder<'_, I>
````

Creates a builder for starting a workflow.

**WorkflowBuilder Methods:**

- `id(external_id: impl Into<String>)` - Set business identifier
- `namespace(ns: impl Into<String>)` - Set namespace (default: "default")
- `version(version: impl Into<String>)` - Set workflow version
- `retry_policy(policy: RetryPolicy)` - Set retry policy
- `retries(max_attempts: i32)` - Shortcut to set max retry attempts

we should clarify on the retry related part, and maybe simplify it, see how its done elsewhere.

similarly here -

```rust
pub fn builder(addr: &str, task_queue: &str) -> WorkerBuilder
```

Creates a builder for configuring a worker.

**WorkerBuilder Methods:**

- `namespace(ns: &str)` - Set namespace (default: "default")
- `max_concurrent(n: usize)` - Max concurrent workflows (default: 100)
- `hostname(h: &str)` - Set hostname (default: auto-detected)
- `version(v: &str)` - Set worker version
- `label(key: &str, value: &str)` - Add worker label
- `queue_concurrency_limit(limit: i32)` - Set queue-wide concurrency limit
- `workflow_type_concurrency(workflow_type: &str, limit: i32)` - Per-workflow-type limit
- `default_step_retry(policy: RetryPolicy)` - Default retry policy for steps

too many APIs, related to concurrency.

Preferably, all the concurrency here at least on the server side should be handled by the queue, and the worker should have minimal cocurrency setup.

## Retry Policies

### Worker-Level Defaults

Set default retry policy for all steps:

```rust
let mut worker = WorkerBuilder::new("http://localhost:50051", "orders")
    .default_step_retry(RetryPolicy {        maximum_attempts: Some(3),        initial_interval: Some(Duration::from_secs(1)),        backoff_coefficient: Some(2.0),        maximum_interval: Some(Duration::from_secs(60)),        non_retryable_errors: vec![            "InvalidArgument".to_string(),            "PreconditionFailed".to_string(),        ],    })    .build()    .await?;
```

### Step-Level Overrides

Override retry policy for specific steps:

```rust
let result = ctx
    .run_with_input_with_retry(        "critical-operation",        &input,        Some(RetryPolicy {            maximum_attempts: Some(10),            initial_interval: Some(Duration::from_millis(500)),            backoff_coefficient: Some(1.5),            maximum_interval: Some(Duration::from_secs(30)),            non_retryable_errors: vec![],        }),        async { critical_operation(input).await },    )    .await?;
```

### Workflow-Level Retry

Set retry policy when starting a workflow:

```rust
let run_id = client
    .workflow("my-workflow", "queue", input)    .retry_policy(RetryPolicy {        maximum_attempts: Some(5),        initial_interval: Some(Duration::from_secs(2)),        backoff_coefficient: Some(2.0),        maximum_interval: Some(Duration::from_secs(300)),        non_retryable_errors: vec![],    })    .await?;
```

Heres how sleep in temporal looks liek -

```go
timer := workflow.NewTimer(timerCtx, duration)
sleep = workflow.Sleep(ctx, 10*time.Second)
```

and in inngest -

```ts
import { Inngest } from "inngest";

const inngest = new Inngest({ id: "signup-flow" });

export const fn = inngest.createFunction(
  { id: "send-signup-email" },
  { event: "app/user.created" },
  async ({ event, step }) => {
    await step.sleep("wait-a-moment", "1 hour");
    await step.run("do-some-work-in-the-future", async () => {
      // This runs after 1 hour
    });
  },
);
```

```ts
import { Inngest } from "inngest";

const inngest = new Inngest({ id: "signup-flow" });

export const fn = inngest.createFunction(
  { id: "send-signup-email" },
  { event: "app/user.created" },
  async ({ event, step }) => {
    await step.sleepUntil("wait-for-iso-string", "2023-04-01T12:30:00");

    // You can also sleep until a timestamp within the event data.  This lets you
    // pass in a time for you to run the job:
    await step.sleepUntil("wait-for-timestamp", event.data.run_at); // Assuming event.data.run_at is a timestamp.

    await step.run("do-some-work-in-the-future", async () => {
      // This runs at the specified time.
    });
  },
);
```

There should only be one way of executing, not two -

#### Execute Steps

```rust
pub async fn run<R, Fut>(&mut self, step_id: &str, fut: Fut) -> anyhow::Result<R>
where
    R: Serialize + DeserializeOwned + Send + 'static,
    Fut: Future<Output = anyhow::Result<R>> + Send,
```

Executes a step without input. Automatically retries on failure if step execution wasn't completed before.

**Example:**

```rust
let result = ctx.run("fetch-user", async {
    fetch_user_from_db(user_id).await
}).await?;
```

```rust
pub async fn run_with_input<I, R, Fut>(
    &mut self,
    step_id: &str,
    input: &I,
    fut: Fut,
) -> anyhow::Result<R>
where
    I: Serialize + Send + 'static,
    R: Serialize + DeserializeOwned + Send + 'static,
    Fut: Future<Output = anyhow::Result<R>> + Send,
```

Executes a step with typed input. Input is serialized and cached for replay.

Too much contrived -

### RetryPolicy

Controls retry behavior for failed steps.

```rust
#[derive(Clone, Default)]
pub struct RetryPolicy {
    pub maximum_attempts: Option<i32>,
    pub initial_interval: Option<Duration>,
    pub backoff_coefficient: Option<f64>,
    pub maximum_interval: Option<Duration>,
    pub non_retryable_errors: Vec<String>,
}
```

**Fields:**

- `maximum_attempts`: Maximum number of retry attempts (0 = unlimited)
- `initial_interval`: Initial delay before first retry
- `backoff_coefficient`: Multiplier for exponential backoff (e.g., 2.0 doubles each time)
- `maximum_interval`: Maximum delay between retries
- `non_retryable_errors`: Error codes that should not be retried
