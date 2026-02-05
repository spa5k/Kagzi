# Kagzi Rust SDK

The Kagzi Rust SDK provides a Rust interface for defining workflows, running workers, and interacting with the Kagzi server via gRPC.

## Overview

Kagzi is a durable workflow orchestration system that guarantees at-least-once execution of workflow steps. The SDK enables you to:

- Define workflows as async functions with typed inputs and outputs
- Start workflows programmatically or on schedules (cron expressions)
- Run workers that claim and execute workflows
- Track workers/queues via Telemetry and a queue registry

## Task queues (important)

Kagzi uses a logical **task queue** identifier (scoped to a namespace). Internally, queue backends map `{namespace, task_queue}` to native primitives (Kafka, NATS JetStream, Postgres LISTEN/NOTIFY, …).

- Default routing: if `task_queue` is omitted, Kagzi uses the per-namespace `"default"` queue.
- The UI can show queues even before they’re used by allowing explicit queue creation (metadata-only).
- The current SDK worker polls the `"default"` queue in its namespace (custom routing can be added later without changing the core model).

## Features

- **Type-safe workflows**: Define workflows with typed inputs/outputs using async functions
- **Durable execution**: Step checkpointing and deterministic replay behavior
- **Retries**: Step failures can be retried by the server based on configured policies
- **Scheduled workflows**: Cron-based workflow execution with catch-up support
- **Telemetry**: Worker snapshots + events for observability (UI-friendly)
- **Queue registry**: Optional queue metadata (display name, labels, extra JSON)

## Architecture (high level)

```
┌─────────────────┐         gRPC          ┌──────────────────┐
│   Client App    │◄─────────────────────►│   Kagzi Server   │
│                 │                        │                  │
│  - Start WF     │                        │  - DB state     │
│  - Schedules    │                        │  - Telemetry    │
│  - Queue meta   │                        │  - Queue bus    │
└─────────────────┘                        └──────────────────┘
                                                   │
                                                   │ wakeups + claim
                                                   │
┌─────────────────┐         gRPC          ┌──────────────────┐
│   Worker App    │◄─────────────────────►│  WorkerService   │
│                 │                        │  - ClaimTask    │
│  - Execute steps│                        │  - Step report  │
│  - Telemetry    │                        │  - Heartbeat    │
└─────────────────┘                        └──────────────────┘
```

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
kagzi = { path = "../sdk/kagzi-rs" } # or a published version
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["macros", "rt-multi-thread"] }
anyhow = "1.0"
```

## Quick start

### 1) Define a workflow

```rust
use kagzi::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct HelloInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelloOutput {
    message: String,
}

async fn hello_workflow(_ctx: Context, input: HelloInput) -> anyhow::Result<HelloOutput> {
    Ok(HelloOutput {
        message: format!("Hello, {}!", input.name),
    })
}
```

### 2) Run a worker

```rust
use kagzi::Worker;

let mut worker = Worker::new("http://localhost:50051")
    .namespace("default")
    .workflows([("hello_workflow", hello_workflow)])
    .build()
    .await?;

worker.run().await?;
```

### 3) Start a workflow

```rust
use kagzi::Kagzi;

let client = Kagzi::connect("http://localhost:50051").await?;
let run = client
    .start("hello_workflow")
    .namespace("default")
    .input(&HelloInput { name: "Kagzi".into() })?
    .send()
    .await?;

println!("run_id={}", run.id);
```

### 4) Create a schedule

```rust
use kagzi::Kagzi;

let client = Kagzi::connect("http://localhost:50051").await?;
let schedule = client
    .schedule("daily-report")
    .namespace("default")
    .workflow("generate_report")
    .cron("0 9 * * *")
    .send()
    .await?;

println!("schedule_id={}", schedule.schedule_id);
```

## Queue registry (QueueService)

Queues are **metadata-only**: they exist for UX/governance (UI, labels, descriptions). Execution can still implicitly create queues.

Create a queue:

```rust
use kagzi::Kagzi;
use serde_json::json;

let client = Kagzi::connect("http://localhost:50051").await?;
let q = client
    .queue("high_priority")
    .namespace("default")
    .display_name("High Priority")
    .description("Latency-sensitive workflows")
    .label("lane", "realtime")
    .extra_json(json!({ "owner": "payments" }))
    .send()
    .await?;

println!("created: {}/{}", q.namespace, q.task_queue);
```

List queues:

```rust
let queues = client.list_queues("default", None).await?;
for q in queues {
    println!("queue: {} (enabled={})", q.task_queue, q.enabled);
}
```

Update queue metadata:

> Note: `update_queue` currently accepts the raw protobuf request type (`kagzi_proto::kagzi::UpdateQueueRequest`).

```rust
use kagzi_proto::kagzi::UpdateQueueRequest;

let updated = client
    .update_queue(UpdateQueueRequest {
        namespace: "default".to_string(),
        task_queue: "high_priority".to_string(),
        display_name: Some("High Priority (v2)".to_string()),
        description: None,
        enabled: None,
        labels: Default::default(),
        extra_json: None,
    })
    .await?;

println!("updated enabled={}", updated.enabled);
```

## Common patterns

### Fan-out / fan-in

```rust
use futures::future::try_join_all;

async fn process_batch(mut ctx: Context, items: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut tasks = Vec::with_capacity(items.len());
    for (idx, item) in items.into_iter().enumerate() {
        let step_name = format!("process-item-{idx}");
        tasks.push(ctx.step(step_name).run(|| async move { Ok::<_, anyhow::Error>(item) }));
    }
    try_join_all(tasks).await
}
```

### Saga / compensation

```rust
use kagzi::KagziError;

async fn transaction(mut ctx: Context) -> anyhow::Result<()> {
    let reservation_id: String = ctx
        .step("reserve-inventory")
        .run(|| async { Ok::<_, anyhow::Error>("resv-123".to_string()) })
        .await?;

    let payment = ctx
        .step("charge-card")
        .run(|| async { anyhow::bail!(KagziError::retry_after("gateway timeout", std::time::Duration::from_secs(30))) })
        .await;

    match payment {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = ctx
                .step("release-inventory")
                .run(|| async move {
                    // release(reservation_id).await?;
                    Ok::<_, anyhow::Error>(reservation_id)
                })
                .await;
            Err(e)
        }
    }
}
```

### Long-running workflows

```rust
async fn long_running(mut ctx: Context) -> anyhow::Result<()> {
    loop {
        ctx.sleep("poll", "1h").await?;
        let done: bool = ctx
            .step("check-status")
            .run(|| async { Ok::<_, anyhow::Error>(false) })
            .await?;
        if done {
            break;
        }
    }
    Ok(())
}
```

## Troubleshooting

### Workflow not picked up

Check:

1. Worker is running and registered in the same namespace
2. The workflow type matches what the worker registered
3. Your queue backend is healthy (Kafka/NATS/Postgres listener)

### Steps repeated unexpectedly

This is normal at-least-once behavior. Steps must be idempotent:

- Use upserts instead of inserts
- Check existence before creating external resources
- Use idempotency keys for external APIs

## Retries

### Worker default retry policy

```rust
use kagzi::{Retry, Worker};

let mut worker = Worker::new("http://localhost:50051")
    .namespace("default")
    .retry(Retry::exponential(5))
    .workflows([("hello_workflow", hello_workflow)])
    .build()
    .await?;
```

### Per-step override

```rust
use kagzi::Retry;

let value: String = ctx
    .step("call-external")
    .retry(Retry::linear(3, std::time::Duration::from_secs(2)))
    .run(|| async { Ok::<_, anyhow::Error>("ok".to_string()) })
    .await?;
```

## Error handling

Kagzi uses `KagziError` to encode retry semantics and metadata.

```rust
use kagzi::KagziError;
use kagzi_proto::kagzi::ErrorCode;

// Non-retryable validation failure:
return Err(KagziError::new(ErrorCode::InvalidArgument, "bad input").into());

// Retryable failure with delay:
return Err(KagziError::retry_after(
    "temporary outage",
    std::time::Duration::from_secs(15),
).into());
```

## Performance considerations

- Prefer steps that take ~1–10 seconds for good observability and bounded retries.
- Keep payloads under ~1MB; store large blobs externally and pass references.
- Scale horizontally by running more workers in the same namespace.

## Notes

- The SDK worker currently polls the per-namespace `"default"` queue.
- Worker telemetry is enabled by default; disable it with `WorkerBuilder::telemetry_enabled(false)`.

## License

Licensed under the same terms as the Kagzi project. See the repository `LICENSE`.
